//! Tests ported from C Git's `t/unit-tests/u-reftable-record.c`.
//!
//! These tests validate varint encoding/decoding, key encoding/decoding,
//! and record (ref/log/obj/index) encode/decode roundtrips against the
//! C Git reftable specification.
//!
//! The correct varint encoding is MSB-first with a +1 adjustment on
//! continuation bytes (NOT LEB128). Tests that depend on this encoding
//! will FAIL with the current LEB128 implementation until the varint
//! code is rewritten.

use gix_reftable::{read_varint, write_varint, RefRecord};

// ---------------------------------------------------------------------------
// Varint tests
// ---------------------------------------------------------------------------

/// Port of `test_reftable_record__varint_roundtrip` from C Git.
///
/// Tests that encode(decode(x)) == x for a set of representative values.
/// This test validates MSB-first varint encoding per C Git spec.
/// It will FAIL with the current LEB128 implementation for multi-byte values.
#[test]
fn varint_roundtrip() {
    let inputs: &[u64] = &[
        0,
        1,
        27,
        127,
        128,
        257,
        4096,
        1u64 << 63,
        u64::MAX, // (1<<63) + (1<<63) - 1 = u64::MAX
    ];

    for &input in inputs {
        let mut buf = Vec::new();
        write_varint(input, &mut buf);
        assert!(
            !buf.is_empty(),
            "write_varint should produce at least one byte for value {input}"
        );
        let (got, consumed) = read_varint(&buf).expect("read_varint should succeed");
        assert_eq!(
            consumed,
            buf.len(),
            "consumed bytes should equal buffer length for value {input}"
        );
        assert_eq!(got, input, "roundtrip failed for value {input}");
    }
}

/// Verify specific byte-level encoding for the MSB-first varint format.
///
/// These test vectors are computed from the C Git `put_var_int` algorithm:
///   - Build from LSB to MSB, each continuation byte has `--value` adjustment
///   - MSB byte does NOT have the 0x80 continuation bit
///
/// This test will FAIL with the current LEB128 implementation.
#[test]
fn varint_known_encodings() {
    // Test vectors computed from C Git's put_var_int:
    // 0 -> [0x00]
    let check = |value: u64, expected: &[u8]| {
        let mut buf = Vec::new();
        write_varint(value, &mut buf);
        assert_eq!(
            buf, expected,
            "encoding of {value} should be {expected:02x?}, got {buf:02x?}"
        );
        let (decoded, consumed) = read_varint(&buf).expect("decode should succeed");
        assert_eq!(consumed, expected.len());
        assert_eq!(decoded, value, "decode of {expected:02x?} should be {value}");
    };

    check(0, &[0x00]);
    check(1, &[0x01]);
    check(127, &[0x7f]);
    // 128: varint[9]=0x00, value>>=7->1, --value=0, varint[8]=0x80. Result: [0x80, 0x00]
    check(128, &[0x80, 0x00]);
    // 16383: varint[9]=0x7f, value>>=7->127, --126, varint[8]=0x80|0x7e=0xfe. Result: [0xfe, 0x7f]
    check(16383, &[0xfe, 0x7f]);
    // 16384: varint[9]=0x00, value>>=7->128, --127, varint[8]=0x80|0x7f=0xff. Result: [0xff, 0x00]
    check(16384, &[0xff, 0x00]);
}

/// Port of `test_reftable_record__varint_overflow` from C Git.
///
/// A buffer of all 0xFF continuation bytes followed by 0x00 should overflow.
#[test]
fn varint_overflow() {
    let buf: [u8; 10] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    let result = read_varint(&buf);
    assert!(result.is_err(), "decoding an overflowing varint should return an error");
}

/// Decoding from an empty buffer should fail.
#[test]
fn varint_empty_input() {
    let result = read_varint(&[]);
    assert!(result.is_err(), "empty input should fail");
}

/// Single-byte values (0..=127) should encode to exactly one byte.
#[test]
fn varint_single_byte_range() {
    for value in 0..=127u64 {
        let mut buf = Vec::new();
        write_varint(value, &mut buf);
        assert_eq!(buf.len(), 1, "value {value} should encode to a single byte");
        assert_eq!(buf[0], value as u8);
    }
}

// ---------------------------------------------------------------------------
// Key encoding/decoding tests
// ---------------------------------------------------------------------------

/// Port of `test_reftable_record__key_roundtrip` from C Git.
///
/// Tests prefix-compressed key encoding: given a previous key and a new key,
/// encode the new key with prefix compression, then decode it back.
///
/// The key encoding format is:
///   prefix_len (varint) + (suffix_len << 3 | extra) (varint) + suffix bytes
///
/// Since the Rust API handles key encoding internally within record parsing,
/// we test this indirectly through ref record roundtrips with prefix compression.
#[test]
fn key_encoding_through_ref_record_prefix_compression() {
    use bstr::BString;
    use gix_hash::ObjectId;

    let hash_size = 20;
    let oid = ObjectId::from_bytes_or_panic(&[0xAA; 20]);

    // First record: "refs/heads/master" (no prefix compression)
    let first = RefRecord {
        name: BString::from("refs/heads/master"),
        update_index: 1,
        value: gix_reftable::RefRecordValue::Val1 { target: oid },
    };

    // Serialize the first record with no previous name
    let first_bytes = gix_reftable::write::serialize_ref_record(&first, &[], 1, hash_size);

    // Parse it back
    let (parsed_first, _) =
        gix_reftable::parse_ref_record(&first_bytes, &[], hash_size, 1).expect("first record should parse");
    assert_eq!(parsed_first.name(), b"refs/heads/master");

    // Second record: "refs/tags/bla" shares prefix "refs/" (5 bytes)
    let second = RefRecord {
        name: BString::from("refs/tags/bla"),
        update_index: 1,
        value: gix_reftable::RefRecordValue::Val1 { target: oid },
    };

    let second_bytes = gix_reftable::write::serialize_ref_record(&second, b"refs/heads/master", 1, hash_size);

    // Parse with the previous key as prefix context
    let (parsed_second, consumed) = gix_reftable::parse_ref_record(&second_bytes, b"refs/heads/master", hash_size, 1)
        .expect("second record should parse with prefix");
    assert_eq!(parsed_second.name(), b"refs/tags/bla");
    assert_eq!(consumed, second_bytes.len());
}

// ---------------------------------------------------------------------------
// Ref record roundtrip tests
// ---------------------------------------------------------------------------

/// Port of `test_reftable_record__ref_record_roundtrip` from C Git.
///
/// Tests encode/decode roundtrip for all ref record value types:
/// - Deletion (value_type=0)
/// - Val1 (value_type=1): single OID
/// - Val2 (value_type=2): OID + target OID
/// - Symref (value_type=3): symbolic reference target
#[test]
fn ref_record_roundtrip_all_types() {
    use bstr::BString;
    use gix_hash::ObjectId;

    let hash_size = 20;
    let min_update_index = 1u64;

    // Helper to create a deterministic hash from an integer
    fn make_hash(j: u8) -> [u8; 20] {
        let mut h = [0u8; 20];
        for i in 0..20 {
            h[i] = (j >> (i % 8)) & 0xff;
        }
        h
    }

    // Deletion
    {
        let record = RefRecord {
            name: BString::from("refs/heads/master"),
            update_index: min_update_index,
            value: gix_reftable::RefRecordValue::Deletion,
        };
        let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);
        let (parsed, consumed) = gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index)
            .expect("deletion record should parse");
        assert_eq!(consumed, bytes.len());
        assert_eq!(parsed.name(), b"refs/heads/master");
        assert!(
            matches!(parsed.value, gix_reftable::RefRecordValue::Deletion),
            "expected Deletion variant"
        );
    }

    // Val1 (single OID)
    {
        let oid_bytes = make_hash(1);
        let record = RefRecord {
            name: BString::from("refs/heads/master"),
            update_index: min_update_index,
            value: gix_reftable::RefRecordValue::Val1 {
                target: ObjectId::from_bytes_or_panic(&oid_bytes),
            },
        };
        let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);
        let (parsed, consumed) =
            gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index).expect("val1 record should parse");
        assert_eq!(consumed, bytes.len());
        match &parsed {
            RefRecord {
                name,
                update_index,
                value: gix_reftable::RefRecordValue::Val1 { target },
                ..
            } => {
                assert_eq!(&name[..], &b"refs/heads/master"[..]);
                assert_eq!(target.as_bytes(), &oid_bytes);
                assert_eq!(*update_index, min_update_index);
            }
            other => panic!("expected Val1, got {other:?}"),
        }
    }

    // Val2 (OID + target OID)
    {
        let oid1_bytes = make_hash(1);
        let oid2_bytes = make_hash(2);
        let record = RefRecord {
            name: BString::from("refs/heads/master"),
            update_index: min_update_index,
            value: gix_reftable::RefRecordValue::Val2 {
                target: ObjectId::from_bytes_or_panic(&oid1_bytes),
                target_value: ObjectId::from_bytes_or_panic(&oid2_bytes),
            },
        };
        let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);
        let (parsed, consumed) =
            gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index).expect("val2 record should parse");
        assert_eq!(consumed, bytes.len());
        match &parsed {
            RefRecord {
                name,
                update_index,
                value: gix_reftable::RefRecordValue::Val2 { target, target_value },
                ..
            } => {
                assert_eq!(&name[..], &b"refs/heads/master"[..]);
                assert_eq!(target.as_bytes(), &oid1_bytes);
                assert_eq!(target_value.as_bytes(), &oid2_bytes);
                assert_eq!(*update_index, min_update_index);
            }
            other => panic!("expected Val2, got {other:?}"),
        }
    }

    // Symref
    {
        let record = RefRecord {
            name: BString::from("refs/heads/master"),
            update_index: min_update_index,
            value: gix_reftable::RefRecordValue::Symref {
                target: BString::from("target"),
            },
        };
        let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);
        let (parsed, consumed) = gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index)
            .expect("symref record should parse");
        assert_eq!(consumed, bytes.len());
        match &parsed {
            RefRecord {
                name,
                update_index,
                value: gix_reftable::RefRecordValue::Symref { target },
                ..
            } => {
                assert_eq!(&name[..], &b"refs/heads/master"[..]);
                assert_eq!(&target[..], &b"target"[..]);
                assert_eq!(*update_index, min_update_index);
            }
            other => panic!("expected Symref, got {other:?}"),
        }
    }
}

/// Port of `test_reftable_record__ref_record_comparison` from C Git.
///
/// Tests that ref records with the same name compare as equal (by key),
/// and records with different names compare correctly.
#[test]
fn ref_record_comparison() {
    use bstr::BString;
    use gix_hash::ObjectId;

    // Two records with same name but different value types
    let rec_val1 = RefRecord {
        name: BString::from("refs/heads/master"),
        update_index: 0,
        value: gix_reftable::RefRecordValue::Val1 {
            target: ObjectId::null(gix_hash::Kind::Sha1),
        },
    };
    let rec_deletion = RefRecord {
        name: BString::from("refs/heads/master"),
        update_index: 0,
        value: gix_reftable::RefRecordValue::Deletion,
    };
    let rec_head = RefRecord {
        name: BString::from("HEAD"),
        update_index: 0,
        value: gix_reftable::RefRecordValue::Symref {
            target: BString::from("refs/heads/master"),
        },
    };

    // Same name -> records are NOT equal (different value types) but keys match
    assert_ne!(rec_val1, rec_deletion, "different value types should not be equal");
    // Key comparison: same name
    assert_eq!(rec_val1.name(), rec_deletion.name());

    // Different names: "refs/heads/master" > "HEAD"
    assert!(rec_deletion.name() > rec_head.name());
}

/// Port of `test_reftable_record__ref_record_compare_name` from C Git.
#[test]
fn ref_record_compare_name() {
    use bstr::BString;
    use gix_hash::ObjectId;

    let rec_a = RefRecord {
        name: BString::from("refs/heads/a"),
        update_index: 0,
        value: gix_reftable::RefRecordValue::Val1 {
            target: ObjectId::null(gix_hash::Kind::Sha1),
        },
    };
    let rec_b = RefRecord {
        name: BString::from("refs/heads/b"),
        update_index: 0,
        value: gix_reftable::RefRecordValue::Val1 {
            target: ObjectId::null(gix_hash::Kind::Sha1),
        },
    };
    let rec_a2 = RefRecord {
        name: BString::from("refs/heads/a"),
        update_index: 0,
        value: gix_reftable::RefRecordValue::Val1 {
            target: ObjectId::null(gix_hash::Kind::Sha1),
        },
    };

    assert!(rec_a.name() < rec_b.name());
    assert!(rec_b.name() > rec_a.name());
    assert_eq!(rec_a.name(), rec_a2.name());
}

// ---------------------------------------------------------------------------
// Log record tests
// ---------------------------------------------------------------------------
// Note: The current Rust API has LogRecord as a struct, not an enum with
// value types like C Git. These tests verify the existing LogRecord type.

/// Port of `test_reftable_record__log_record_comparison` from C Git.
///
/// Log record ordering: first by refname, then by update_index (REVERSED -
/// higher update_index sorts first within the same refname). This is because
/// log entries are stored newest-first.
#[test]
fn log_record_comparison() {
    use gix_reftable::LogRecord;

    let log1 = LogRecord {
        ref_name: "refs/heads/master".into(),
        update_index: 42,
        old_id: gix_hash::ObjectId::null(gix_hash::Kind::Sha1),
        new_id: gix_hash::ObjectId::null(gix_hash::Kind::Sha1),
        name: "".into(),
        email: "".into(),
        time: 0,
        tz_offset: 0,
        message: "".into(),
    };
    let log2 = LogRecord {
        ref_name: "refs/heads/master".into(),
        update_index: 22,
        ..log1.clone()
    };
    let log3 = LogRecord {
        ref_name: "refs/heads/main".into(),
        update_index: 22,
        ..log1.clone()
    };

    // Same refname, different update_index -> not equal
    assert_ne!(log1, log2);

    // Different refname -> not equal
    assert_ne!(log2, log3);

    // Key ordering: "refs/heads/master" > "refs/heads/main" (lexicographic)
    assert!(log2.ref_name > log3.ref_name);

    // Within same refname, log records are ordered by update_index descending
    // (higher update_index comes first). So log1 (42) < log2 (22) in the
    // reversed comparison used by reftable (C Git cmp returns < 0).
    // We verify the raw field values here since Rust doesn't have the
    // reversed comparison built in.
    assert!(log1.update_index > log2.update_index);

    // Make them equal
    let log2_eq = LogRecord {
        update_index: 42,
        ..log2.clone()
    };
    assert_eq!(log1.ref_name, log2_eq.ref_name);
    assert_eq!(log1.update_index, log2_eq.update_index);
}

/// Port of `test_reftable_record__log_record_compare_key` from C Git.
///
/// Log record key is: refname + inverted update_index (as big-endian u64).
/// So for the same refname, higher update_index sorts BEFORE lower.
#[test]
fn log_record_compare_key() {
    use gix_reftable::LogRecord;

    let base = LogRecord {
        ref_name: "".into(),
        update_index: 0,
        old_id: gix_hash::ObjectId::null(gix_hash::Kind::Sha1),
        new_id: gix_hash::ObjectId::null(gix_hash::Kind::Sha1),
        name: "".into(),
        email: "".into(),
        time: 0,
        tz_offset: 0,
        message: "".into(),
    };

    let log_a1 = LogRecord {
        ref_name: "refs/heads/a".into(),
        update_index: 1,
        ..base.clone()
    };
    let log_b2 = LogRecord {
        ref_name: "refs/heads/b".into(),
        update_index: 2,
        ..base.clone()
    };
    let log_a3 = LogRecord {
        ref_name: "refs/heads/a".into(),
        update_index: 3,
        ..base.clone()
    };

    // Different refnames: "a" < "b"
    assert!(log_a1.ref_name < log_b2.ref_name);
    assert!(log_b2.ref_name > log_a1.ref_name);

    // Same refname "refs/heads/a", update_index 3 vs 1:
    // In reftable, higher update_index sorts first (key uses inverted index).
    // So log_a3 (index=3) comes BEFORE log_a1 (index=1) in sort order.
    assert!(log_a3.update_index > log_a1.update_index);

    // With same update_index, same refname -> equal keys
    let log_a1_dup = LogRecord {
        ref_name: "refs/heads/a".into(),
        update_index: 1,
        ..base.clone()
    };
    assert_eq!(log_a1.ref_name, log_a1_dup.ref_name);
    assert_eq!(log_a1.update_index, log_a1_dup.update_index);
}

// ---------------------------------------------------------------------------
// Log record roundtrip tests
// ---------------------------------------------------------------------------

/// Port of `test_reftable_record__log_record_roundtrip` from C Git.
///
/// Tests encode/decode roundtrip for log records with:
/// - Full update log (name, email, message, old/new hash, time, tz_offset)
/// - Deletion log (no update data)
/// - Minimal update log (just hashes, no name/email/message)
///
/// NOTE: This test cannot be fully exercised until log record encode/decode
/// is implemented in the Rust code. For now it validates the LogRecord type
/// construction and field access.
#[test]
fn log_record_roundtrip_construction() {
    use gix_reftable::LogRecord;

    fn make_hash(j: u8) -> [u8; 20] {
        let mut h = [0u8; 20];
        for i in 0..20 {
            h[i] = (j >> (i % 8)) & 0xff;
        }
        h
    }

    // Full update log record (mirrors C Git test)
    let log_update = LogRecord {
        ref_name: "refs/heads/master".into(),
        update_index: 42,
        old_id: gix_hash::ObjectId::from_bytes_or_panic(&make_hash(2)),
        new_id: gix_hash::ObjectId::from_bytes_or_panic(&make_hash(1)),
        name: "han-wen".into(),
        email: "hanwen@google.com".into(),
        time: 1577123507,
        tz_offset: 100,
        message: "test".into(),
    };

    assert_eq!(&log_update.ref_name[..], &b"refs/heads/master"[..]);
    assert_eq!(log_update.update_index, 42);
    assert_eq!(&log_update.name[..], &b"han-wen"[..]);
    assert_eq!(&log_update.email[..], &b"hanwen@google.com"[..]);
    assert_eq!(log_update.time, 1577123507);
    assert_eq!(log_update.tz_offset, 100);
    assert_eq!(&log_update.message[..], &b"test"[..]);

    // Deletion log record (C Git: value_type = REFTABLE_LOG_DELETION)
    // In the Rust API, a deletion log would have null OIDs and empty fields.
    let log_deletion = LogRecord {
        ref_name: "refs/heads/master".into(),
        update_index: 22,
        old_id: gix_hash::ObjectId::null(gix_hash::Kind::Sha1),
        new_id: gix_hash::ObjectId::null(gix_hash::Kind::Sha1),
        name: "".into(),
        email: "".into(),
        time: 0,
        tz_offset: 0,
        message: "".into(),
    };

    assert_eq!(&log_deletion.ref_name[..], &b"refs/heads/master"[..]);
    assert_eq!(log_deletion.update_index, 22);

    // Minimal update (just hashes, no name/email/message - like C Git's third record)
    let log_minimal = LogRecord {
        ref_name: "branch".into(),
        update_index: 33,
        old_id: gix_hash::ObjectId::from_bytes_or_panic(&make_hash(4)),
        new_id: gix_hash::ObjectId::from_bytes_or_panic(&make_hash(3)),
        name: "".into(),
        email: "".into(),
        time: 0,
        tz_offset: 0,
        message: "".into(),
    };

    assert_eq!(&log_minimal.ref_name[..], &b"branch"[..]);
    assert_eq!(log_minimal.update_index, 33);
}

// ---------------------------------------------------------------------------
// Ref record update_index delta encoding test
// ---------------------------------------------------------------------------

/// Tests that update_index is stored as a delta from min_update_index.
///
/// When min_update_index=10 and a record has update_index=15, the encoded
/// delta should be 5. On decode, we should get back update_index=15.
#[test]
fn ref_record_update_index_delta() {
    use bstr::BString;
    use gix_hash::ObjectId;

    let hash_size = 20;
    let min_update_index = 10u64;

    let record = RefRecord {
        name: BString::from("refs/heads/main"),
        update_index: 15,
        value: gix_reftable::RefRecordValue::Val1 {
            target: ObjectId::from_bytes_or_panic(&[0xAA; 20]),
        },
    };

    let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);

    let (parsed, _) =
        gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index).expect("record should parse");

    assert_eq!(parsed.update_index(), 15);
}

/// Tests that update_index=0 relative to min works correctly.
#[test]
fn ref_record_update_index_delta_zero() {
    use bstr::BString;

    let hash_size = 20;
    let min_update_index = 42u64;

    let record = RefRecord {
        name: BString::from("refs/heads/old"),
        update_index: 42,
        value: gix_reftable::RefRecordValue::Deletion,
    };

    let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);

    let (parsed, _) =
        gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index).expect("record should parse");

    assert_eq!(parsed.update_index(), 42);
}

// ---------------------------------------------------------------------------
// Edge case tests
// ---------------------------------------------------------------------------

/// Varint encoding/decoding at the boundary of multi-byte transitions.
///
/// This test validates MSB-first varint encoding per C Git spec.
/// It will FAIL with the current LEB128 implementation.
#[test]
fn varint_boundary_values() {
    // Values right around the 1-byte / 2-byte boundary
    let boundary_values: &[u64] = &[
        126,     // 1 byte: [0x7e]
        127,     // 1 byte: [0x7f]
        128,     // 2 bytes: [0x80, 0x00]
        129,     // 2 bytes: [0x80, 0x01]
        16382,   // 2 bytes
        16383,   // 2 bytes: [0xfe, 0x7f]
        16384,   // 2 bytes: [0xff, 0x00]
        16385,   // 3 bytes
        2113663, // 3 bytes: max 3-byte value
        2113664, // 3 bytes or 4 bytes boundary
    ];

    for &value in boundary_values {
        let mut buf = Vec::new();
        write_varint(value, &mut buf);
        let (decoded, consumed) =
            read_varint(&buf).unwrap_or_else(|_| panic!("should decode varint for value {value}"));
        assert_eq!(consumed, buf.len(), "consumed mismatch for {value}");
        assert_eq!(decoded, value, "roundtrip failed for {value}");
    }
}

/// Ref record with very long name (stress test prefix compression).
#[test]
fn ref_record_long_name() {
    use bstr::BString;
    use gix_hash::ObjectId;

    let hash_size = 20;
    let min_update_index = 1u64;

    // Generate a long ref name
    let long_name: String = format!("refs/heads/{}", "a".repeat(500));
    let record = RefRecord {
        name: BString::from(long_name.as_str()),
        update_index: 1,
        value: gix_reftable::RefRecordValue::Val1 {
            target: ObjectId::from_bytes_or_panic(&[0xFF; 20]),
        },
    };

    let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);
    let (parsed, consumed) = gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index)
        .expect("long name record should parse");
    assert_eq!(consumed, bytes.len());
    assert_eq!(parsed.name(), long_name.as_bytes());
}

// ---------------------------------------------------------------------------
// Additional record tests ported from C Git
// ---------------------------------------------------------------------------

/// Port of varint truncated continuation from C Git concept.
///
/// A buffer with the continuation bit set (0x80) but no following byte
/// should fail with UnexpectedEof.
#[test]
fn varint_truncated_continuation() {
    // Single byte with continuation bit set, but no next byte
    let result = read_varint(&[0x80]);
    assert!(result.is_err(), "truncated continuation byte should fail");

    // Two continuation bytes, missing final byte
    let result = read_varint(&[0x80, 0x80]);
    assert!(result.is_err(), "double truncated continuation should fail");
}

/// Verify that all RefRecord variants return the correct update_index.
///
/// Port of the concept from C Git where all value types are exercised.
#[test]
fn ref_record_update_index_all_variants() {
    use bstr::BString;
    use gix_hash::ObjectId;

    let null = ObjectId::null(gix_hash::Kind::Sha1);

    let deletion = RefRecord {
        name: BString::from("refs/heads/old"),
        update_index: 42,
        value: gix_reftable::RefRecordValue::Deletion,
    };
    assert_eq!(deletion.update_index(), 42);

    let val1 = RefRecord {
        name: BString::from("refs/heads/main"),
        update_index: 100,
        value: gix_reftable::RefRecordValue::Val1 { target: null },
    };
    assert_eq!(val1.update_index(), 100);

    let val2 = RefRecord {
        name: BString::from("refs/tags/v1"),
        update_index: 200,
        value: gix_reftable::RefRecordValue::Val2 {
            target: null,
            target_value: null,
        },
    };
    assert_eq!(val2.update_index(), 200);

    let symref = RefRecord {
        name: BString::from("HEAD"),
        update_index: 300,
        value: gix_reftable::RefRecordValue::Symref {
            target: BString::from("refs/heads/main"),
        },
    };
    assert_eq!(symref.update_index(), 300);
}

/// Port of `reftable_record_is_deletion` concept from C Git.
///
/// Only the Deletion variant represents a deletion; all others do not.
#[test]
fn ref_record_is_deletion_check() {
    use bstr::BString;
    use gix_hash::ObjectId;

    let null = ObjectId::null(gix_hash::Kind::Sha1);

    let deletion = RefRecord {
        name: BString::from("refs/heads/old"),
        update_index: 1,
        value: gix_reftable::RefRecordValue::Deletion,
    };
    assert!(matches!(deletion.value, gix_reftable::RefRecordValue::Deletion));

    let val1 = RefRecord {
        name: BString::from("refs/heads/main"),
        update_index: 1,
        value: gix_reftable::RefRecordValue::Val1 { target: null },
    };
    assert!(!matches!(val1.value, gix_reftable::RefRecordValue::Deletion));

    let symref = RefRecord {
        name: BString::from("HEAD"),
        update_index: 1,
        value: gix_reftable::RefRecordValue::Symref {
            target: BString::from("refs/heads/main"),
        },
    };
    assert!(!matches!(symref.value, gix_reftable::RefRecordValue::Deletion));
}

/// Port of `test_reftable_record__ref_record_roundtrip` multi-variant concept.
///
/// Tests that every ref record value type roundtrips correctly through
/// serialize -> parse with a non-zero min_update_index.
#[test]
fn ref_record_roundtrip_all_types_nonzero_min() {
    use bstr::BString;
    use gix_hash::ObjectId;

    let hash_size = 20;
    let min_update_index = 100u64;

    fn make_hash(j: u8) -> [u8; 20] {
        let mut h = [0u8; 20];
        for i in 0..20 {
            h[i] = (j >> (i % 8)) & 0xff;
        }
        h
    }

    // Deletion with update_index > min
    {
        let record = RefRecord {
            name: BString::from("refs/heads/deleted"),
            update_index: 105,
            value: gix_reftable::RefRecordValue::Deletion,
        };
        let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);
        let (parsed, consumed) =
            gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index).expect("should parse");
        assert_eq!(consumed, bytes.len());
        assert!(matches!(parsed.value, gix_reftable::RefRecordValue::Deletion));
        assert_eq!(parsed.update_index(), 105);
    }

    // Val1
    {
        let record = RefRecord {
            name: BString::from("refs/heads/main"),
            update_index: 150,
            value: gix_reftable::RefRecordValue::Val1 {
                target: ObjectId::from_bytes_or_panic(&make_hash(10)),
            },
        };
        let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);
        let (parsed, _) =
            gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index).expect("should parse");
        assert_eq!(parsed.update_index(), 150);
        assert_eq!(parsed.name(), b"refs/heads/main");
    }

    // Val2
    {
        let record = RefRecord {
            name: BString::from("refs/tags/v1.0"),
            update_index: 200,
            value: gix_reftable::RefRecordValue::Val2 {
                target: ObjectId::from_bytes_or_panic(&make_hash(1)),
                target_value: ObjectId::from_bytes_or_panic(&make_hash(2)),
            },
        };
        let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);
        let (parsed, _) =
            gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index).expect("should parse");
        assert_eq!(parsed.update_index(), 200);
        match &parsed.value {
            gix_reftable::RefRecordValue::Val2 { target, target_value } => {
                assert_eq!(target.as_bytes(), &make_hash(1));
                assert_eq!(target_value.as_bytes(), &make_hash(2));
            }
            other => panic!("expected Val2, got {other:?}"),
        }
    }

    // Symref
    {
        let record = RefRecord {
            name: BString::from("HEAD"),
            update_index: 100, // exactly min
            value: gix_reftable::RefRecordValue::Symref {
                target: BString::from("refs/heads/main"),
            },
        };
        let bytes = gix_reftable::write::serialize_ref_record(&record, &[], min_update_index, hash_size);
        let (parsed, _) =
            gix_reftable::parse_ref_record(&bytes, &[], hash_size, min_update_index).expect("should parse");
        assert_eq!(parsed.update_index(), 100);
        match &parsed {
            RefRecord {
                value: gix_reftable::RefRecordValue::Symref { target },
                ..
            } => {
                assert_eq!(&target[..], b"refs/heads/main");
            }
            other => panic!("expected Symref, got {other:?}"),
        }
    }
}

/// Test parse_ref_record with an invalid value_type (4, 5, 6, 7).
///
/// Port of error handling concept: value_type must be 0-3.
#[test]
fn ref_record_invalid_value_type() {
    use gix_reftable::write_varint;

    for bad_type in 4u8..=7 {
        let mut data = Vec::new();
        write_varint(0, &mut data); // prefix_len = 0
        let suffix_and_type: u64 = (4 << 3) | (bad_type as u64); // suffix_len=4, bad type
        write_varint(suffix_and_type, &mut data);
        data.extend_from_slice(b"refs"); // suffix
        write_varint(0, &mut data); // update_index_delta

        let result = gix_reftable::parse_ref_record(&data, &[], 20, 1);
        assert!(result.is_err(), "value_type {bad_type} should fail parsing");
    }
}

/// Test parse_ref_record with truncated data (suffix extends past end).
#[test]
fn ref_record_truncated_suffix() {
    use gix_reftable::write_varint;

    let mut data = Vec::new();
    write_varint(0, &mut data); // prefix_len = 0
                                // Claim suffix_len=100 but only provide 4 bytes
    let suffix_and_type: u64 = (100 << 3) | 1;
    write_varint(suffix_and_type, &mut data);
    data.extend_from_slice(b"refs"); // only 4 bytes, not 100

    let result = gix_reftable::parse_ref_record(&data, &[], 20, 1);
    assert!(result.is_err(), "truncated suffix should fail");
}

/// Test parse_ref_record with truncated OID data (Val1 without enough hash bytes).
#[test]
fn ref_record_truncated_oid() {
    use gix_reftable::write_varint;

    let mut data = Vec::new();
    write_varint(0, &mut data); // prefix_len
    let suffix_and_type: u64 = (1 << 3) | 1; // suffix_len=1, type=val1
    write_varint(suffix_and_type, &mut data);
    data.push(b'a'); // 1-byte suffix
    write_varint(0, &mut data); // update_index_delta
                                // No OID bytes follow - should fail

    let result = gix_reftable::parse_ref_record(&data, &[], 20, 1);
    assert!(result.is_err(), "missing OID bytes should fail");
}

/// Test parse_ref_record where prefix_len exceeds the provided prefix.
#[test]
fn ref_record_prefix_len_exceeds_prefix() {
    use gix_reftable::write_varint;

    let mut data = Vec::new();
    write_varint(20, &mut data); // prefix_len = 20, but prefix is only 5 bytes
    let suffix_and_type: u64 = (3 << 3) | 0; // suffix_len=3, type=deletion
    write_varint(suffix_and_type, &mut data);
    data.extend_from_slice(b"abc"); // suffix
    write_varint(0, &mut data); // update_index_delta

    let result = gix_reftable::parse_ref_record(&data, b"short", 20, 1);
    assert!(result.is_err(), "prefix_len > prefix length should fail");
}

/// LogRecord equality: records with all fields equal should be equal.
#[test]
fn log_record_equality() {
    use gix_reftable::LogRecord;

    let log1 = LogRecord {
        ref_name: "refs/heads/master".into(),
        update_index: 42,
        old_id: gix_hash::ObjectId::null(gix_hash::Kind::Sha1),
        new_id: gix_hash::ObjectId::null(gix_hash::Kind::Sha1),
        name: "author".into(),
        email: "author@example.com".into(),
        time: 1577123507,
        tz_offset: 100,
        message: "test".into(),
    };

    let log2 = log1.clone();
    assert_eq!(log1, log2, "cloned LogRecords should be equal");

    let log3 = LogRecord {
        message: "different".into(),
        ..log1.clone()
    };
    assert_ne!(log1, log3, "different message should not be equal");

    let log4 = LogRecord {
        time: 0,
        ..log1.clone()
    };
    assert_ne!(log1, log4, "different time should not be equal");
}

/// Multiple records with prefix compression, simulating a block's worth of records.
#[test]
fn ref_record_sequential_prefix_compression() {
    use bstr::BString;
    use gix_hash::ObjectId;

    let hash_size = 20;
    let min_update_index = 5u64;

    // Create a series of sorted ref names that share common prefixes
    let names = [
        "refs/heads/alpha",
        "refs/heads/beta",
        "refs/heads/gamma",
        "refs/tags/v1.0",
        "refs/tags/v1.1",
        "refs/tags/v2.0",
    ];

    let records: Vec<RefRecord> = names
        .iter()
        .enumerate()
        .map(|(i, name)| RefRecord {
            name: BString::from(*name),
            update_index: min_update_index,
            value: gix_reftable::RefRecordValue::Val1 {
                target: ObjectId::from_bytes_or_panic(&[i as u8; 20]),
            },
        })
        .collect();

    // Encode each record with prefix compression from the previous
    let mut prev_name: Vec<u8> = Vec::new();
    let mut encoded_records: Vec<Vec<u8>> = Vec::new();

    for record in &records {
        let bytes = gix_reftable::write::serialize_ref_record(record, &prev_name, min_update_index, hash_size);
        encoded_records.push(bytes);
        prev_name = record.name().to_vec();
    }

    // Decode them back in sequence
    prev_name.clear();
    for (i, encoded) in encoded_records.iter().enumerate() {
        let (parsed, consumed) = gix_reftable::parse_ref_record(encoded, &prev_name, hash_size, min_update_index)
            .unwrap_or_else(|e| panic!("record {i} ({}) should parse: {e}", names[i]));
        assert_eq!(consumed, encoded.len(), "consumed mismatch for record {i}");
        assert_eq!(parsed.name(), names[i].as_bytes(), "name mismatch for record {i}");
        prev_name = parsed.name().to_vec();
    }
}
