//! Block-level operations for reftable files.
//!
//! A reftable block contains a block header, sorted records with prefix compression,
//! and a restart table at the end for binary search.
//!
//! ## Block layout
//!
//! ```text
//! +------------------+
//! | block_type (1B)  |  Block header (4 bytes total)
//! | block_len (3B)   |  BE24, total block length including header
//! +------------------+
//! | record data ...  |  Prefix-compressed key/value records
//! +------------------+
//! | restart[0] (3B)  |  BE24, absolute offset from block start
//! | restart[1] (3B)  |
//! | ...              |
//! +------------------+
//! | restart_count(2B)|  BE16, number of restart entries
//! +------------------+
//! | padding (0s) ... |  Zero-padding to fill block_size (ref/obj/idx blocks)
//! +------------------+
//! ```
//!
//! Log blocks have zlib-compressed record data (everything after the 4-byte header).

use crate::{get_be24, put_be24, BlockType, Error, RefRecord};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read as _, Write as _};

/// A block header as read from the file.
#[derive(Debug, Clone)]
pub struct BlockHeader {
    /// The type of records in this block.
    pub block_type: BlockType,
    /// Total block length including the 4-byte header.
    pub block_len: u32,
}

/// Parse a block header from the start of a block.
///
/// The block header is 4 bytes:
/// - Byte 0: block type character ('r', 'g', 'o', 'i')
/// - Bytes 1-3: BE24 block_len (total block length including this header)
///
/// Returns the header and the number of header bytes consumed (always 4).
pub fn parse_block_header(data: &[u8]) -> Result<(BlockHeader, usize), Error> {
    if data.len() < 4 {
        return Err(Error::UnexpectedEof);
    }

    let block_type = BlockType::from_byte(data[0]).ok_or(Error::InvalidBlockType { byte: data[0] })?;

    let block_len = get_be24(data[1..4].try_into().expect("3 bytes for BE24"));

    Ok((BlockHeader { block_type, block_len }, 4))
}

/// Read ref records from a ref block.
///
/// This reads all records from a ref block, handling prefix compression.
/// The `block_data` should be the raw block bytes including header.
///
/// `offset_in_file` is the byte offset of this block within the reftable file.
/// C Git stores `block_len` as a file-level end offset (not a block size), so we
/// need this to correctly compute the data extent.
///
/// If `offset_in_file` is 0, `block_len` is interpreted as a block-relative size
/// (for backward compatibility with gitoxide-produced blocks).
pub fn read_ref_records(block_data: &[u8], hash_size: usize, min_update_index: u64) -> Result<Vec<RefRecord>, Error> {
    read_ref_records_at(block_data, hash_size, min_update_index, 0)
}

/// Like [`read_ref_records`], but accounting for C Git's `block_len` encoding.
///
/// In C Git, the first block in a file uses `header_off = file_header_size` (24 for V1)
/// when writing, which means `block_len = header_off + actual_content_size`. Subsequent
/// blocks use `header_off = 0`, so `block_len = actual_content_size`.
///
/// Pass `header_off` for the block being read:
/// - For the **first** block in a file: pass the file header size (e.g., 24 for V1).
/// - For **subsequent** blocks: pass 0.
/// - For gitoxide-written blocks (legacy format): pass 0.
pub fn read_ref_records_at(
    block_data: &[u8],
    hash_size: usize,
    min_update_index: u64,
    header_off: usize,
) -> Result<Vec<RefRecord>, Error> {
    if block_data.len() < 4 {
        return Err(Error::UnexpectedEof);
    }

    // Parse block header
    let (header, header_size) = parse_block_header(block_data)?;
    if header.block_type != BlockType::Ref {
        return Err(Error::InvalidBlockType {
            byte: header.block_type as u8,
        });
    }

    // C Git writes `block_len = header_off + content_size` where:
    // - For the first block: header_off = file_header_size (24 for V1)
    // - For subsequent blocks: header_off = 0
    //
    // Subtract `header_off` to get the actual content size within `block_data`.
    let data_end = if header.block_len > 0 {
        let content_size = (header.block_len as usize).saturating_sub(header_off);
        content_size.min(block_data.len())
    } else {
        block_data.len()
    };

    if data_end < header_size + 2 {
        return Ok(Vec::new());
    }

    // Read restart count from the last 2 bytes of the data portion
    let restart_count = u16::from_be_bytes([block_data[data_end - 2], block_data[data_end - 1]]);

    // The restart offsets occupy restart_count * 3 bytes before the restart_count field
    let restart_table_size = (restart_count as usize) * 3 + 2;
    let records_end = data_end.saturating_sub(restart_table_size);

    let mut records = Vec::new();
    let mut pos = header_size;
    let mut last_name: Vec<u8> = Vec::new();

    while pos < records_end {
        match crate::parse_ref_record(&block_data[pos..records_end], &last_name, hash_size, min_update_index) {
            Ok((record, consumed)) => {
                last_name = record.name().to_vec();
                records.push(record);
                pos += consumed;
            }
            Err(_) => break,
        }
    }

    Ok(records)
}

/// Decompress a log block's data in place.
///
/// Log blocks store their record data (everything after the 4-byte block header)
/// zlib-compressed. The `block_len` in the header gives the *uncompressed* total size.
///
/// `raw_block` is the raw bytes as read from the file (header + compressed data).
/// Returns a new buffer with the header preserved and data decompressed.
pub fn decompress_log_block(raw_block: &[u8]) -> Result<Vec<u8>, Error> {
    if raw_block.len() < 4 {
        return Err(Error::UnexpectedEof);
    }

    let (header, _) = parse_block_header(raw_block)?;
    if header.block_type != BlockType::Log {
        return Err(Error::InvalidBlockType {
            byte: header.block_type as u8,
        });
    }

    let block_header_skip = 4;
    let uncompressed_size = header.block_len as usize;

    // Security: Cap decompression to prevent OOM from crafted reftable files
    // (zlib decompression bomb). A single block should never exceed 64 MB.
    const MAX_DECOMPRESSED_SIZE: usize = 64 * 1024 * 1024;
    if uncompressed_size > MAX_DECOMPRESSED_SIZE {
        return Err(Error::ZlibError);
    }

    let mut result = Vec::with_capacity(uncompressed_size);
    // Copy the 4-byte block header verbatim
    result.extend_from_slice(&raw_block[..block_header_skip]);

    // Decompress the remaining data with a size limit to prevent decompression bombs.
    let compressed_data = &raw_block[block_header_skip..];
    let mut decoder = ZlibDecoder::new(compressed_data);
    let max_decompress = uncompressed_size.saturating_sub(block_header_skip);
    let mut decompressed = Vec::new();
    std::io::Read::take(&mut decoder, (max_decompress + 1) as u64)
        .read_to_end(&mut decompressed)
        .map_err(|_| Error::ZlibError)?;

    result.extend_from_slice(&decompressed);

    if result.len() != uncompressed_size {
        return Err(Error::ZlibError);
    }

    Ok(result)
}

/// Compress a log block's data.
///
/// Takes an uncompressed log block (header + records + restart table + restart count)
/// and compresses everything after the 4-byte header using zlib.
///
/// The `block_len` in the header is set to the uncompressed total size before compression.
/// Returns the compressed block (4-byte header + compressed data).
pub fn compress_log_block(uncompressed_block: &mut Vec<u8>) -> Result<Vec<u8>, Error> {
    if uncompressed_block.len() < 4 {
        return Err(Error::UnexpectedEof);
    }

    let block_header_skip = 4;

    // Set block_len to the uncompressed size
    let uncompressed_size = uncompressed_block.len() as u32;
    put_be24(
        (&mut uncompressed_block[1..4]).try_into().expect("3 bytes for BE24"),
        uncompressed_size,
    );

    // Compress the data after the header
    let data_to_compress = &uncompressed_block[block_header_skip..];
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data_to_compress).map_err(|_| Error::ZlibError)?;
    let compressed = encoder.finish().map_err(|_| Error::ZlibError)?;

    // Build compressed block: header + compressed data
    let mut result = Vec::with_capacity(block_header_skip + compressed.len());
    result.extend_from_slice(&uncompressed_block[..block_header_skip]);
    result.extend_from_slice(&compressed);

    Ok(result)
}

/// A reftable stack: manages multiple reftable files as layers.
///
/// The stack is tracked by a `tables.list` file in the reftable directory.
/// Lookups search from newest to oldest table.
#[derive(Debug, Clone)]
pub struct Stack {
    /// Path to the reftable directory.
    pub path: std::path::PathBuf,
    /// Ordered list of table filenames (oldest first).
    pub tables: Vec<String>,
}

impl Stack {
    /// Read a reftable stack from the given directory.
    ///
    /// Reads and parses the `tables.list` manifest file.
    pub fn open(reftable_dir: impl Into<std::path::PathBuf>) -> Result<Self, Error> {
        let path = reftable_dir.into();
        let list_path = path.join("tables.list");
        let content = std::fs::read_to_string(&list_path)?;
        let tables: Vec<String> = content
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_owned())
            .collect();
        Ok(Stack { path, tables })
    }

    /// Get the full path to a specific table file.
    pub fn table_path(&self, table_name: &str) -> std::path::PathBuf {
        self.path.join(table_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_block_header_ref_type() {
        // block_type='r', block_len=100 as BE24
        let mut data = [0u8; 4];
        data[0] = b'r';
        crate::put_be24((&mut data[1..4]).try_into().expect("3 bytes"), 100);
        let (header, consumed) = parse_block_header(&data).expect("should parse");
        assert_eq!(header.block_type, BlockType::Ref);
        assert_eq!(header.block_len, 100);
        assert_eq!(consumed, 4);
    }

    #[test]
    fn parse_block_header_with_block_len() {
        let mut data = [0u8; 4];
        data[0] = b'r';
        crate::put_be24((&mut data[1..4]).try_into().expect("3 bytes"), 4096);
        let (header, _) = parse_block_header(&data).expect("should parse");
        assert_eq!(header.block_len, 4096);
    }

    #[test]
    fn parse_block_header_invalid_type() {
        let data = [b'x', 0, 0, 0];
        assert!(parse_block_header(&data).is_err());
    }

    #[test]
    fn parse_block_header_too_short() {
        let data = [b'r', 0];
        assert!(parse_block_header(&data).is_err());
    }

    #[test]
    fn log_block_compress_decompress_roundtrip() {
        // Build a fake log block: header + some data + restart table + restart count
        let mut block = Vec::new();
        block.push(b'g'); // Log block type
        block.extend_from_slice(&[0, 0, 0]); // block_len placeholder

        // Some fake record data
        let record_data = b"some log record data that should be compressed well";
        block.extend_from_slice(record_data);

        // A single restart at offset 4 + restart count of 1
        let mut restart_buf = [0u8; 3];
        crate::put_be24(&mut restart_buf, 4);
        block.extend_from_slice(&restart_buf);
        block.extend_from_slice(&1u16.to_be_bytes());

        let original_len = block.len();

        // Compress
        let compressed = compress_log_block(&mut block).expect("compression should succeed");
        // The compressed block should have the log block header
        assert_eq!(compressed[0], b'g');
        // block_len should be the uncompressed size
        let block_len = crate::get_be24(compressed[1..4].try_into().expect("3 bytes"));
        assert_eq!(block_len as usize, original_len);

        // Decompress
        let decompressed = decompress_log_block(&compressed).expect("decompression should succeed");
        assert_eq!(decompressed.len(), original_len);
        // Header preserved
        assert_eq!(decompressed[0], b'g');
        // Data matches original (after block_len was set)
        assert_eq!(&decompressed[4..4 + record_data.len()], record_data);
    }

    #[test]
    fn stack_table_path() {
        let stack = Stack {
            path: "/tmp/repo/.git/reftable".into(),
            tables: vec!["0x000000000001-0x000000000002-abcdef01.ref".to_owned()],
        };
        let expected = std::path::PathBuf::from("/tmp/repo/.git/reftable/0x000000000001-0x000000000002-abcdef01.ref");
        assert_eq!(stack.table_path("0x000000000001-0x000000000002-abcdef01.ref"), expected);
    }
}
