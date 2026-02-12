//! Property-based tests for CommitRef parse -> write roundtrip.
//!
//! Generates valid commit byte sequences and verifies that parsing then writing
//! produces the exact same bytes.

use gix_object::{CommitRef, WriteTo};
use proptest::prelude::*;

/// Generate a valid 40-character lowercase hex string (SHA1).
fn arb_hex_sha1() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::sample::select(b"0123456789abcdef".iter().copied().collect::<Vec<u8>>()),
        40..=40,
    )
    .prop_map(|bytes| String::from_utf8(bytes).expect("hex is ASCII"))
}

/// Generate a valid signature line like "Name <email> 1234567890 +0100".
fn arb_signature() -> impl Strategy<Value = String> {
    (
        "[A-Za-z ]{1,20}",                     // name
        "[a-z]{1,10}@[a-z]{1,8}\\.[a-z]{2,3}", // email
        0i64..2_000_000_000i64,                // seconds
        0u32..100u32,                          // hours
        0u32..60u32,                           // minutes
        proptest::bool::ANY,                   // positive offset?
    )
        .prop_map(|(name, email, secs, hours, mins, pos)| {
            // When hours and minutes are both zero, the sign is normalized to '+' by
            // `into_owned()`, so always use '+' for zero offsets to ensure roundtrip fidelity.
            let sign = if pos || (hours == 0 && mins == 0) { '+' } else { '-' };
            format!("{name} <{email}> {secs} {sign}{hours:02}{mins:02}")
        })
}

/// Generate valid commit bytes.
fn arb_commit_bytes() -> impl Strategy<Value = Vec<u8>> {
    (
        arb_hex_sha1(),                                   // tree
        proptest::collection::vec(arb_hex_sha1(), 0..=3), // parents
        arb_signature(),                                  // author
        arb_signature(),                                  // committer
        "[A-Za-z0-9 .!?\\-_,;:()\\n]{0,200}",             // message
    )
        .prop_map(|(tree, parents, author, committer, message)| {
            let mut buf = Vec::new();
            buf.extend_from_slice(format!("tree {tree}\n").as_bytes());
            for parent in &parents {
                buf.extend_from_slice(format!("parent {parent}\n").as_bytes());
            }
            buf.extend_from_slice(format!("author {author}\n").as_bytes());
            buf.extend_from_slice(format!("committer {committer}\n").as_bytes());
            buf.extend_from_slice(b"\n");
            buf.extend_from_slice(message.as_bytes());
            buf
        })
}

proptest! {
    /// Parse a valid commit, write it back, and verify byte-for-byte equality.
    #[test]
    fn commit_parse_write_roundtrip(commit_bytes in arb_commit_bytes()) {
        let parsed = CommitRef::from_bytes(&commit_bytes)
            .map_err(|e| proptest::test_runner::TestCaseError::fail(
                format!("parse failed: {}", e)
            ))?;

        let mut written = Vec::new();
        parsed.write_to(&mut written)
            .map_err(|e| proptest::test_runner::TestCaseError::fail(
                format!("write failed: {}", e)
            ))?;

        prop_assert_eq!(
            &written, &commit_bytes,
            "roundtrip mismatch:\n  input:  {:?}\n  output: {:?}",
            String::from_utf8_lossy(&commit_bytes),
            String::from_utf8_lossy(&written)
        );
    }

    /// Parsing and converting to owned, then writing the owned form, must also roundtrip.
    #[test]
    fn commit_owned_roundtrip(commit_bytes in arb_commit_bytes()) {
        let parsed = CommitRef::from_bytes(&commit_bytes)
            .map_err(|e| proptest::test_runner::TestCaseError::fail(
                format!("parse failed: {}", e)
            ))?;

        let owned = parsed.into_owned()
            .map_err(|e| proptest::test_runner::TestCaseError::fail(
                format!("into_owned failed: {}", e)
            ))?;

        let mut written = Vec::new();
        owned.write_to(&mut written)
            .map_err(|e| proptest::test_runner::TestCaseError::fail(
                format!("write failed: {}", e)
            ))?;

        prop_assert_eq!(
            &written, &commit_bytes,
            "owned roundtrip mismatch"
        );
    }
}
