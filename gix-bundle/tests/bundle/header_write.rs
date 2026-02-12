//! Tests for bundle header writing and roundtrip (write -> parse -> compare).
//! These verify format compatibility with C Git's bundle output.

use bstr::BString;
use gix_bundle::{header, Header, Prerequisite, Ref, Version};
use gix_hash::ObjectId;

fn oid(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex.as_bytes()).expect("valid hex")
}

/// Ported from t5607: verify v2 header format matches C Git's output.
/// Expected: "# v2 git bundle\n" followed by refs, then blank line.
#[test]
fn write_v2_header_format() {
    let header = Header {
        version: Version::V2,
        prerequisites: Vec::new(),
        refs: vec![Ref {
            id: oid("abcdef0123456789abcdef0123456789abcdef01"),
            name: BString::from("refs/heads/main"),
        }],
        capabilities: Vec::new(),
    };

    let mut buf = Vec::new();
    header.write_to(&mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    // First line must be the v2 signature
    assert!(
        output.starts_with("# v2 git bundle\n"),
        "output should start with v2 signature, got: {:?}",
        output
    );

    // Must end with blank line (double newline at end)
    assert!(
        output.ends_with("\n\n"),
        "output should end with blank line, got: {:?}",
        output
    );

    // Check ref line format: "<hex> <refname>\n"
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines[0], "# v2 git bundle");
    assert_eq!(lines[1], "abcdef0123456789abcdef0123456789abcdef01 refs/heads/main");
}

/// Ported from t5607: 'git bundle v3 has expected contents'
/// Expected: "# v3 git bundle\n@object-format=sha1\n\n..." followed by refs.
#[test]
fn write_v3_header_format() {
    let header = Header {
        version: Version::V3,
        prerequisites: Vec::new(),
        refs: vec![Ref {
            id: oid("abcdef0123456789abcdef0123456789abcdef01"),
            name: BString::from("refs/heads/main"),
        }],
        capabilities: vec![BString::from("object-format=sha1")],
    };

    let mut buf = Vec::new();
    header.write_to(&mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines[0], "# v3 git bundle");
    assert_eq!(lines[1], "@object-format=sha1");
    // Blank line separates capabilities from refs
    assert_eq!(lines[2], "");
    assert_eq!(lines[3], "abcdef0123456789abcdef0123456789abcdef01 refs/heads/main");
}

/// Write prerequisite line format: "-<hex>[ <comment>]\n"
#[test]
fn write_prerequisite_format() {
    let prereq_id = oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    let header = Header {
        version: Version::V2,
        prerequisites: vec![Prerequisite {
            id: prereq_id,
            comment: Some(BString::from("initial commit")),
        }],
        refs: vec![Ref {
            id: oid("abcdef0123456789abcdef0123456789abcdef01"),
            name: BString::from("refs/heads/main"),
        }],
        capabilities: Vec::new(),
    };

    let mut buf = Vec::new();
    header.write_to(&mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines[1], "-deadbeefdeadbeefdeadbeefdeadbeefdeadbeef initial commit");
}

/// Write prerequisite without comment: "-<hex>\n"
#[test]
fn write_prerequisite_without_comment() {
    let header = Header {
        version: Version::V2,
        prerequisites: vec![Prerequisite {
            id: oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
            comment: None,
        }],
        refs: vec![Ref {
            id: oid("abcdef0123456789abcdef0123456789abcdef01"),
            name: BString::from("refs/heads/main"),
        }],
        capabilities: Vec::new(),
    };

    let mut buf = Vec::new();
    header.write_to(&mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        lines[1], "-deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        "prerequisite without comment should have no trailing space"
    );
}

/// Roundtrip: write -> parse -> compare for a complex v2 header.
#[test]
fn roundtrip_v2_complex() {
    let header = Header {
        version: Version::V2,
        prerequisites: vec![
            Prerequisite {
                id: oid("1111111111111111111111111111111111111111"),
                comment: Some(BString::from("first prerequisite")),
            },
            Prerequisite {
                id: oid("2222222222222222222222222222222222222222"),
                comment: None,
            },
        ],
        refs: vec![
            Ref {
                id: oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                name: BString::from("HEAD"),
            },
            Ref {
                id: oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                name: BString::from("refs/heads/main"),
            },
            Ref {
                id: oid("cccccccccccccccccccccccccccccccccccccccc"),
                name: BString::from("refs/tags/v1.0"),
            },
        ],
        capabilities: Vec::new(),
    };

    let mut buf = Vec::new();
    header.write_to(&mut buf).unwrap();

    let (parsed, consumed) = header::decode(&buf, gix_hash::Kind::Sha1).unwrap();
    assert_eq!(consumed, buf.len());
    assert_eq!(parsed, header);
}

/// Roundtrip: write -> parse -> compare for a v3 header with multiple capabilities.
#[test]
fn roundtrip_v3_with_multiple_capabilities() {
    let header = Header {
        version: Version::V3,
        prerequisites: vec![Prerequisite {
            id: oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
            comment: Some(BString::from("boundary commit")),
        }],
        refs: vec![Ref {
            id: oid("abcdef0123456789abcdef0123456789abcdef01"),
            name: BString::from("refs/heads/main"),
        }],
        capabilities: vec![BString::from("object-format=sha1"), BString::from("filter=blob:none")],
    };

    let mut buf = Vec::new();
    header.write_to(&mut buf).unwrap();

    let (parsed, _) = header::decode(&buf, gix_hash::Kind::Sha1).unwrap();
    assert_eq!(parsed, header);
}

/// Roundtrip: v2 with no prerequisites (complete history bundle).
#[test]
fn roundtrip_v2_no_prerequisites() {
    let header = Header {
        version: Version::V2,
        prerequisites: Vec::new(),
        refs: vec![
            Ref {
                id: oid("abcdef0123456789abcdef0123456789abcdef01"),
                name: BString::from("refs/heads/main"),
            },
            Ref {
                id: oid("1234567890abcdef1234567890abcdef12345678"),
                name: BString::from("refs/tags/v2.0"),
            },
        ],
        capabilities: Vec::new(),
    };

    let mut buf = Vec::new();
    header.write_to(&mut buf).unwrap();

    let (parsed, consumed) = header::decode(&buf, gix_hash::Kind::Sha1).unwrap();
    assert_eq!(consumed, buf.len());
    assert_eq!(parsed, header);
}

/// Roundtrip: v3 without capabilities.
#[test]
fn roundtrip_v3_empty_capabilities() {
    let header = Header {
        version: Version::V3,
        prerequisites: Vec::new(),
        refs: vec![Ref {
            id: oid("abcdef0123456789abcdef0123456789abcdef01"),
            name: BString::from("refs/heads/main"),
        }],
        capabilities: Vec::new(),
    };

    let mut buf = Vec::new();
    header.write_to(&mut buf).unwrap();

    let (parsed, _) = header::decode(&buf, gix_hash::Kind::Sha1).unwrap();
    assert_eq!(parsed.version, Version::V3);
    assert!(parsed.capabilities.is_empty());
    assert_eq!(parsed.refs.len(), 1);
}

/// V3 header without capabilities should NOT write extra blank line.
/// Format should be: "# v3 git bundle\n<ref>\n\n"
#[test]
fn v3_without_capabilities_no_extra_blank_line() {
    let header = Header {
        version: Version::V3,
        prerequisites: Vec::new(),
        refs: vec![Ref {
            id: oid("abcdef0123456789abcdef0123456789abcdef01"),
            name: BString::from("refs/heads/main"),
        }],
        capabilities: Vec::new(),
    };

    let mut buf = Vec::new();
    header.write_to(&mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    // Count blank lines (lines that are empty)
    let blank_count = output.lines().filter(|l| l.is_empty()).count();
    // There should be exactly 0 blank lines in the `lines()` output
    // (the trailing \n\n creates one empty element in lines())
    // Actually the final \n\n would show as one empty string at end
    assert!(
        blank_count <= 1,
        "v3 without capabilities should not have extra blank lines, got {} in: {:?}",
        blank_count,
        output
    );
}
