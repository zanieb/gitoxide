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
        let content_size = (header.block_len as usize)
            .checked_sub(header_off)
            .ok_or(Error::UnexpectedEof)?;
        if content_size > block_data.len() {
            return Err(Error::UnexpectedEof);
        }
        content_size
    } else {
        block_data.len()
    };

    if data_end < header_size + 2 {
        return Err(Error::UnexpectedEof);
    }

    // Read restart count from the last 2 bytes of the data portion
    let restart_count = u16::from_be_bytes([block_data[data_end - 2], block_data[data_end - 1]]);

    // The restart offsets occupy restart_count * 3 bytes before the restart_count field
    let restart_table_size = (restart_count as usize) * 3 + 2;
    let available_after_header = data_end - header_size;
    if restart_table_size > available_after_header {
        return Err(Error::UnexpectedEof);
    }
    let records_end = data_end - restart_table_size;

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
            Err(err) => return Err(err),
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
pub fn compress_log_block(uncompressed_block: &mut [u8]) -> Result<Vec<u8>, Error> {
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

/// Read all reference records from a complete reftable file.
///
/// This parses the table header and footer, then walks consecutive ref blocks
/// from the start of the file until a non-ref block, padding, or the footer is
/// reached.
pub fn read_table_ref_records(data: &[u8]) -> Result<Vec<RefRecord>, Error> {
    let header = crate::parse_header(data)?;
    let file_header_size = crate::header_size(header.version);
    let footer_size = crate::footer_size(header.version);
    if data.len() < file_header_size + footer_size {
        return Err(Error::UnexpectedEof);
    }

    let footer_start = data.len() - footer_size;
    let footer = crate::parse_footer(&data[footer_start..])?;
    if footer.header != header {
        return Err(Error::HeaderFooterMismatch);
    }
    let hash_size = header.object_hash.len_in_bytes();

    let mut records = Vec::new();
    let mut block_start = file_header_size;
    let mut is_first_block = true;

    while block_start + 4 <= footer_start {
        let block_data = &data[block_start..footer_start];
        if block_data.is_empty() || block_data[0] == 0 {
            break;
        }

        let (block_header, _) = parse_block_header(block_data)?;
        if block_header.block_type != BlockType::Ref {
            break;
        }

        let block_end = if header.block_size > 0 {
            let next_boundary = if is_first_block {
                header.block_size as usize
            } else {
                block_start + header.block_size as usize
            };
            next_boundary.min(footer_start)
        } else {
            let content_len = if is_first_block {
                (block_header.block_len as usize).saturating_sub(file_header_size)
            } else {
                block_header.block_len as usize
            };
            block_start.saturating_add(content_len).min(footer_start)
        };

        let header_off = if is_first_block { file_header_size } else { 0 };
        let mut block_records = read_ref_records_at(
            &data[block_start..block_end],
            hash_size,
            footer.header.min_update_index,
            header_off,
        )?;
        records.append(&mut block_records);

        if block_end <= block_start {
            break;
        }
        block_start = block_end;
        is_first_block = false;
    }

    Ok(records)
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

fn validate_table_name(table_name: &str) -> Result<(), Error> {
    let mut components = std::path::Path::new(table_name).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) if !table_name.is_empty() => Ok(()),
        _ => Err(Error::InvalidTableName {
            name: table_name.to_owned(),
        }),
    }
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
            .map(|line| {
                validate_table_name(line)?;
                Ok(line.to_owned())
            })
            .collect::<Result<_, Error>>()?;
        Ok(Stack { path, tables })
    }

    /// Get the full path to a specific table file.
    pub fn table_path(&self, table_name: &str) -> std::path::PathBuf {
        self.path.join(table_name)
    }

    /// Read the visible reference records from all tables in this stack.
    ///
    /// Tables are applied in `tables.list` order, oldest to newest. Newer records
    /// replace older records with the same name, and deletion records hide older
    /// values.
    pub fn ref_records(&self) -> Result<Vec<RefRecord>, Error> {
        let mut visible = std::collections::BTreeMap::<Vec<u8>, RefRecord>::new();
        for table_name in &self.tables {
            validate_table_name(table_name)?;
            let table_data = std::fs::read(self.table_path(table_name))?;
            for record in read_table_ref_records(&table_data)? {
                if matches!(&record.value, crate::RefRecordValue::Deletion) {
                    visible.remove(record.name());
                } else {
                    visible.insert(record.name().to_vec(), record);
                }
            }
        }
        Ok(visible.into_values().collect())
    }

    /// Find a visible reference record by name.
    pub fn find_ref(&self, name: &[u8]) -> Result<Option<RefRecord>, Error> {
        Ok(self.ref_records()?.into_iter().find(|record| record.name() == name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(byte: u8) -> gix_hash::ObjectId {
        gix_hash::ObjectId::from_bytes_or_panic(&[byte; 20])
    }

    fn val1(name: &str, byte: u8, update_index: u64) -> RefRecord {
        RefRecord {
            name: bstr::BString::from(name),
            update_index,
            value: crate::RefRecordValue::Val1 { target: oid(byte) },
        }
    }

    fn deletion(name: &str, update_index: u64) -> RefRecord {
        RefRecord {
            name: bstr::BString::from(name),
            update_index,
            value: crate::RefRecordValue::Deletion,
        }
    }

    fn table_with_records(records: &[RefRecord], min_update_index: u64, max_update_index: u64) -> Vec<u8> {
        let opts = crate::write::Options {
            block_size: crate::DEFAULT_BLOCK_SIZE,
            min_update_index,
            max_update_index,
            version: crate::Version::V1,
            object_hash: gix_hash::Kind::Sha1,
        };
        let header = crate::write::write_header(&opts);
        let block = crate::write::write_ref_block_at(records, opts.min_update_index, 20, opts.block_size, header.len())
            .expect("block should write");
        let footer = crate::Footer {
            header: crate::Header {
                version: opts.version,
                block_size: opts.block_size,
                min_update_index: opts.min_update_index,
                max_update_index: opts.max_update_index,
                object_hash: gix_hash::Kind::Sha1,
            },
            ref_index_offset: 0,
            obj_offset: 0,
            obj_id_len: 0,
            obj_index_offset: 0,
            log_offset: 0,
            log_index_offset: 0,
        };
        let mut table = Vec::new();
        table.extend_from_slice(&header);
        table.extend_from_slice(&block);
        table.extend_from_slice(&crate::serialize_footer(&footer));
        table
    }

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
    fn read_ref_records_propagates_record_errors() {
        let mut block = Vec::new();
        block.push(b'r');
        block.extend_from_slice(&[0, 0, 0]);
        crate::write_varint(0, &mut block);
        crate::write_varint(7, &mut block);
        crate::write_varint(0, &mut block);
        block.extend_from_slice(&0u16.to_be_bytes());
        let block_len = block.len() as u32;
        crate::put_be24((&mut block[1..4]).try_into().expect("3 bytes"), block_len);

        assert!(matches!(
            read_ref_records(&block, 20, 1),
            Err(Error::InvalidRefValueType { value_type: 7 })
        ));
    }

    #[test]
    fn read_ref_records_rejects_truncated_restart_table() {
        let mut block = Vec::new();
        block.push(b'r');
        block.extend_from_slice(&[0, 0, 0]);
        block.extend_from_slice(&1u16.to_be_bytes());
        let block_len = block.len() as u32;
        crate::put_be24((&mut block[1..4]).try_into().expect("3 bytes"), block_len);

        assert!(matches!(read_ref_records(&block, 20, 1), Err(Error::UnexpectedEof)));
    }

    #[test]
    fn read_ref_records_rejects_block_len_before_header_offset() {
        let mut block = Vec::new();
        block.push(b'r');
        block.extend_from_slice(&[0, 0, 0]);
        block.extend_from_slice(&0u16.to_be_bytes());
        crate::put_be24((&mut block[1..4]).try_into().expect("3 bytes"), 10);

        assert!(matches!(
            read_ref_records_at(&block, 20, 1, crate::HEADER_SIZE_V1),
            Err(Error::UnexpectedEof)
        ));
    }

    #[test]
    fn read_ref_records_rejects_block_len_beyond_data() {
        let mut block = Vec::new();
        block.push(b'r');
        block.extend_from_slice(&[0, 0, 0]);
        block.extend_from_slice(&0u16.to_be_bytes());
        crate::put_be24((&mut block[1..4]).try_into().expect("3 bytes"), 100);

        assert!(matches!(read_ref_records(&block, 20, 1), Err(Error::UnexpectedEof)));
    }

    #[test]
    fn read_ref_records_rejects_missing_restart_count() {
        let mut block = Vec::new();
        block.push(b'r');
        block.extend_from_slice(&[0, 0, 0]);
        let block_len = block.len() as u32;
        crate::put_be24((&mut block[1..4]).try_into().expect("3 bytes"), block_len);

        assert!(matches!(read_ref_records(&block, 20, 1), Err(Error::UnexpectedEof)));
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
    fn read_table_ref_records_reads_complete_table() {
        let records = vec![
            RefRecord {
                name: bstr::BString::from("HEAD"),
                update_index: 2,
                value: crate::RefRecordValue::Symref {
                    target: bstr::BString::from("refs/heads/main"),
                },
            },
            val1("refs/heads/main", 0xaa, 2),
        ];
        let table = table_with_records(&records, 2, 2);

        let actual = read_table_ref_records(&table).expect("table should read");
        assert_eq!(actual, records);
    }

    #[test]
    fn read_table_ref_records_rejects_footer_header_mismatch() {
        let records = vec![val1("refs/heads/main", 0xaa, 1)];
        let opts = crate::write::Options {
            block_size: crate::DEFAULT_BLOCK_SIZE,
            min_update_index: 1,
            max_update_index: 1,
            version: crate::Version::V1,
            object_hash: gix_hash::Kind::Sha1,
        };
        let header = crate::write::write_header(&opts);
        let block = crate::write::write_ref_block_at(
            records.as_slice(),
            opts.min_update_index,
            20,
            opts.block_size,
            header.len(),
        )
        .expect("block should write");
        let footer = crate::Footer {
            header: crate::Header {
                version: opts.version,
                block_size: opts.block_size,
                min_update_index: 2,
                max_update_index: opts.max_update_index,
                object_hash: gix_hash::Kind::Sha1,
            },
            ref_index_offset: 0,
            obj_offset: 0,
            obj_id_len: 0,
            obj_index_offset: 0,
            log_offset: 0,
            log_index_offset: 0,
        };

        let mut table = Vec::new();
        table.extend_from_slice(&header);
        table.extend_from_slice(&block);
        table.extend_from_slice(&crate::serialize_footer(&footer));

        assert!(matches!(
            read_table_ref_records(&table),
            Err(Error::HeaderFooterMismatch)
        ));
    }

    #[test]
    fn stack_ref_records_applies_newer_tables_and_deletions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let reftable_dir = dir.path().join("reftable");
        std::fs::create_dir(&reftable_dir).expect("reftable dir");

        let first = table_with_records(
            &[val1("refs/heads/main", 0xaa, 1), val1("refs/heads/old", 0xbb, 1)],
            1,
            1,
        );
        let second = table_with_records(&[val1("refs/heads/main", 0xcc, 2), deletion("refs/heads/old", 2)], 2, 2);
        std::fs::write(reftable_dir.join("0x000000000001-0x000000000001-00000001.ref"), first).expect("first table");
        std::fs::write(reftable_dir.join("0x000000000002-0x000000000002-00000002.ref"), second).expect("second table");
        std::fs::write(
            reftable_dir.join("tables.list"),
            "0x000000000001-0x000000000001-00000001.ref\n0x000000000002-0x000000000002-00000002.ref\n",
        )
        .expect("tables.list");

        let stack = Stack::open(&reftable_dir).expect("stack should open");
        let refs = stack.ref_records().expect("refs should read");
        assert_eq!(refs.len(), 1, "newer deletion should hide old ref: {refs:?}");
        assert_eq!(refs[0].name(), b"refs/heads/main");
        match &refs[0].value {
            crate::RefRecordValue::Val1 { target } => assert_eq!(*target, oid(0xcc)),
            other => panic!("expected Val1, got {other:?}"),
        }

        assert!(
            stack
                .find_ref(b"refs/heads/old")
                .expect("lookup should succeed")
                .is_none(),
            "deleted ref should not be visible"
        );
        assert!(
            stack
                .find_ref(b"refs/heads/main")
                .expect("lookup should succeed")
                .is_some(),
            "updated ref should be visible"
        );
    }

    #[test]
    fn stack_open_rejects_parent_table_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let reftable_dir = dir.path().join("reftable");
        std::fs::create_dir(&reftable_dir).expect("reftable dir");
        std::fs::write(reftable_dir.join("tables.list"), "../outside.ref\n").expect("tables.list");

        assert!(matches!(
            Stack::open(&reftable_dir),
            Err(Error::InvalidTableName { name }) if name == "../outside.ref"
        ));
    }

    #[test]
    fn stack_ref_records_rejects_parent_table_names() {
        let stack = Stack {
            path: "/tmp/repo/.git/reftable".into(),
            tables: vec!["../outside.ref".to_owned()],
        };

        assert!(matches!(
            stack.ref_records(),
            Err(Error::InvalidTableName { name }) if name == "../outside.ref"
        ));
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
