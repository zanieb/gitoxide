//! Property-based tests for ObjectId hex encode/decode roundtrip.

use gix_hash::ObjectId;
use proptest::prelude::*;

/// Generate arbitrary 20-byte SHA1 digests.
fn arb_sha1_bytes() -> impl Strategy<Value = [u8; 20]> {
    proptest::array::uniform20(any::<u8>())
}

proptest! {
    /// ObjectId -> hex -> ObjectId must roundtrip for any SHA1 digest.
    #[test]
    fn sha1_hex_roundtrip(bytes in arb_sha1_bytes()) {
        let id = ObjectId::from_bytes_or_panic(&bytes);

        // Encode to hex string
        let hex = id.to_hex().to_string();

        // Must be exactly 40 hex characters
        prop_assert_eq!(hex.len(), 40, "SHA1 hex must be 40 chars");

        // All characters must be lowercase hex
        for ch in hex.chars() {
            prop_assert!(
                ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase(),
                "hex char must be lowercase hex digit, got {ch}"
            );
        }

        // Decode back
        let decoded = ObjectId::from_hex(hex.as_bytes())
            .expect("decoding valid hex should succeed");
        prop_assert_eq!(id, decoded, "hex roundtrip mismatch");
    }

    /// ObjectId -> raw bytes -> ObjectId must roundtrip.
    #[test]
    fn sha1_bytes_roundtrip(bytes in arb_sha1_bytes()) {
        let id = ObjectId::from_bytes_or_panic(&bytes);
        let raw = id.as_slice();
        let reconstructed = ObjectId::from_bytes_or_panic(raw);
        prop_assert_eq!(id, reconstructed, "bytes roundtrip mismatch");
    }

    /// from_hex must reject arbitrary non-hex strings of wrong length.
    #[test]
    fn from_hex_rejects_wrong_length(len in 0usize..100, fill in any::<u8>()) {
        // Skip lengths that happen to be valid (40 for SHA1, 64 for SHA256)
        if len == 40 || len == 64 {
            return Ok(());
        }
        let hex_char = if fill % 2 == 0 { b'a' } else { b'0' };
        let input = vec![hex_char; len];
        prop_assert!(ObjectId::from_hex(&input).is_err(), "should reject length {len}");
    }
}
