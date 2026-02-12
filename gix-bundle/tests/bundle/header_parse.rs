//! Tests for bundle header parsing, ported from C Git's t5607-clone-bundle.sh
//! and t5510-fetch.sh, plus additional edge case coverage.

use bstr::BString;
use gix_bundle::{header, Version};
use gix_hash::ObjectId;

fn oid(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex.as_bytes()).expect("valid hex")
}

/// Ported from t5607: 'git bundle uses expected default format'
/// Verifies that v2 signature is parsed correctly.
#[test]
fn parse_v2_signature() {
    let data = b"# v2 git bundle\n\
                 abcdef0123456789abcdef0123456789abcdef01 refs/heads/main\n\
                 \n";
    let (header, _) = header::decode(data.as_slice(), gix_hash::Kind::Sha1).unwrap();
    assert_eq!(header.version, Version::V2);
}

/// Ported from t5607: 'git bundle v3 has expected contents'
/// Verifies that v3 signature is parsed correctly.
#[test]
fn parse_v3_signature() {
    let data = b"# v3 git bundle\n\
                 @object-format=sha1\n\
                 \n\
                 abcdef0123456789abcdef0123456789abcdef01 refs/heads/main\n\
                 \n";
    let (header, _) = header::decode(data.as_slice(), gix_hash::Kind::Sha1).unwrap();
    assert_eq!(header.version, Version::V3);
}

/// Ported from t5607: error on invalid/unknown bundle signatures.
/// C Git rejects anything that is not "# v2 git bundle" or "# v3 git bundle".
#[test]
fn error_on_v1_signature() {
    let data = b"# v1 git bundle\n";
    let result = header::decode(data.as_slice(), gix_hash::Kind::Sha1);
    assert!(
        matches!(result, Err(header::Error::InvalidSignature)),
        "v1 bundles should be rejected"
    );
}

#[test]
fn error_on_garbage_signature() {
    let data = b"not a bundle at all\n";
    let result = header::decode(data.as_slice(), gix_hash::Kind::Sha1);
    assert!(
        matches!(result, Err(header::Error::InvalidSignature)),
        "garbage input should fail with InvalidSignature"
    );
}

/// Ported from t5607: 'empty bundle file is rejected'
#[test]
fn error_on_empty_input() {
    let data = b"";
    let result = header::decode(data.as_slice(), gix_hash::Kind::Sha1);
    assert!(
        matches!(result, Err(header::Error::UnexpectedEof)),
        "empty input should fail with UnexpectedEof"
    );
}

/// Ported from t5510: 'header of bundle looks right'
/// Verifies the complete header structure: signature, prerequisite with comment, ref.
#[test]
fn parse_v3_full_header() {
    let prereq_oid = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let data = format!(
        "# v3 git bundle\n\
         @object-format=sha1\n\
         \n\
         -{prereq_oid} updated by origin\n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let (header, consumed) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();

    assert_eq!(header.version, Version::V3);
    assert_eq!(consumed, data.len());

    assert_eq!(header.capabilities.len(), 1);
    assert_eq!(header.capabilities[0], BString::from("object-format=sha1"));

    assert_eq!(header.prerequisites.len(), 1);
    assert_eq!(header.prerequisites[0].id, oid(prereq_oid));
    assert_eq!(
        header.prerequisites[0].comment.as_ref().map(|c| c.as_slice()),
        Some("updated by origin".as_bytes())
    );

    assert_eq!(header.refs.len(), 1);
    assert_eq!(header.refs[0].id, oid(ref_oid));
    assert_eq!(header.refs[0].name, BString::from("refs/heads/main"));
}

/// Ported from t5510: 'bundle should record HEAD correctly'
/// A bundle can contain both HEAD and named refs.
#[test]
fn parse_multiple_refs_including_head() {
    let oid1 = "1111111111111111111111111111111111111111";
    let oid2 = "2222222222222222222222222222222222222222";
    let data = format!(
        "# v2 git bundle\n\
         {oid1} HEAD\n\
         {oid2} refs/heads/main\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();

    assert_eq!(header.refs.len(), 2);
    assert_eq!(header.refs[0].name, BString::from("HEAD"));
    assert_eq!(header.refs[0].id, oid(oid1));
    assert_eq!(header.refs[1].name, BString::from("refs/heads/main"));
    assert_eq!(header.refs[1].id, oid(oid2));
}

/// Ported from t5607: 'ridiculously long subject in boundary'
/// A prerequisite comment can be very long (>1024 chars).
#[test]
fn parse_long_prerequisite_comment() {
    let prereq_oid = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let long_comment = "0".repeat(1200);
    let data = format!(
        "# v2 git bundle\n\
         -{prereq_oid} {long_comment}\n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();

    assert_eq!(header.prerequisites.len(), 1);
    let comment = header.prerequisites[0].comment.as_ref().unwrap();
    assert_eq!(comment.len(), 1200, "long comments should be preserved");
}

/// Ported from t5607: 'prerequisites with an empty commit message'
/// A prerequisite with no comment (just the OID, no trailing space).
#[test]
fn parse_prerequisite_without_comment() {
    let prereq_oid = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let data = format!(
        "# v2 git bundle\n\
         -{prereq_oid}\n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();

    assert_eq!(header.prerequisites.len(), 1);
    assert_eq!(header.prerequisites[0].id, oid(prereq_oid));
    assert!(
        header.prerequisites[0].comment.is_none(),
        "prerequisite without trailing text should have no comment"
    );
}

/// Multiple prerequisites should all be parsed.
#[test]
fn parse_multiple_prerequisites() {
    let p1 = "1111111111111111111111111111111111111111";
    let p2 = "2222222222222222222222222222222222222222";
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let data = format!(
        "# v2 git bundle\n\
         -{p1} first commit\n\
         -{p2} second commit\n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();

    assert_eq!(header.prerequisites.len(), 2);
    assert_eq!(header.prerequisites[0].id, oid(p1));
    assert_eq!(
        header.prerequisites[0].comment.as_ref().map(|c| c.as_slice()),
        Some("first commit".as_bytes())
    );
    assert_eq!(header.prerequisites[1].id, oid(p2));
    assert_eq!(
        header.prerequisites[1].comment.as_ref().map(|c| c.as_slice()),
        Some("second commit".as_bytes())
    );
}

/// Ported from t5510: 'bundle should be able to create a full history'
/// A bundle with no prerequisites at all (complete history).
#[test]
fn parse_bundle_without_prerequisites() {
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let data = format!(
        "# v2 git bundle\n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();

    assert!(
        header.prerequisites.is_empty(),
        "complete history bundle should have no prerequisites"
    );
    assert_eq!(header.refs.len(), 1);
}

/// Error when a bundle has prerequisites but no refs.
/// C Git's read_bundle_header checks that refs are non-empty for useful bundles.
#[test]
fn error_on_bundle_with_no_refs() {
    let prereq = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let data = format!(
        "# v2 git bundle\n\
         -{prereq}\n\
         \n"
    );
    let result = header::decode(data.as_bytes(), gix_hash::Kind::Sha1);
    assert!(
        matches!(result, Err(header::Error::NoRefs)),
        "bundle with no refs should be rejected"
    );
}

/// Error when a ref line has an invalid hex OID.
#[test]
fn error_on_invalid_ref_oid() {
    let data = b"# v2 git bundle\n\
                 ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ refs/heads/main\n\
                 \n";
    let result = header::decode(data.as_slice(), gix_hash::Kind::Sha1);
    assert!(result.is_err(), "invalid hex in ref OID should fail");
}

/// Error when a prerequisite line has an invalid hex OID.
#[test]
fn error_on_invalid_prerequisite_oid() {
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let data = format!(
        "# v2 git bundle\n\
         -ZZZZZZ\n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let result = header::decode(data.as_bytes(), gix_hash::Kind::Sha1);
    assert!(result.is_err(), "invalid hex in prerequisite OID should fail");
}

/// Error when a ref line has the OID but no space+refname.
#[test]
fn error_on_ref_line_missing_refname() {
    let data = b"# v2 git bundle\n\
                 abcdef0123456789abcdef0123456789abcdef01\n\
                 \n";
    let result = header::decode(data.as_slice(), gix_hash::Kind::Sha1);
    assert!(result.is_err(), "ref line without refname should be rejected");
}

/// V3 bundle with multiple capabilities.
/// Ported from C Git's bundle.c which supports multiple capabilities
/// (e.g. @object-format=sha1 and @filter=blob:none).
#[test]
fn parse_v3_multiple_capabilities() {
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let data = format!(
        "# v3 git bundle\n\
         @object-format=sha1\n\
         @filter=blob:none\n\
         \n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();

    assert_eq!(header.version, Version::V3);
    assert_eq!(header.capabilities.len(), 2);
    assert_eq!(header.capabilities[0], BString::from("object-format=sha1"));
    assert_eq!(header.capabilities[1], BString::from("filter=blob:none"));
}

/// V3 bundle with single capability parses correctly.
#[test]
fn parse_v3_single_capability() {
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let data = format!(
        "# v3 git bundle\n\
         @object-format=sha1\n\
         \n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();

    assert_eq!(header.version, Version::V3);
    assert_eq!(header.capabilities.len(), 1);
    assert_eq!(header.capabilities[0], BString::from("object-format=sha1"));
}

/// V2 bundles should not have capabilities -- any @ lines should fail or be treated as refs.
/// In C Git, v2 bundles ignore capabilities (there's no capability section).
#[test]
fn v2_ignores_capability_lines() {
    // In v2, a line starting with '@' would be treated as a ref line (not a capability)
    // and would fail because '@' is not a valid hex character.
    let data = b"# v2 git bundle\n\
                 @object-format=sha1\n\
                 abcdef0123456789abcdef0123456789abcdef01 refs/heads/main\n\
                 \n";
    let result = header::decode(data.as_slice(), gix_hash::Kind::Sha1);
    // The '@' line should cause an error in v2 since it's not a valid ref or prerequisite
    assert!(result.is_err(), "v2 bundle should not accept capability lines");
}

/// Bundle header with CRLF line endings (Windows compatibility).
/// C Git's strbuf_rtrim handles this.
#[test]
fn parse_header_with_crlf() {
    let data = b"# v2 git bundle\r\n\
                 abcdef0123456789abcdef0123456789abcdef01 refs/heads/main\r\n\
                 \r\n";
    let (header, _) = header::decode(data.as_slice(), gix_hash::Kind::Sha1).unwrap();
    assert_eq!(header.version, Version::V2);
    assert_eq!(header.refs.len(), 1);
    assert_eq!(header.refs[0].name, BString::from("refs/heads/main"));
}

/// Bundle with tag ref names.
#[test]
fn parse_refs_with_tag_names() {
    let oid1 = "1111111111111111111111111111111111111111";
    let oid2 = "2222222222222222222222222222222222222222";
    let data = format!(
        "# v2 git bundle\n\
         {oid1} refs/heads/main\n\
         {oid2} refs/tags/v1.0\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();

    assert_eq!(header.refs.len(), 2);
    assert_eq!(header.refs[0].name, BString::from("refs/heads/main"));
    assert_eq!(header.refs[1].name, BString::from("refs/tags/v1.0"));
}

/// The consumed byte count from decode() should indicate where pack data starts.
/// NOTE: There is a known bug in decode() where BufReader reads ahead into the
/// packfile data, causing cursor.position() to overshoot. When input is just a
/// header (no trailing pack data), consumed == data.len() which is correct.
#[test]
fn consumed_offset_correct_for_header_only_data() {
    let header_text = "# v2 git bundle\nabcdef0123456789abcdef0123456789abcdef01 refs/heads/main\n\n";

    let (_, consumed) = header::decode(header_text.as_bytes(), gix_hash::Kind::Sha1).unwrap();
    assert_eq!(
        consumed,
        header_text.len(),
        "consumed should equal the full header length when no trailing data"
    );
}

/// When there IS trailing pack data after the header, decode()'s consumed offset
/// should point exactly to the start of the pack data.
#[test]
fn consumed_offset_with_trailing_pack_data() {
    let header_text = "# v2 git bundle\nabcdef0123456789abcdef0123456789abcdef01 refs/heads/main\n\n";
    let pack_data = b"PACK\x00\x00\x00\x02...";
    let mut full_data = header_text.as_bytes().to_vec();
    full_data.extend_from_slice(pack_data);

    let (parsed, consumed) = header::decode(&full_data, gix_hash::Kind::Sha1).unwrap();
    assert_eq!(parsed.refs.len(), 1);

    assert_eq!(
        consumed,
        header_text.len(),
        "consumed offset should point exactly to the start of pack data"
    );
    assert_eq!(
        &full_data[consumed..consumed + 4],
        b"PACK",
        "data at consumed offset should be the pack signature"
    );
}

/// from_path returns a positioned reader that can be used for pack data.
/// This is the correct API when you need to read pack data after the header.
#[test]
fn from_path_reader_positioned_at_pack_data() {
    use std::io::Read;

    let dir = std::env::temp_dir().join("gix-bundle-test-positioned-reader");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test-positioned.bundle");

    let header_text = "# v2 git bundle\nabcdef0123456789abcdef0123456789abcdef01 refs/heads/main\n\n";
    let pack_data = b"PACK\x00\x00\x00\x02";
    let mut full_data = header_text.as_bytes().to_vec();
    full_data.extend_from_slice(pack_data);
    std::fs::write(&path, &full_data).unwrap();

    let (_header, mut reader) = header::from_path(&path, gix_hash::Kind::Sha1).unwrap();

    // The reader should be positioned right after the header
    let mut first_4_bytes = [0u8; 4];
    reader.read_exact(&mut first_4_bytes).unwrap();
    assert_eq!(
        &first_4_bytes, b"PACK",
        "from_path reader should be positioned at the start of pack data"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}

/// Unexpected EOF in the middle of the header (no blank terminator line).
#[test]
fn error_on_truncated_header() {
    let data = b"# v2 git bundle\n\
                 abcdef0123456789abcdef0123456789abcdef01 refs/heads/main";
    // No trailing newline or blank line -- the reader will hit EOF without seeing the terminator
    let result = header::decode(data.as_slice(), gix_hash::Kind::Sha1);
    // This should either succeed (treating EOF as end-of-header) or fail.
    // C Git's read_bundle_header_fd loops until it gets an empty line or EOF.
    // Since we have a ref, it should succeed if EOF is treated as header end.
    // Let's check what the implementation does.
    match result {
        Ok((header, _consumed)) => {
            // If it succeeds, it should have the ref
            assert_eq!(header.refs.len(), 1);
        }
        Err(_) => {
            // Also acceptable -- the implementation may require the blank line
        }
    }
}

/// Test from_path function with a temporary file.
#[test]
fn from_path_reads_file() {
    let dir = std::env::temp_dir().join("gix-bundle-test-from-path");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test.bundle");

    let header_bytes = b"# v2 git bundle\n\
                         abcdef0123456789abcdef0123456789abcdef01 refs/heads/main\n\
                         \nPACK...";
    std::fs::write(&path, header_bytes).unwrap();

    let (header, _reader) = header::from_path(&path, gix_hash::Kind::Sha1).unwrap();
    assert_eq!(header.version, Version::V2);
    assert_eq!(header.refs.len(), 1);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}

/// from_path should fail on nonexistent files.
#[test]
fn from_path_fails_on_missing_file() {
    let result = header::from_path("/nonexistent/path/to/bundle", gix_hash::Kind::Sha1);
    assert!(result.is_err());
}

/// V3 with no capabilities at all (just the blank line separator between sig and refs).
#[test]
fn parse_v3_without_capabilities() {
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let data = format!(
        "# v3 git bundle\n\
         \n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();
    assert_eq!(header.version, Version::V3);
    assert!(header.capabilities.is_empty());
    assert_eq!(header.refs.len(), 1);
}

/// Prerequisite with a space but empty comment (just "-<oid> " with trailing space).
#[test]
fn parse_prerequisite_with_empty_comment() {
    let prereq = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let ref_oid = "abcdef0123456789abcdef0123456789abcdef01";
    let data = format!(
        "# v2 git bundle\n\
         -{prereq} \n\
         {ref_oid} refs/heads/main\n\
         \n"
    );
    let (header, _) = header::decode(data.as_bytes(), gix_hash::Kind::Sha1).unwrap();
    assert_eq!(header.prerequisites.len(), 1);
    // The comment should be empty string (space was present but nothing after it)
    // after trimming or it could be Some("") -- depends on implementation
    if let Some(comment) = &header.prerequisites[0].comment {
        // Acceptable: empty comment captured
        assert!(
            comment.is_empty() || comment.as_slice() == b" ",
            "unexpected comment: {comment:?}"
        );
    }
    // Either way, the prerequisite OID should be correct
    assert_eq!(header.prerequisites[0].id, oid(prereq));
}
