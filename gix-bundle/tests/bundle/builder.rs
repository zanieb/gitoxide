//! Tests for the bundle Builder API (gix_bundle::create::Builder).

use bstr::BString;
use gix_bundle::create::{Builder, Error};
use gix_bundle::{header, Version};
use gix_hash::ObjectId;

fn oid(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex.as_bytes()).expect("valid hex")
}

fn write_dummy_pack(
    writer: &mut dyn std::io::Write,
    _tips: &[ObjectId],
    _exclude: &[ObjectId],
) -> Result<bool, std::io::Error> {
    writer.write_all(b"PACK")?;
    Ok(true)
}

/// Builder should accumulate refs correctly.
#[test]
fn builder_add_ref() {
    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    let id = oid("abcdef0123456789abcdef0123456789abcdef01");

    builder.add_ref("refs/heads/main", id);

    assert_eq!(builder.tips().len(), 1);
    assert_eq!(builder.tips()[0], id);
}

/// Builder should accumulate prerequisites correctly.
#[test]
fn builder_add_prerequisite() {
    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    let ref_id = oid("abcdef0123456789abcdef0123456789abcdef01");
    let prereq_id = oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

    builder.add_ref("refs/heads/main", ref_id);
    builder.add_prerequisite(prereq_id, Some(BString::from("initial commit")));

    assert_eq!(builder.exclude().len(), 1);
    assert_eq!(builder.exclude()[0], prereq_id);
}

/// Builder with multiple refs and prerequisites.
#[test]
fn builder_multiple_refs_and_prerequisites() {
    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    let id1 = oid("1111111111111111111111111111111111111111");
    let id2 = oid("2222222222222222222222222222222222222222");
    let prereq = oid("3333333333333333333333333333333333333333");

    builder
        .add_ref("refs/heads/main", id1)
        .add_ref("refs/tags/v1.0", id2)
        .add_prerequisite(prereq, None);

    assert_eq!(builder.tips().len(), 2);
    assert_eq!(builder.exclude().len(), 1);
}

/// Builder v3 with capabilities.
#[test]
fn builder_v3_capability() {
    let mut builder = Builder::new(Version::V3, gix_hash::Kind::Sha1);
    let id = oid("abcdef0123456789abcdef0123456789abcdef01");

    builder
        .add_ref("refs/heads/main", id)
        .add_capability("object-format=sha1");

    // write_to should produce a valid header
    let mut buf = Vec::new();
    builder.write_to(&mut buf, write_dummy_pack).unwrap();

    // Parse the output to verify correctness
    let (parsed, _) = header::decode(&buf, gix_hash::Kind::Sha1).unwrap();
    assert_eq!(parsed.version, Version::V3);
    assert_eq!(parsed.capabilities.len(), 1);
    assert_eq!(parsed.capabilities[0], BString::from("object-format=sha1"));
    assert_eq!(parsed.refs.len(), 1);
}

/// Builder v3 automatically records the object format capability.
#[test]
fn builder_v3_adds_object_format_capability() {
    let mut builder = Builder::new(Version::V3, gix_hash::Kind::Sha1);
    builder.add_ref("refs/heads/main", oid("abcdef0123456789abcdef0123456789abcdef01"));

    let mut buf = Vec::new();
    builder.write_to(&mut buf, write_dummy_pack).unwrap();

    let (parsed, _) = header::decode(&buf, gix_hash::Kind::Sha1).unwrap();
    assert_eq!(parsed.capabilities, [BString::from("object-format=sha1")]);
}

/// Builder v3 should not duplicate capabilities supplied by default.
#[test]
fn builder_v3_capabilities_are_unique() {
    let mut builder = Builder::new(Version::V3, gix_hash::Kind::Sha1);
    builder
        .add_capability("object-format=sha1")
        .add_capability("object-format=sha1")
        .add_ref("refs/heads/main", oid("abcdef0123456789abcdef0123456789abcdef01"));

    let mut buf = Vec::new();
    builder.write_to(&mut buf, write_dummy_pack).unwrap();

    let (parsed, _) = header::decode(&buf, gix_hash::Kind::Sha1).unwrap();
    assert_eq!(parsed.capabilities, [BString::from("object-format=sha1")]);
}

/// Builder v3 records SHA-256 object-format when built for SHA-256 repositories.
#[test]
#[cfg(feature = "sha256")]
fn builder_v3_sha256_object_format_capability() {
    let mut builder = Builder::new(Version::V3, gix_hash::Kind::Sha256);
    builder.add_ref("refs/heads/main", gix_hash::ObjectId::null(gix_hash::Kind::Sha256));

    let mut buf = Vec::new();
    builder.write_to(&mut buf, write_dummy_pack).unwrap();

    let (parsed, _) = header::decode(&buf, gix_hash::Kind::Sha256).unwrap();
    assert_eq!(parsed.capabilities, [BString::from("object-format=sha256")]);
}

/// Ported from t5607: 'Refusing to create empty bundle'
/// Builder with no refs should fail.
#[test]
fn builder_write_fails_with_no_refs() {
    let builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    let mut buf = Vec::new();

    let result = builder.write_to(&mut buf, |_writer, _tips, _exclude| -> Result<bool, std::io::Error> {
        Ok(false)
    });

    assert!(result.is_err(), "builder with no refs should fail to write");
}

/// Builder should reject a pack writer that produced no objects.
#[test]
fn builder_write_fails_when_pack_is_empty() {
    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    builder.add_ref("refs/heads/main", oid("abcdef0123456789abcdef0123456789abcdef01"));

    let mut buf = Vec::new();
    let result = builder.write_to(&mut buf, |_writer, _tips, _exclude| -> Result<bool, std::io::Error> {
        Ok(false)
    });

    assert!(
        matches!(result, Err(Error::EmptyPack)),
        "builder should reject an empty generated pack"
    );
}

/// Builder write_to should call the pack writer with correct tips and exclude lists.
#[test]
fn builder_write_passes_tips_and_exclude_to_pack_writer() {
    use std::sync::{Arc, Mutex};

    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    let ref_id = oid("abcdef0123456789abcdef0123456789abcdef01");
    let prereq_id = oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

    builder
        .add_ref("refs/heads/main", ref_id)
        .add_prerequisite(prereq_id, Some(BString::from("comment")));

    let captured_tips: Arc<Mutex<Vec<ObjectId>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_exclude: Arc<Mutex<Vec<ObjectId>>> = Arc::new(Mutex::new(Vec::new()));
    let tips_clone = captured_tips.clone();
    let exclude_clone = captured_exclude.clone();

    let mut buf = Vec::new();
    builder
        .write_to(&mut buf, |writer, tips, exclude| -> Result<bool, std::io::Error> {
            *tips_clone.lock().unwrap() = tips.to_vec();
            *exclude_clone.lock().unwrap() = exclude.to_vec();
            writer.write_all(b"PACK")?;
            Ok(true)
        })
        .unwrap();

    let tips = captured_tips.lock().unwrap();
    let exclude = captured_exclude.lock().unwrap();

    assert_eq!(tips.len(), 1);
    assert_eq!(tips[0], ref_id);
    assert_eq!(exclude.len(), 1);
    assert_eq!(exclude[0], prereq_id);
}

/// Builder should propagate pack writer errors.
#[test]
fn builder_write_propagates_pack_error() {
    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    builder.add_ref("refs/heads/main", oid("abcdef0123456789abcdef0123456789abcdef01"));

    let mut buf = Vec::new();
    let result = builder.write_to(&mut buf, |_writer, _tips, _exclude| -> Result<bool, std::io::Error> {
        Err(std::io::Error::other("pack generation failed"))
    });

    assert!(result.is_err(), "pack writer error should propagate");
}

/// Builder write_to output should be parseable by header::decode.
#[test]
fn builder_output_is_parseable() {
    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    let id1 = oid("abcdef0123456789abcdef0123456789abcdef01");
    let id2 = oid("1234567890abcdef1234567890abcdef12345678");
    let prereq = oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

    builder
        .add_ref("refs/heads/main", id1)
        .add_ref("refs/tags/v1.0", id2)
        .add_prerequisite(prereq, Some(BString::from("boundary")));

    let mut buf = Vec::new();
    builder.write_to(&mut buf, write_dummy_pack).unwrap();

    // The header portion should be parseable
    let (parsed, _consumed) = header::decode(&buf, gix_hash::Kind::Sha1).unwrap();

    assert_eq!(parsed.version, Version::V2);
    assert_eq!(parsed.prerequisites.len(), 1);
    assert_eq!(parsed.prerequisites[0].id, prereq);
    assert_eq!(parsed.refs.len(), 2);
    assert_eq!(parsed.refs[0].id, id1);
    assert_eq!(parsed.refs[1].id, id2);
}

/// Builder write_to should write header followed by pack data from the closure.
#[test]
fn builder_writes_header_then_pack_data() {
    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    builder.add_ref("refs/heads/main", oid("abcdef0123456789abcdef0123456789abcdef01"));

    let mut buf = Vec::new();
    builder
        .write_to(&mut buf, |writer, _tips, _exclude| -> Result<bool, std::io::Error> {
            writer.write_all(b"PACK")?;
            Ok(true)
        })
        .unwrap();

    // The buffer should contain the header followed by "PACK"
    let output = String::from_utf8_lossy(&buf);
    assert!(
        output.starts_with("# v2 git bundle\n"),
        "should start with v2 signature"
    );
    assert!(
        output.ends_with("PACK"),
        "should end with pack data, got: {:?}",
        &output[output.len().saturating_sub(20)..]
    );
}

/// Builder chaining API should work fluently.
#[test]
fn builder_chaining() {
    let mut builder = Builder::new(Version::V3, gix_hash::Kind::Sha1);
    let id = oid("abcdef0123456789abcdef0123456789abcdef01");

    // All methods should return &mut Self for chaining
    builder
        .add_capability("object-format=sha1")
        .add_prerequisite(oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"), None)
        .add_ref("refs/heads/main", id);

    assert_eq!(builder.tips().len(), 1);
    assert_eq!(builder.exclude().len(), 1);
}
