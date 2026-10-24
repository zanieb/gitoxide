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
    /// Object hash kind for object ids in the output table.
    pub object_hash: gix_hash::Kind,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            block_size: DEFAULT_BLOCK_SIZE,
            min_update_index: 1,
            max_update_index: 1,
            version: Version::V1,
            object_hash: gix_hash::Kind::shortest(),
        }
    }
}

/// Serialize a ref record into bytes with prefix compression.
///
/// `prev_name` is the name of the previous record for prefix compression.
/// Returns the serialized bytes.
pub fn serialize_ref_record(
    record: &RefRecord,
    prev_name: &[u8],
    min_update_index: u64,
    hash_size: usize,
) -> Result<Vec<u8>, Error> {
    let name = record.name();

    // Calculate shared prefix with previous record
    let prefix_len = shared_prefix_len(prev_name, name);
    let suffix = &name[prefix_len..];

    let (value_type, extra_data) = match &record.value {
        crate::RefRecordValue::Deletion => (0u8, Vec::new()),
        crate::RefRecordValue::Val1 { target } => {
            let mut data = Vec::new();
            append_object_id(&mut data, target, hash_size)?;
            (1, data)
        }
        crate::RefRecordValue::Val2 { target, target_value } => {
            let mut data = Vec::new();
            append_object_id(&mut data, target, hash_size)?;
            append_object_id(&mut data, target_value, hash_size)?;
            (2, data)
        }
        crate::RefRecordValue::Symref { target } => {
            let mut data = Vec::new();
            write_varint(target.len() as u64, &mut data);
            data.extend_from_slice(target);
            (3, data)
        }
    };

    let update_index_delta =
        record
            .update_index()
            .checked_sub(min_update_index)
            .ok_or(Error::UpdateIndexBelowMinimum {
                update_index: record.update_index(),
                min_update_index,
            })?;

    let mut out = Vec::new();
    write_varint(prefix_len as u64, &mut out);
    let suffix_and_type = ((suffix.len() as u64) << 3) | (value_type as u64);
    write_varint(suffix_and_type, &mut out);
    out.extend_from_slice(suffix);
    write_varint(update_index_delta, &mut out);
    out.extend_from_slice(&extra_data);

    Ok(out)
}

fn append_object_id(out: &mut Vec<u8>, id: &gix_hash::ObjectId, hash_size: usize) -> Result<(), Error> {
    let bytes = id.as_bytes();
    if bytes.len() != hash_size {
        return Err(Error::ObjectIdLengthMismatch {
            actual: bytes.len(),
            expected: hash_size,
        });
    }
    out.extend_from_slice(bytes);
    Ok(())
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
        object_hash: opts.object_hash,
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

/// Write ref records into one or more blocks at a given first-block file offset.
///
/// If `block_size` is non-zero, records are split so each emitted block fits
/// the requested size after accounting for `first_header_off` in the first
/// block. If `block_size` is zero, all records are written as one unaligned
/// block.
pub fn write_ref_blocks_at(
    records: &[RefRecord],
    min_update_index: u64,
    hash_size: usize,
    block_size: u32,
    first_header_off: usize,
) -> Result<Vec<Vec<u8>>, Error> {
    let mut blocks = Vec::new();
    let mut remaining = records;
    let mut header_off = first_header_off;
    loop {
        let (block, count) = encode_ref_block(
            remaining,
            min_update_index,
            hash_size,
            block_size,
            header_off,
            block_size != 0,
        )?;
        blocks.push(block);
        remaining = &remaining[count..];
        if remaining.is_empty() {
            return Ok(blocks);
        }
        header_off = 0;
    }
}

/// Write a single ref block, producing C Git-compatible output.
///
/// The first ref block shares space with the file header. Pass the file header
/// length as `header_off` for that block and zero for subsequent blocks. The
/// encoded block length and restart offsets include the file header length.
///
/// The writer records restart offsets for names stored without prefix compression until
/// the two-byte restart count reaches its maximum.
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
    encode_ref_block(records, min_update_index, hash_size, block_size, header_off, false).map(|(block, _)| block)
}

fn encode_ref_block(
    records: &[RefRecord],
    min_update_index: u64,
    hash_size: usize,
    block_size: u32,
    header_off: usize,
    allow_split: bool,
) -> Result<(Vec<u8>, usize), Error> {
    const MAX_BLOCK_SIZE: usize = 0x00ff_ffff;
    let max_len = if block_size == 0 {
        MAX_BLOCK_SIZE
    } else {
        block_size as usize
    };
    let mut block = vec![BlockType::Ref as u8, 0, 0, 0];
    let mut block_len = header_off.saturating_add(block.len() + 2);
    if max_len > MAX_BLOCK_SIZE || block_len > max_len {
        return Err(Error::BlockTooLarge {
            block_len: block_len.max(max_len),
            max_len: max_len.min(MAX_BLOCK_SIZE),
        });
    }
    let mut prev_name: &[u8] = &[];
    let mut restart_offsets = Vec::new();
    let mut count = 0;

    for record in records {
        if record.name().is_empty() {
            return Err(Error::EmptyRefName);
        }
        if prev_name >= record.name() {
            return Err(Error::RefRecordsOutOfOrder);
        }

        let use_prefix = if count % 16 == 0 { &[] } else { prev_name };
        let serialized = serialize_ref_record(record, use_prefix, min_update_index, hash_size)?;
        let is_restart = serialized[0] == 0 && restart_offsets.len() < usize::from(u16::MAX);
        let next_len = block_len
            .saturating_add(serialized.len())
            .saturating_add(3 * usize::from(is_restart));
        if next_len > max_len {
            if allow_split && count > 0 {
                break;
            }
            return Err(Error::BlockTooLarge {
                block_len: next_len,
                max_len,
            });
        }
        if is_restart {
            restart_offsets.push(block.len());
        }
        block.extend_from_slice(&serialized);
        block_len = next_len;
        prev_name = record.name();
        count += 1;
    }

    for offset in &restart_offsets {
        let mut buf = [0u8; 3];
        put_be24(&mut buf, (header_off + offset) as u32);
        block.extend_from_slice(&buf);
    }

    let restart_count = restart_offsets.len() as u16;
    block.extend_from_slice(&restart_count.to_be_bytes());

    put_be24(
        (&mut block[1..4]).try_into().expect("3 bytes for BE24"),
        block_len as u32,
    );

    if block_size > 0 {
        block.resize(max_len - header_off, 0);
    }

    Ok((block, count))
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
            object_hash: gix_hash::Kind::Sha1,
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
        let bytes = serialize_ref_record(&record, &[], 1, 20).expect("should serialize");
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
        let bytes = serialize_ref_record(&record, prev, 1, 20).expect("should serialize");
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
        let bytes = serialize_ref_record(&record, &[], 1, 20).expect("should serialize");
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
        let bytes = serialize_ref_record(&record, &[], 1, 20).expect("should serialize");
        let (parsed, _) = crate::parse_ref_record(&bytes, &[], 20, 1).expect("should parse");
        assert_eq!(parsed.name, "refs/heads/old");
        assert_eq!(parsed.update_index, 5);
        assert!(matches!(parsed.value, crate::RefRecordValue::Deletion));
    }

    #[test]
    fn serialize_ref_record_rejects_update_index_below_minimum() {
        let record = make_val1("refs/heads/main", 0xAA, 4);

        assert!(matches!(
            serialize_ref_record(&record, &[], 5, 20),
            Err(Error::UpdateIndexBelowMinimum {
                update_index: 4,
                min_update_index: 5
            })
        ));
    }

    #[test]
    fn serialize_ref_record_rejects_hash_size_mismatch() {
        let record = make_val1("refs/heads/main", 0xAA, 1);

        assert!(matches!(
            serialize_ref_record(&record, &[], 1, 32),
            Err(Error::ObjectIdLengthMismatch {
                actual: 20,
                expected: 32
            })
        ));
    }

    #[test]
    fn write_ref_block_rejects_update_index_below_minimum() {
        let records = vec![make_val1("refs/heads/main", 0xAA, 4)];

        assert!(matches!(
            write_ref_block(&records, 5, 20, 0),
            Err(Error::UpdateIndexBelowMinimum {
                update_index: 4,
                min_update_index: 5
            })
        ));
    }

    #[test]
    fn write_ref_block_rejects_empty_names() {
        let mut record = make_val1("refs/heads/main", 0xAA, 1);
        record.name = BString::from("");

        assert!(matches!(write_ref_block(&[record], 1, 20, 0), Err(Error::EmptyRefName)));
    }

    #[test]
    fn write_ref_block_rejects_unsorted_names() {
        let records = vec![
            make_val1("refs/heads/main", 0xAA, 1),
            make_val1("refs/heads/feature", 0xBB, 1),
        ];

        assert!(matches!(
            write_ref_block(&records, 1, 20, 0),
            Err(Error::RefRecordsOutOfOrder)
        ));
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
    fn write_first_ref_block_pads_to_remaining_file_block() {
        let records = vec![make_val1("refs/heads/main", 0xAA, 1)];
        let header_off = crate::HEADER_SIZE_V1;
        let block = write_ref_block_at(&records, 1, 20, 256, header_off).expect("should write");

        assert_eq!(
            header_off + block.len(),
            256,
            "first ref block should share the first file block with the header"
        );

        let block_len = crate::get_be24(block[1..4].try_into().expect("3 bytes"));
        assert!(
            block_len < 256,
            "block_len ({block_len}) should exclude padding while still including the header offset"
        );
    }

    #[test]
    fn aligned_ref_blocks_use_the_longest_prefix_that_fits() -> Result<(), Error> {
        let records: Vec<_> = (0..80)
            .map(|i| make_val1(&format!("refs/heads/topic-{i:03}"), i, 1))
            .collect();
        for (block_size, first_header_off) in [(96, 24), (512, 28)] {
            let blocks = write_ref_blocks_at(&records, 1, 20, block_size, first_header_off)?;
            let mut remaining = &records[..];
            for (i, block) in blocks.iter().enumerate() {
                let header_off = if i == 0 { first_header_off } else { 0 };
                let parsed = crate::block::read_ref_records_at(block, 20, 1, header_off)?;
                let count = parsed.len();
                assert_eq!(parsed, remaining[..count]);
                assert_eq!(
                    block,
                    &write_ref_block_at(&remaining[..count], 1, 20, block_size, header_off)?
                );
                if count < remaining.len() {
                    assert!(matches!(
                        write_ref_block_at(&remaining[..=count], 1, 20, block_size, header_off),
                        Err(Error::BlockTooLarge { .. })
                    ));
                }
                remaining = &remaining[count..];
            }
            assert!(remaining.is_empty());
        }
        Ok(())
    }

    #[test]
    fn ref_block_restart_count_fits_the_file_format() {
        let records: Vec<_> = (0..=usize::from(u16::MAX) * 16)
            .map(|i| crate::RefRecord {
                name: format!("refs/heads/{i:07}").into(),
                update_index: 1,
                value: crate::RefRecordValue::Deletion,
            })
            .collect();
        let block = write_ref_block(&records, 1, 20, 0).expect("one unaligned block");
        assert_eq!(&block[block.len() - 2..], &u16::MAX.to_be_bytes());
        assert_eq!(
            crate::block::read_ref_records(&block, 20, 1)
                .expect("valid block")
                .last(),
            records.last()
        );
    }

    #[test]
    fn ref_block_rejects_offsets_that_do_not_fit_in_three_bytes() {
        for header_off in [0x00ff_ffff, usize::MAX] {
            let err = write_ref_block_at(&[], 1, 20, 0, header_off).expect_err("block end cannot be encoded");
            assert!(matches!(err, Error::BlockTooLarge { .. }));
        }
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
