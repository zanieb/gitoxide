//! Writing reftable files.
//!
//! This module provides functionality to create new reftable files from
//! sorted reference records.

use crate::{put_be24, write_varint, BlockType, Error, RefRecord, Version, DEFAULT_BLOCK_SIZE};

/// Options for writing a reftable file.
#[derive(Debug, Clone)]
pub struct Options {
    /// Block size for the output file.
    pub block_size: u32,
    /// Minimum update index.
    pub min_update_index: u64,
    /// Maximum update index.
    pub max_update_index: u64,
    /// Reftable version.
    pub version: Version,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            block_size: DEFAULT_BLOCK_SIZE,
            min_update_index: 1,
            max_update_index: 1,
            version: Version::V1,
        }
    }
}

/// Serialize a ref record into bytes with prefix compression.
///
/// `prev_name` is the name of the previous record for prefix compression.
/// Returns the serialized bytes.
pub fn serialize_ref_record(record: &RefRecord, prev_name: &[u8], min_update_index: u64, hash_size: usize) -> Vec<u8> {
    let name = record.name();

    // Calculate shared prefix with previous record
    let prefix_len = shared_prefix_len(prev_name, name);
    let suffix = &name[prefix_len..];

    let (value_type, extra_data) = match &record.value {
        crate::RefRecordValue::Deletion => (0u8, Vec::new()),
        crate::RefRecordValue::Val1 { target } => {
            let mut data = Vec::with_capacity(hash_size);
            data.extend_from_slice(target.as_bytes());
            (1, data)
        }
        crate::RefRecordValue::Val2 { target, target_value } => {
            let mut data = Vec::with_capacity(2 * hash_size);
            data.extend_from_slice(target.as_bytes());
            data.extend_from_slice(target_value.as_bytes());
            (2, data)
        }
        crate::RefRecordValue::Symref { target } => {
            let mut data = Vec::new();
            write_varint(target.len() as u64, &mut data);
            data.extend_from_slice(target);
            (3, data)
        }
    };

    let update_index_delta = record.update_index().saturating_sub(min_update_index);

    let mut out = Vec::new();
    write_varint(prefix_len as u64, &mut out);
    let suffix_and_type = ((suffix.len() as u64) << 3) | (value_type as u64);
    write_varint(suffix_and_type, &mut out);
    out.extend_from_slice(suffix);
    write_varint(update_index_delta, &mut out);
    out.extend_from_slice(&extra_data);

    out
}

/// Write a complete reftable file header.
///
/// Returns exactly `header_size(version)` bytes.
pub fn write_header(opts: &Options) -> Vec<u8> {
    let header = crate::Header {
        version: opts.version,
        block_size: opts.block_size,
        min_update_index: opts.min_update_index,
        max_update_index: opts.max_update_index,
    };
    crate::serialize_header(&header)
}

/// Write a single ref block containing the given records.
///
/// The block is padded to `block_size` if specified. If `block_size` is 0,
/// no padding is applied (unaligned mode).
///
/// Returns the serialized block bytes.
///
/// **Note**: this uses gitoxide's internal block format (block-relative offsets).
/// For C Git-compatible output, use [`write_ref_block_at`].
pub fn write_ref_block(
    records: &[RefRecord],
    min_update_index: u64,
    hash_size: usize,
    block_size: u32,
) -> Result<Vec<u8>, Error> {
    write_ref_block_at(records, min_update_index, hash_size, block_size, 0)
}

/// Write a single ref block at a given file offset, producing C Git-compatible output.
///
/// `header_off` is the byte offset of this block within the reftable file.
/// C Git stores `block_len` as a file-level end offset, and restart offsets as
/// file-level positions. When `header_off` is 0, the output matches the legacy
/// gitoxide format (block-relative offsets).
///
/// Records with `prefix_len == 0` are automatically marked as restart points,
/// matching C Git's behavior where ANY record without prefix compression becomes
/// a restart regardless of the restart interval.
///
/// The block is padded to `block_size` if specified. If `block_size` is 0,
/// no padding is applied (unaligned mode).
pub fn write_ref_block_at(
    records: &[RefRecord],
    min_update_index: u64,
    hash_size: usize,
    block_size: u32,
    header_off: usize,
) -> Result<Vec<u8>, Error> {
    let mut block = Vec::new();

    // Block header: 4 bytes (type + BE24 block_len placeholder)
    block.push(BlockType::Ref as u8);
    block.extend_from_slice(&[0, 0, 0]);

    // Serialize records with prefix compression, tracking restart points.
    // In C Git, a record is a restart when its prefix_len == 0 (which always
    // happens at restart_interval boundaries because the previous key is suppressed,
    // but can also happen naturally when records share no prefix).
    let mut prev_name: Vec<u8> = Vec::new();
    let mut restart_offsets: Vec<u32> = Vec::new();
    let restart_interval = 16;

    for (i, record) in records.iter().enumerate() {
        let use_prefix = if i % restart_interval == 0 {
            &[] as &[u8]
        } else {
            &prev_name
        };

        let serialized = serialize_ref_record(record, use_prefix, min_update_index, hash_size);

        // Check if this record has prefix_len == 0 (the first varint in serialized).
        // If so, it's a restart point. The restart offset is a file-level position.
        let is_restart = serialized.first() == Some(&0);
        if is_restart {
            restart_offsets.push((header_off + block.len()) as u32);
        }

        block.extend_from_slice(&serialized);
        prev_name = record.name().to_vec();
    }

    // Write restart table: each entry is 3-byte BE24 file-level offset
    for offset in &restart_offsets {
        let mut buf = [0u8; 3];
        put_be24(&mut buf, *offset);
        block.extend_from_slice(&buf);
    }

    // Restart count as uint16 BE
    let restart_count = restart_offsets.len() as u16;
    block.extend_from_slice(&restart_count.to_be_bytes());

    // block_len = file-level end offset (header_off + current block size)
    // This matches C Git's `put_be24(w->block + 1 + w->header_off, w->next)`
    let block_len = (header_off + block.len()) as u32;
    put_be24((&mut block[1..4]).try_into().expect("3 bytes for BE24"), block_len);

    // Pad to block_size if needed
    if block_size > 0 && block.len() < block_size as usize {
        block.resize(block_size as usize, 0);
    }

    Ok(block)
}

/// Calculate the length of the shared prefix between two byte slices.
fn shared_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter()).take_while(|(a, b)| a == b).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bstr::BString;
    use gix_hash::ObjectId;

    fn make_val1(name: &str, oid_byte: u8, update_index: u64) -> RefRecord {
        RefRecord {
            name: BString::from(name),
            update_index,
            value: crate::RefRecordValue::Val1 {
                target: ObjectId::from_bytes_or_panic(&[oid_byte; 20]),
            },
        }
    }

    #[test]
    fn write_header_produces_correct_bytes() {
        let opts = Options {
            block_size: 4096,
            min_update_index: 1,
            max_update_index: 10,
            version: crate::Version::V1,
        };
        let header = write_header(&opts);
        assert_eq!(header.len(), crate::HEADER_SIZE_V1);
        assert_eq!(&header[0..4], crate::MAGIC);
        assert_eq!(header[4], opts.version.as_u8());

        // Verify roundtrip
        let parsed = crate::parse_header(&header).expect("should parse");
        assert_eq!(parsed.version, opts.version);
        assert_eq!(parsed.block_size, 4096);
        assert_eq!(parsed.min_update_index, 1);
        assert_eq!(parsed.max_update_index, 10);
    }

    #[test]
    fn serialize_ref_record_no_prefix() {
        let record = make_val1("refs/heads/main", 0xAA, 1);
        let bytes = serialize_ref_record(&record, &[], 1, 20);
        // Should be parseable
        let (parsed, _consumed) = crate::parse_ref_record(&bytes, &[], 20, 1).expect("should parse");
        assert_eq!(parsed.name(), b"refs/heads/main");
        assert_eq!(parsed.update_index, 1);
        match parsed.value {
            crate::RefRecordValue::Val1 { target } => {
                assert_eq!(target, ObjectId::from_bytes_or_panic(&[0xAA; 20]));
            }
            _ => panic!("expected Val1"),
        }
    }

    #[test]
    fn serialize_ref_record_with_prefix() {
        let record = make_val1("refs/heads/feature", 0xBB, 1);
        let prev = b"refs/heads/main";
        let bytes = serialize_ref_record(&record, prev, 1, 20);
        // Parse back with the prefix
        let (parsed, _) = crate::parse_ref_record(&bytes, prev, 20, 1).expect("should parse");
        assert_eq!(parsed.name(), b"refs/heads/feature");
    }

    #[test]
    fn serialize_symref_record() {
        let record = RefRecord {
            name: BString::from("HEAD"),
            update_index: 1,
            value: crate::RefRecordValue::Symref {
                target: BString::from("refs/heads/main"),
            },
        };
        let bytes = serialize_ref_record(&record, &[], 1, 20);
        let (parsed, _) = crate::parse_ref_record(&bytes, &[], 20, 1).expect("should parse");
        assert_eq!(parsed.name, "HEAD");
        match parsed.value {
            crate::RefRecordValue::Symref { target } => {
                assert_eq!(target, "refs/heads/main");
            }
            _ => panic!("expected Symref"),
        }
    }

    #[test]
    fn serialize_deletion_record() {
        let record = RefRecord {
            name: BString::from("refs/heads/old"),
            update_index: 5,
            value: crate::RefRecordValue::Deletion,
        };
        let bytes = serialize_ref_record(&record, &[], 1, 20);
        let (parsed, _) = crate::parse_ref_record(&bytes, &[], 20, 1).expect("should parse");
        assert_eq!(parsed.name, "refs/heads/old");
        assert_eq!(parsed.update_index, 5);
        assert!(matches!(parsed.value, crate::RefRecordValue::Deletion));
    }

    #[test]
    fn write_ref_block_roundtrip() {
        let records = vec![
            make_val1("refs/heads/alpha", 0xAA, 1),
            make_val1("refs/heads/beta", 0xBB, 1),
            make_val1("refs/heads/gamma", 0xCC, 1),
        ];

        let block = write_ref_block(&records, 1, 20, 0).expect("should write");
        assert!(block.len() > 4, "block should have header + records");

        // The block should start with 'r'
        assert_eq!(block[0], b'r');

        // Verify block_len in header
        let block_len = crate::get_be24(block[1..4].try_into().expect("3 bytes"));
        assert_eq!(block_len as usize, block.len(), "block_len should match total size");

        // Read back the records
        let parsed = crate::block::read_ref_records(&block, 20, 1).expect("should read");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].name(), b"refs/heads/alpha");
        assert_eq!(parsed[1].name(), b"refs/heads/beta");
        assert_eq!(parsed[2].name(), b"refs/heads/gamma");
    }

    #[test]
    fn write_ref_block_with_padding() {
        let records = vec![make_val1("refs/heads/main", 0xAA, 1)];
        let block = write_ref_block(&records, 1, 20, 256).expect("should write");
        assert_eq!(block.len(), 256, "block should be padded to block_size");

        // block_len should be the meaningful data size, not the padded size
        let block_len = crate::get_be24(block[1..4].try_into().expect("3 bytes"));
        assert!(
            block_len < 256,
            "block_len ({block_len}) should be less than padded size"
        );
    }

    #[test]
    fn write_ref_block_restart_offsets_are_absolute() {
        // With more than 16 records, we get at least 2 restart points.
        let mut records = Vec::new();
        for i in 0..20 {
            records.push(make_val1(
                &format!("refs/heads/{i:04}"),
                (i as u8).wrapping_mul(0x11),
                1,
            ));
        }

        let block = write_ref_block(&records, 1, 20, 0).expect("should write");
        let block_len = crate::get_be24(block[1..4].try_into().expect("3 bytes")) as usize;

        // Read restart count
        let restart_count = u16::from_be_bytes([block[block_len - 2], block[block_len - 1]]);
        assert!(restart_count >= 2, "should have at least 2 restart points");

        // First restart offset should be 4 (right after the block header)
        let restart_table_start = block_len - 2 - (restart_count as usize) * 3;
        let first_restart = crate::get_be24(
            block[restart_table_start..restart_table_start + 3]
                .try_into()
                .expect("3 bytes"),
        );
        assert_eq!(
            first_restart, 4,
            "first restart offset should be 4 (absolute, right after header)"
        );

        // Verify all records can be read back
        let parsed = crate::block::read_ref_records(&block, 20, 1).expect("should read");
        assert_eq!(parsed.len(), 20);
    }

    #[test]
    fn shared_prefix_len_works() {
        assert_eq!(shared_prefix_len(b"refs/heads/main", b"refs/heads/master"), 13);
        assert_eq!(shared_prefix_len(b"abc", b"abc"), 3);
        assert_eq!(shared_prefix_len(b"abc", b"xyz"), 0);
        assert_eq!(shared_prefix_len(b"", b"xyz"), 0);
    }
}
