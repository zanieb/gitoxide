//! Tests for bundle prerequisite verification.
//! Ported from C Git's verify_bundle() behavior and t5607-clone-bundle.sh.

use bstr::BString;
use gix_bundle::{Header, Prerequisite, Ref, Version};
use gix_hash::ObjectId;

fn oid(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex.as_bytes()).expect("valid hex")
}

fn make_header(prerequisites: Vec<Prerequisite>) -> Header {
    Header {
        version: Version::V2,
        prerequisites,
        refs: vec![Ref {
            id: oid("abcdef0123456789abcdef0123456789abcdef01"),
            name: BString::from("refs/heads/main"),
        }],
        capabilities: Vec::new(),
    }
}

/// A bundle with no prerequisites should always verify successfully.
/// C Git: "The bundle records a complete history."
#[test]
fn no_prerequisites_always_succeeds() {
    let header = make_header(Vec::new());

    let outcome =
        header.verify_prerequisites(|_id| panic!("contains should not be called when there are no prerequisites"));

    let outcome = outcome.unwrap();
    assert_eq!(outcome.num_prerequisites, 0);
}

/// All prerequisites present: verification succeeds.
#[test]
fn all_prerequisites_present() {
    let p1 = oid("1111111111111111111111111111111111111111");
    let p2 = oid("2222222222222222222222222222222222222222");

    let header = make_header(vec![
        Prerequisite {
            id: p1,
            comment: Some(BString::from("first")),
        },
        Prerequisite {
            id: p2,
            comment: Some(BString::from("second")),
        },
    ]);

    let outcome = header.verify_prerequisites(|id| *id == p1 || *id == p2);

    let outcome = outcome.unwrap();
    assert_eq!(outcome.num_prerequisites, 2);
}

/// Ported from t5510: 'unbundle 1' -- missing prerequisite should fail.
/// C Git: "Repository lacks these prerequisite commits: ..."
#[test]
fn missing_prerequisite_fails() {
    let p1 = oid("1111111111111111111111111111111111111111");
    let p2 = oid("2222222222222222222222222222222222222222");

    let header = make_header(vec![
        Prerequisite {
            id: p1,
            comment: Some(BString::from("exists")),
        },
        Prerequisite {
            id: p2,
            comment: Some(BString::from("missing")),
        },
    ]);

    // p1 exists but p2 does not
    let result = header.verify_prerequisites(|id| *id == p1);

    assert!(result.is_err(), "missing prerequisite should fail verification");
    let err = result.unwrap_err();
    match err {
        gix_bundle::verify::Error::MissingPrerequisite { id } => {
            assert_eq!(id, p2, "error should identify the missing prerequisite");
        }
    }
}

/// First missing prerequisite is reported (fail-fast behavior).
#[test]
fn first_missing_prerequisite_is_reported() {
    let p1 = oid("1111111111111111111111111111111111111111");
    let p2 = oid("2222222222222222222222222222222222222222");

    let header = make_header(vec![
        Prerequisite { id: p1, comment: None },
        Prerequisite { id: p2, comment: None },
    ]);

    // Neither exists
    let result = header.verify_prerequisites(|_id| false);

    assert!(result.is_err());
    match result.unwrap_err() {
        gix_bundle::verify::Error::MissingPrerequisite { id } => {
            assert_eq!(id, p1, "first missing prerequisite should be reported");
        }
    }
}

/// Single prerequisite, present.
#[test]
fn single_prerequisite_present() {
    let p = oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

    let header = make_header(vec![Prerequisite {
        id: p,
        comment: Some(BString::from("initial commit")),
    }]);

    let outcome = header.verify_prerequisites(|id| *id == p).unwrap();
    assert_eq!(outcome.num_prerequisites, 1);
}

/// Single prerequisite, missing.
#[test]
fn single_prerequisite_missing() {
    let p = oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

    let header = make_header(vec![Prerequisite {
        id: p,
        comment: Some(BString::from("initial commit")),
    }]);

    let result = header.verify_prerequisites(|_id| false);
    assert!(result.is_err());
    match result.unwrap_err() {
        gix_bundle::verify::Error::MissingPrerequisite { id } => {
            assert_eq!(id, p, "should report the missing prerequisite");
        }
    }
}

/// Verify error message includes the missing OID.
#[test]
fn error_message_includes_oid() {
    let p = oid("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

    let header = make_header(vec![Prerequisite { id: p, comment: None }]);

    let result = header.verify_prerequisites(|_id| false);
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
        "error message should contain the missing OID, got: {}",
        msg
    );
}

/// Ported from t5607: verify behavior -- a bundle with all prerequisites
/// satisfied should pass, simulating a real repository check.
#[test]
fn verify_simulates_repository_object_lookup() {
    use std::collections::HashSet;

    // Simulate a repository that contains certain objects
    let known_objects: HashSet<ObjectId> = [
        oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        oid("cccccccccccccccccccccccccccccccccccccccc"),
    ]
    .into_iter()
    .collect();

    let header = make_header(vec![
        Prerequisite {
            id: oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            comment: Some(BString::from("commit A")),
        },
        Prerequisite {
            id: oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            comment: Some(BString::from("commit B")),
        },
    ]);

    let outcome = header.verify_prerequisites(|id| known_objects.contains(id)).unwrap();
    assert_eq!(outcome.num_prerequisites, 2);
}

/// Same as above but with one unknown object -- should fail.
#[test]
fn verify_fails_when_repository_missing_object() {
    use std::collections::HashSet;

    let known_objects: HashSet<ObjectId> = [oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")].into_iter().collect();

    let header = make_header(vec![
        Prerequisite {
            id: oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            comment: Some(BString::from("commit A")),
        },
        Prerequisite {
            id: oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            comment: Some(BString::from("commit B -- not in repo")),
        },
    ]);

    let result = header.verify_prerequisites(|id| known_objects.contains(id));
    assert!(result.is_err());
    match result.unwrap_err() {
        gix_bundle::verify::Error::MissingPrerequisite { id } => {
            assert_eq!(id, oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
        }
    }
}
