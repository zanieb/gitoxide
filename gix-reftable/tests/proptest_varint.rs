//! Property-based tests for reftable varint encode/decode roundtrip.
//!
//! The reftable varint uses MSB-first encoding with a +1 adjustment on
//! continuation bytes. These tests verify correctness across the full u64 range.

use gix_reftable::{read_varint, write_varint};
use proptest::prelude::*;

proptest! {
    /// Every u64 value must survive a write_varint -> read_varint roundtrip.
    #[test]
    fn varint_roundtrip(value: u64) {
        let mut buf = Vec::new();
        write_varint(value, &mut buf);

        // Must produce at least one byte
        prop_assert!(!buf.is_empty(), "write_varint produced empty output for {}", value);

        // At most 10 bytes for a u64
        prop_assert!(buf.len() <= 10, "varint encoding used {} bytes for {}", buf.len(), value);

        let (decoded, consumed) = read_varint(&buf).expect("read_varint should succeed");
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded, value);
    }

    /// Varints must use a canonical (minimal) encoding: no leading zero-bytes
    /// beyond what's needed for the value.
    #[test]
    fn varint_encoding_is_canonical(value: u64) {
        let mut buf = Vec::new();
        write_varint(value, &mut buf);

        // For values that fit in one byte (0..=127), encoding must be exactly 1 byte
        if value <= 127 {
            prop_assert_eq!(buf.len(), 1);
            prop_assert_eq!(buf[0] & 0x80, 0);
        }
    }

    /// If we have trailing garbage after a valid varint, the decode must
    /// still consume only the varint bytes and return the correct value.
    #[test]
    fn varint_ignores_trailing_data(value: u64, extra in proptest::collection::vec(any::<u8>(), 0..16)) {
        let mut buf = Vec::new();
        write_varint(value, &mut buf);
        let varint_len = buf.len();
        buf.extend_from_slice(&extra);

        let (decoded, consumed) = read_varint(&buf).expect("read_varint should succeed");
        prop_assert_eq!(consumed, varint_len);
        prop_assert_eq!(decoded, value);
    }
}
