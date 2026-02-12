#![deny(rust_2018_idioms)]
#![forbid(unsafe_code)]

//! A crate implementing the reftable binary format for git reference storage.
//!
//! The reftable format stores references in sorted blocks within binary table files.
//! Multiple tables can be stacked, with a `tables.list` manifest tracking the active set.
//!
//! This crate provides:
//! - Reading reftable files (header, blocks, records)
//! - Writing reftable files
//! - Managing reftable stacks (the `tables.list` manifest)
//!
//! ## Format overview
//!
//! A reftable file consists of:
//! 1. A 24-byte file header with magic bytes "REFT"
//! 2. Ref blocks containing sorted reference records
//! 3. Optional obj blocks (reverse index: object -> ref names)
//! 4. Optional log blocks (reflog entries, may be zlib-compressed)
//! 5. A footer with offsets and a file checksum

use bstr::BString;
use gix_hash::ObjectId;

pub mod block;
pub mod write;

/// The magic bytes at the start of a reftable file.
pub const MAGIC: &[u8; 4] = b"REFT";

/// The reftable format version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Version {
    /// Version 1 of the reftable format (SHA-1 only).
    V1 = 1,
    /// Version 2 of the reftable format (supports SHA-256).
    V2 = 2,
}

impl Version {
    /// Parse a version number from its byte representation.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(Version::V1),
            2 => Some(Version::V2),
            _ => None,
        }
    }

    /// Return the version number as a `u8`.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// The minimum block size.
pub const MIN_BLOCK_SIZE: u32 = 256;

/// The default block size (4 KiB).
pub const DEFAULT_BLOCK_SIZE: u32 = 4096;

/// Size of the file header for version 1 (24 bytes).
pub const HEADER_SIZE_V1: usize = 24;

/// Size of the file header for version 2 (28 bytes).
pub const HEADER_SIZE_V2: usize = 28;

/// Size of the file footer for version 1 (68 bytes).
pub const FOOTER_SIZE_V1: usize = 68;

/// Size of the file footer for version 2 (72 bytes).
pub const FOOTER_SIZE_V2: usize = 72;

/// Block types in a reftable file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockType {
    /// Reference records.
    Ref = b'r',
    /// Log records (reflog entries).
    Log = b'g',
    /// Object index records (reverse mapping).
    Obj = b'o',
    /// Index block (pointing to other blocks).
    Index = b'i',
}

impl BlockType {
    /// Parse a block type from its byte representation.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            b'r' => Some(BlockType::Ref),
            b'g' => Some(BlockType::Log),
            b'o' => Some(BlockType::Obj),
            b'i' => Some(BlockType::Index),
            _ => None,
        }
    }
}

/// A reference record in a reftable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefRecord {
    /// The reference name.
    pub name: BString,
    /// Update index (transaction counter).
    pub update_index: u64,
    /// The value of this reference record.
    pub value: RefRecordValue,
}

/// The value part of a reference record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefRecordValue {
    /// A direct reference pointing to an object.
    Val1 {
        /// The target object id.
        target: ObjectId,
    },
    /// A reference with both a target and a peeled target (e.g., annotated tag).
    Val2 {
        /// The target object id (e.g., tag object).
        target: ObjectId,
        /// The peeled target (e.g., the commit the tag points to).
        target_value: ObjectId,
    },
    /// A symbolic reference.
    Symref {
        /// The target reference name.
        target: BString,
    },
    /// A deletion marker.
    Deletion,
}

impl RefRecord {
    /// Get the reference name.
    pub fn name(&self) -> &[u8] {
        self.name.as_ref()
    }

    /// Get the update index.
    pub fn update_index(&self) -> u64 {
        self.update_index
    }
}

/// A log record (reflog entry) in a reftable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    /// The reference name.
    pub ref_name: BString,
    /// Update index (transaction counter), used for ordering.
    pub update_index: u64,
    /// The old value of the reference (null OID if created).
    pub old_id: ObjectId,
    /// The new value of the reference (null OID if deleted).
    pub new_id: ObjectId,
    /// Committer name.
    pub name: BString,
    /// Committer email.
    pub email: BString,
    /// Timestamp in seconds since epoch.
    pub time: u64,
    /// Timezone offset in minutes.
    pub tz_offset: i16,
    /// The reflog message.
    pub message: BString,
}

/// The file header of a reftable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Format version.
    pub version: Version,
    /// Block size in bytes (0 means unaligned).
    pub block_size: u32,
    /// Minimum update index for records in this table.
    pub min_update_index: u64,
    /// Maximum update index for records in this table.
    pub max_update_index: u64,
}

/// The file footer of a reftable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    /// Copy of the header fields.
    pub header: Header,
    /// Offset of the first ref index block (0 if no index).
    pub ref_index_offset: u64,
    /// Offset and object_id_len packed field for obj section.
    pub obj_offset: u64,
    /// Object ID prefix length (low 5 bits of the packed obj field).
    pub obj_id_len: u8,
    /// Offset of the obj index (0 if no obj index).
    pub obj_index_offset: u64,
    /// Offset of the first log block (0 if no log blocks).
    pub log_offset: u64,
    /// Offset of the log index (0 if no log index).
    pub log_index_offset: u64,
}

/// Errors that can occur when reading a reftable.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("invalid reftable magic bytes")]
    InvalidMagic,
    #[error("unsupported reftable version {version}")]
    UnsupportedVersion { version: u8 },
    #[error("block size {size} is too small (minimum {MIN_BLOCK_SIZE})")]
    BlockSizeTooSmall { size: u32 },
    #[error("I/O error")]
    Io(#[from] std::io::Error),
    #[error("invalid block type: {byte:#x}")]
    InvalidBlockType { byte: u8 },
    #[error("unexpected end of data")]
    UnexpectedEof,
    #[error("invalid varint encoding")]
    InvalidVarint,
    #[error("invalid ref record value type: {value_type}")]
    InvalidRefValueType { value_type: u8 },
    #[error("CRC32 checksum mismatch: expected {expected:#010x}, got {actual:#010x}")]
    Crc32Mismatch { expected: u32, actual: u32 },
    #[error("zlib decompression error")]
    ZlibError,
}

/// Read a varint from a byte slice using the reftable MSB-first encoding with +1 adjustment.
///
/// This is NOT standard LEB128. The reftable format uses a big-endian varint where
/// continuation bytes apply a +1 adjustment (since the remainder cannot be 0 when
/// the continuation bit is set, 0 need not be encoded).
///
/// Matches `get_var_int` in C Git's `reftable/record.c`.
pub fn read_varint(data: &[u8]) -> Result<(u64, usize), Error> {
    if data.is_empty() {
        return Err(Error::UnexpectedEof);
    }

    let mut val: u64 = (data[0] & 0x7f) as u64;
    let mut i = 1usize;

    while data[i - 1] & 0x80 != 0 {
        // +1 adjustment: since the continuation bit is set, the remaining value
        // cannot be 0. We add 1 to undo the -1 applied during encoding.
        val = val.checked_add(1).ok_or(Error::InvalidVarint)?;
        // Overflow check: if high bits are set, shifting would overflow
        if val & (!0u64 << (64 - 7)) != 0 {
            return Err(Error::InvalidVarint);
        }
        if i >= data.len() {
            return Err(Error::UnexpectedEof);
        }
        val = (val << 7) + (data[i] & 0x7f) as u64;
        i += 1;
    }

    Ok((val, i))
}

/// Encode a value as a varint using the reftable MSB-first encoding with +1 adjustment.
///
/// Bytes are written most-significant first. When the continuation bit is set, the
/// value is decremented by 1 before masking (since 0 cannot occur as a remainder
/// when the continuation bit is set).
///
/// Matches `put_var_int` in C Git's `reftable/record.c`.
pub fn write_varint(value: u64, out: &mut Vec<u8>) {
    let mut varint = [0u8; 10];
    let mut pos = varint.len() - 1;
    let mut val = value;

    // Store the lowest 7 bits in the last position (no continuation bit).
    varint[pos] = (val & 0x7f) as u8;

    // Process remaining bits MSB-first with the +1 adjustment.
    while {
        val >>= 7;
        val != 0
    } {
        val -= 1; // the +1 adjustment: we subtract 1 during encoding
        pos -= 1;
        varint[pos] = 0x80 | (val & 0x7f) as u8;
    }

    out.extend_from_slice(&varint[pos..]);
}

/// Write a 3-byte big-endian 24-bit value to `out`.
pub fn put_be24(out: &mut [u8; 3], val: u32) {
    out[0] = (val >> 16) as u8;
    out[1] = (val >> 8) as u8;
    out[2] = val as u8;
}

/// Read a 3-byte big-endian 24-bit value from `data`.
pub fn get_be24(data: &[u8; 3]) -> u32 {
    ((data[0] as u32) << 16) | ((data[1] as u32) << 8) | (data[2] as u32)
}

/// Return the header size for a given reftable version.
pub fn header_size(version: Version) -> usize {
    match version {
        Version::V1 => HEADER_SIZE_V1,
        Version::V2 => HEADER_SIZE_V2,
    }
}

/// Return the footer size for a given reftable version.
pub fn footer_size(version: Version) -> usize {
    match version {
        Version::V1 => FOOTER_SIZE_V1,
        Version::V2 => FOOTER_SIZE_V2,
    }
}

/// Parse a reftable file header.
///
/// The header is 24 bytes for version 1, 28 bytes for version 2:
/// - Bytes 0-3: magic "REFT"
/// - Byte 4: version (1 or 2)
/// - Bytes 5-7: BE24 block_size
/// - Bytes 8-15: BE64 min_update_index
/// - Bytes 16-23: BE64 max_update_index
/// - (Version 2 only) Bytes 24-27: BE32 hash algorithm ID
pub fn parse_header(data: &[u8]) -> Result<Header, Error> {
    if data.len() < HEADER_SIZE_V1 {
        return Err(Error::UnexpectedEof);
    }
    if &data[0..4] != MAGIC {
        return Err(Error::InvalidMagic);
    }
    let version = Version::from_byte(data[4]).ok_or(Error::UnsupportedVersion { version: data[4] })?;

    // Bytes 5-7: 3-byte big-endian block_size
    let block_size = get_be24(data[5..8].try_into().expect("3 bytes for BE24"));
    let min_update_index = u64::from_be_bytes(data[8..16].try_into().expect("8 bytes"));
    let max_update_index = u64::from_be_bytes(data[16..24].try_into().expect("8 bytes"));

    Ok(Header {
        version,
        block_size,
        min_update_index,
        max_update_index,
    })
}

/// Serialize a reftable file header into bytes.
///
/// Returns exactly 24 bytes for version 1, 28 bytes for version 2.
pub fn serialize_header(header: &Header) -> Vec<u8> {
    let size = header_size(header.version);
    let mut out = vec![0u8; size];
    out[0..4].copy_from_slice(MAGIC);
    out[4] = header.version.as_u8();
    put_be24(
        (&mut out[5..8]).try_into().expect("3 bytes for BE24"),
        header.block_size,
    );
    out[8..16].copy_from_slice(&header.min_update_index.to_be_bytes());
    out[16..24].copy_from_slice(&header.max_update_index.to_be_bytes());
    out
}

/// Parse a reftable file footer.
///
/// The footer has the same initial layout as the header, followed by
/// section statistics and a CRC32 checksum.
///
/// For version 1 (68 bytes):
/// - Bytes 0-23: header copy
/// - Bytes 24-31: BE64 ref_stats.index_offset
/// - Bytes 32-39: BE64 (obj_stats.offset << 5 | obj_id_len)
/// - Bytes 40-47: BE64 obj_stats.index_offset
/// - Bytes 48-55: BE64 log_stats.offset
/// - Bytes 56-63: BE64 log_stats.index_offset
/// - Bytes 64-67: CRC32 of bytes 0-63
pub fn parse_footer(data: &[u8]) -> Result<Footer, Error> {
    let header = parse_header(data)?;
    let fsize = footer_size(header.version);
    if data.len() < fsize {
        return Err(Error::UnexpectedEof);
    }

    let hsize = header_size(header.version);

    // Verify CRC32 checksum
    let expected_crc = u32::from_be_bytes(data[fsize - 4..fsize].try_into().expect("4 bytes"));
    let actual_crc = crc32fast::hash(&data[..fsize - 4]);
    if expected_crc != actual_crc {
        return Err(Error::Crc32Mismatch {
            expected: expected_crc,
            actual: actual_crc,
        });
    }

    let mut pos = hsize;

    let ref_index_offset = u64::from_be_bytes(data[pos..pos + 8].try_into().expect("8 bytes"));
    pos += 8;

    let obj_packed = u64::from_be_bytes(data[pos..pos + 8].try_into().expect("8 bytes"));
    pos += 8;
    let obj_offset = obj_packed >> 5;
    let obj_id_len = (obj_packed & 0x1f) as u8;

    let obj_index_offset = u64::from_be_bytes(data[pos..pos + 8].try_into().expect("8 bytes"));
    pos += 8;

    let log_offset = u64::from_be_bytes(data[pos..pos + 8].try_into().expect("8 bytes"));
    pos += 8;

    let log_index_offset = u64::from_be_bytes(data[pos..pos + 8].try_into().expect("8 bytes"));

    Ok(Footer {
        header,
        ref_index_offset,
        obj_offset,
        obj_id_len,
        obj_index_offset,
        log_offset,
        log_index_offset,
    })
}

/// Serialize a reftable file footer into bytes with CRC32 checksum.
pub fn serialize_footer(footer: &Footer) -> Vec<u8> {
    let hsize = header_size(footer.header.version);
    let fsize = footer_size(footer.header.version);
    let mut out = vec![0u8; fsize];

    // Write header portion
    let header_bytes = serialize_header(&footer.header);
    out[..hsize].copy_from_slice(&header_bytes);

    let mut pos = hsize;

    out[pos..pos + 8].copy_from_slice(&footer.ref_index_offset.to_be_bytes());
    pos += 8;

    let obj_packed = (footer.obj_offset << 5) | (footer.obj_id_len as u64);
    out[pos..pos + 8].copy_from_slice(&obj_packed.to_be_bytes());
    pos += 8;

    out[pos..pos + 8].copy_from_slice(&footer.obj_index_offset.to_be_bytes());
    pos += 8;

    out[pos..pos + 8].copy_from_slice(&footer.log_offset.to_be_bytes());
    pos += 8;

    out[pos..pos + 8].copy_from_slice(&footer.log_index_offset.to_be_bytes());
    pos += 8;

    // CRC32 of everything except the last 4 bytes
    let crc = crc32fast::hash(&out[..pos]);
    out[pos..pos + 4].copy_from_slice(&crc.to_be_bytes());

    out
}

/// Parse a ref record from a byte slice.
///
/// `prefix` is the shared prefix from the previous record in the block (for prefix compression).
/// Returns the record and the number of bytes consumed.
pub fn parse_ref_record(
    data: &[u8],
    prefix: &[u8],
    hash_size: usize,
    min_update_index: u64,
) -> Result<(RefRecord, usize), Error> {
    let mut pos = 0;

    // Read prefix length
    let (prefix_len, n) = read_varint(&data[pos..])?;
    pos += n;
    let prefix_len = prefix_len as usize;

    // Read suffix length (includes value type in low 3 bits)
    let (suffix_and_type, n) = read_varint(&data[pos..])?;
    pos += n;

    let value_type = (suffix_and_type & 0x7) as u8;
    let suffix_len = (suffix_and_type >> 3) as usize;

    // Validate lengths before allocating to prevent OOM on crafted input.
    if prefix_len > prefix.len() {
        return Err(Error::UnexpectedEof);
    }
    if pos + suffix_len > data.len() {
        return Err(Error::UnexpectedEof);
    }
    let mut name = Vec::with_capacity(prefix_len + suffix_len);
    name.extend_from_slice(&prefix[..prefix_len]);
    name.extend_from_slice(&data[pos..pos + suffix_len]);
    pos += suffix_len;

    // Read update_index delta
    let (update_index_delta, n) = read_varint(&data[pos..])?;
    pos += n;
    let update_index = min_update_index + update_index_delta;

    let name = BString::from(name);

    let record_value = match value_type {
        0 => {
            // Deletion
            RefRecordValue::Deletion
        }
        1 => {
            // val1: single OID
            if pos + hash_size > data.len() {
                return Err(Error::UnexpectedEof);
            }
            let target = ObjectId::from_bytes_or_panic(&data[pos..pos + hash_size]);
            pos += hash_size;
            RefRecordValue::Val1 { target }
        }
        2 => {
            // val2: OID + peeled OID
            if pos + 2 * hash_size > data.len() {
                return Err(Error::UnexpectedEof);
            }
            let target = ObjectId::from_bytes_or_panic(&data[pos..pos + hash_size]);
            pos += hash_size;
            let target_value = ObjectId::from_bytes_or_panic(&data[pos..pos + hash_size]);
            pos += hash_size;
            RefRecordValue::Val2 { target, target_value }
        }
        3 => {
            // symref: target ref name
            let (target_len, n) = read_varint(&data[pos..])?;
            pos += n;
            let target_len = target_len as usize;
            if pos + target_len > data.len() {
                return Err(Error::UnexpectedEof);
            }
            let target = BString::from(&data[pos..pos + target_len]);
            pos += target_len;
            RefRecordValue::Symref { target }
        }
        _ => return Err(Error::InvalidRefValueType { value_type }),
    };

    Ok((
        RefRecord {
            name,
            update_index,
            value: record_value,
        },
        pos,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for value in [0u64, 1, 127, 128, 16383, 16384, u64::MAX / 2, u64::MAX] {
            let mut buf = Vec::new();
            write_varint(value, &mut buf);
            let (decoded, consumed) = read_varint(&buf).expect("decode should succeed");
            assert_eq!(decoded, value, "roundtrip failed for {value}");
            assert_eq!(consumed, buf.len());
        }
    }

    #[test]
    fn varint_single_byte_values() {
        // Values 0..=127 should encode to a single byte
        for value in 0..=127u64 {
            let mut buf = Vec::new();
            write_varint(value, &mut buf);
            assert_eq!(buf.len(), 1, "value {value} should be 1 byte");
            assert_eq!(buf[0], value as u8);
            let (decoded, consumed) = read_varint(&buf).expect("decode should succeed");
            assert_eq!(decoded, value);
            assert_eq!(consumed, 1);
        }
    }

    #[test]
    fn varint_msb_first_encoding() {
        // Value 128 in the reftable MSB-first varint with +1 adjustment:
        // encode: val=128, lowest 7 bits = 0x00, varint[9]=0x00
        //   val >>= 7 -> val=1, val != 0
        //   val -= 1 -> val=0, varint[8] = 0x80 | (0 & 0x7f) = 0x80
        //   val >>= 7 -> val=0, loop ends
        // Result: [0x80, 0x00]
        let mut buf = Vec::new();
        write_varint(128, &mut buf);
        assert_eq!(buf, vec![0x80, 0x00]);

        let (decoded, consumed) = read_varint(&buf).expect("decode should succeed");
        assert_eq!(decoded, 128);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn varint_300() {
        // Value 300 in MSB-first with +1 adjustment:
        // encode: val=300, lowest 7 bits = 300 & 0x7f = 44 = 0x2c, varint[9]=0x2c
        //   val >>= 7 -> val=2, val != 0
        //   val -= 1 -> val=1, varint[8] = 0x80 | (1 & 0x7f) = 0x81
        //   val >>= 7 -> val=0, loop ends
        // Result: [0x81, 0x2c]
        let mut buf = Vec::new();
        write_varint(300, &mut buf);
        assert_eq!(buf, vec![0x81, 0x2c]);

        let (decoded, consumed) = read_varint(&buf).expect("decode should succeed");
        assert_eq!(decoded, 300);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn varint_empty_input() {
        let result = read_varint(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn varint_two_byte_boundary() {
        // Values at the boundary between 1 and 2 bytes
        // 127 -> 1 byte, 128 -> 2 bytes
        let mut buf = Vec::new();
        write_varint(127, &mut buf);
        assert_eq!(buf.len(), 1);

        let mut buf = Vec::new();
        write_varint(128, &mut buf);
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn parse_valid_header_v1() {
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(MAGIC);
        data[4] = 1; // version 1
        put_be24((&mut data[5..8]).try_into().expect("3 bytes"), 4096); // block_size = 4096
        data[8..16].copy_from_slice(&1u64.to_be_bytes()); // min_update_index = 1
        data[16..24].copy_from_slice(&10u64.to_be_bytes()); // max_update_index = 10

        let header = parse_header(&data).expect("should parse");
        assert_eq!(header.version, Version::V1);
        assert_eq!(header.block_size, 4096);
        assert_eq!(header.min_update_index, 1);
        assert_eq!(header.max_update_index, 10);
    }

    #[test]
    fn header_roundtrip() {
        let header = Header {
            version: Version::V1,
            block_size: 4096,
            min_update_index: 1,
            max_update_index: 10,
        };
        let bytes = serialize_header(&header);
        assert_eq!(bytes.len(), HEADER_SIZE_V1);
        let parsed = parse_header(&bytes).expect("should parse");
        assert_eq!(parsed, header);
    }

    #[test]
    fn header_roundtrip_v2() {
        let header = Header {
            version: Version::V2,
            block_size: 65536,
            min_update_index: 100,
            max_update_index: 200,
        };
        let bytes = serialize_header(&header);
        assert_eq!(bytes.len(), HEADER_SIZE_V2);
        let parsed = parse_header(&bytes).expect("should parse");
        assert_eq!(parsed, header);
    }

    #[test]
    fn parse_header_invalid_magic() {
        let data = vec![0u8; 24];
        assert!(matches!(parse_header(&data), Err(Error::InvalidMagic)));
    }

    #[test]
    fn parse_header_unsupported_version() {
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(MAGIC);
        data[4] = 99;
        assert!(matches!(
            parse_header(&data),
            Err(Error::UnsupportedVersion { version: 99 })
        ));
    }

    #[test]
    fn parse_header_too_short() {
        let data = vec![0u8; 10];
        assert!(matches!(parse_header(&data), Err(Error::UnexpectedEof)));
    }

    #[test]
    fn footer_roundtrip() {
        let footer = Footer {
            header: Header {
                version: Version::V1,
                block_size: 4096,
                min_update_index: 1,
                max_update_index: 10,
            },
            ref_index_offset: 0,
            obj_offset: 1024,
            obj_id_len: 4,
            obj_index_offset: 0,
            log_offset: 2048,
            log_index_offset: 0,
        };
        let bytes = serialize_footer(&footer);
        assert_eq!(bytes.len(), FOOTER_SIZE_V1);
        let parsed = parse_footer(&bytes).expect("should parse");
        assert_eq!(parsed, footer);
    }

    #[test]
    fn footer_crc_mismatch() {
        let footer = Footer {
            header: Header {
                version: Version::V1,
                block_size: 4096,
                min_update_index: 1,
                max_update_index: 10,
            },
            ref_index_offset: 0,
            obj_offset: 0,
            obj_id_len: 0,
            obj_index_offset: 0,
            log_offset: 0,
            log_index_offset: 0,
        };
        let mut bytes = serialize_footer(&footer);
        // Corrupt the CRC
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        assert!(matches!(parse_footer(&bytes), Err(Error::Crc32Mismatch { .. })));
    }

    #[test]
    fn block_type_from_byte() {
        assert_eq!(BlockType::from_byte(b'r'), Some(BlockType::Ref));
        assert_eq!(BlockType::from_byte(b'g'), Some(BlockType::Log));
        assert_eq!(BlockType::from_byte(b'o'), Some(BlockType::Obj));
        assert_eq!(BlockType::from_byte(b'i'), Some(BlockType::Index));
        assert_eq!(BlockType::from_byte(b'x'), None);
    }

    #[test]
    fn ref_record_name_accessor() {
        let rec = RefRecord {
            name: "refs/heads/main".into(),
            update_index: 1,
            value: RefRecordValue::Val1 {
                target: ObjectId::null(gix_hash::Kind::Sha1),
            },
        };
        assert_eq!(rec.name(), b"refs/heads/main");
        assert_eq!(rec.update_index(), 1);
    }

    #[test]
    fn parse_ref_deletion_record() {
        let mut data = Vec::new();
        write_varint(0, &mut data); // prefix_len
        let suffix_and_type: u64 = (15 << 3) | 0; // suffix_len=15, type=0 (deletion)
        write_varint(suffix_and_type, &mut data);
        data.extend_from_slice(b"refs/heads/main"); // suffix
        write_varint(0, &mut data); // update_index_delta

        let (record, consumed) = parse_ref_record(&data, &[], 20, 1).expect("should parse");
        assert_eq!(consumed, data.len());
        assert_eq!(record.name, "refs/heads/main");
        assert_eq!(record.update_index, 1);
        assert!(matches!(record.value, RefRecordValue::Deletion));
    }

    #[test]
    fn parse_ref_val1_record() {
        let hash_size = 20;
        let oid_bytes = [0xAAu8; 20];
        let oid = ObjectId::from_bytes_or_panic(&oid_bytes);

        let mut data = Vec::new();
        write_varint(0, &mut data); // prefix_len
        let suffix_and_type: u64 = (15 << 3) | 1; // suffix_len=15, type=1 (val1)
        write_varint(suffix_and_type, &mut data);
        data.extend_from_slice(b"refs/heads/main"); // suffix
        write_varint(5, &mut data); // update_index_delta = 5
        data.extend_from_slice(&oid_bytes); // OID

        let (record, consumed) = parse_ref_record(&data, &[], hash_size, 10).expect("should parse");
        assert_eq!(consumed, data.len());
        assert_eq!(record.name, "refs/heads/main");
        assert_eq!(record.update_index, 15); // 10 + 5
        match record.value {
            RefRecordValue::Val1 { target } => {
                assert_eq!(target, oid);
            }
            _ => panic!("expected Val1 record"),
        }
    }

    #[test]
    fn parse_ref_symref_record() {
        let mut data = Vec::new();
        write_varint(0, &mut data); // prefix_len
        let suffix_and_type: u64 = (4 << 3) | 3; // suffix_len=4, type=3 (symref)
        write_varint(suffix_and_type, &mut data);
        data.extend_from_slice(b"HEAD"); // suffix = ref name
        write_varint(0, &mut data); // update_index_delta
        write_varint(15, &mut data); // target ref name length
        data.extend_from_slice(b"refs/heads/main"); // target ref name

        let (record, _consumed) = parse_ref_record(&data, &[], 20, 1).expect("should parse");
        assert_eq!(record.name, "HEAD");
        assert_eq!(record.update_index, 1);
        match record.value {
            RefRecordValue::Symref { target } => {
                assert_eq!(target, "refs/heads/main");
            }
            _ => panic!("expected Symref record"),
        }
    }

    #[test]
    fn parse_ref_val2_record() {
        let hash_size = 20;
        let oid1_bytes = [0xAAu8; 20];
        let oid2_bytes = [0xBBu8; 20];

        let mut data = Vec::new();
        write_varint(0, &mut data); // prefix_len
        let suffix_and_type: u64 = (12 << 3) | 2; // suffix_len=12, type=2 (val2)
        write_varint(suffix_and_type, &mut data);
        data.extend_from_slice(b"refs/tags/v1"); // suffix
        write_varint(0, &mut data); // update_index_delta
        data.extend_from_slice(&oid1_bytes);
        data.extend_from_slice(&oid2_bytes);

        let (record, _consumed) = parse_ref_record(&data, &[], hash_size, 1).expect("should parse");
        assert_eq!(record.name, "refs/tags/v1");
        assert_eq!(record.update_index, 1);
        match record.value {
            RefRecordValue::Val2 { target, target_value } => {
                assert_eq!(target, ObjectId::from_bytes_or_panic(&oid1_bytes));
                assert_eq!(target_value, ObjectId::from_bytes_or_panic(&oid2_bytes));
            }
            _ => panic!("expected Val2 record"),
        }
    }

    #[test]
    fn parse_ref_record_with_prefix_compression() {
        let hash_size = 20;
        let oid_bytes = [0xCCu8; 20];

        // Previous record name: "refs/heads/main"
        let prev_name = b"refs/heads/main";

        // New record: "refs/heads/feature" shares "refs/heads/" (11 bytes prefix)
        let suffix = b"feature";
        let mut data = Vec::new();
        write_varint(11, &mut data); // prefix_len = 11 ("refs/heads/")
        let suffix_and_type: u64 = (suffix.len() as u64) << 3 | 1; // type=1 (val1)
        write_varint(suffix_and_type, &mut data);
        data.extend_from_slice(suffix);
        write_varint(0, &mut data); // update_index_delta
        data.extend_from_slice(&oid_bytes);

        let (record, _consumed) = parse_ref_record(&data, prev_name, hash_size, 1).expect("should parse");
        assert_eq!(record.name, "refs/heads/feature");
        assert_eq!(record.update_index, 1);
        match record.value {
            RefRecordValue::Val1 { target } => {
                assert_eq!(target, ObjectId::from_bytes_or_panic(&oid_bytes));
            }
            _ => panic!("expected Val1 record"),
        }
    }

    #[test]
    fn be24_roundtrip() {
        let mut buf = [0u8; 3];
        for val in [0u32, 1, 255, 256, 4096, 65535, 0xFF_FFFF] {
            put_be24(&mut buf, val);
            assert_eq!(get_be24(&buf), val, "BE24 roundtrip failed for {val}");
        }
    }
}
