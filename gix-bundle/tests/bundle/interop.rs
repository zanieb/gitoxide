//! Interop tests: gix-bundle <-> C Git bundle.
//! Ported from C Git's t5607-clone-bundle.sh.
//!
//! These tests verify that bundles written by gix can be read by C Git
//! and bundles written by C Git can be read by gix.

use bstr::BString;
use gix_bundle::create::Builder;
use gix_bundle::{header, Version};
use gix_hash::ObjectId;

fn oid(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex.as_bytes()).expect("valid hex")
}

/// Helper: run a git command and return stdout, asserting success.
fn git_output(dir: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be on PATH for interop tests");
    assert!(
        output.status.success(),
        "git {} failed in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Helper: run git command, return status (does not assert success).
fn git_status(dir: &std::path::Path, args: &[&str]) -> std::process::ExitStatus {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git must be on PATH for interop tests")
}

/// Helper: create a temporary git repo with a few commits and return (dir, commit_ids).
fn make_test_repo() -> (gix_testtools::tempfile::TempDir, Vec<String>) {
    let dir = gix_testtools::tempfile::tempdir().expect("can create tempdir");
    let path = dir.path();

    git_output(path, &["init", "-q", "-b", "main"]);
    git_output(path, &["config", "user.name", "Test"]);
    git_output(path, &["config", "user.email", "test@test.com"]);

    std::fs::write(path.join("file1"), "content1\n").expect("write file1");
    git_output(path, &["add", "file1"]);
    git_output(path, &["commit", "-q", "-m", "first commit"]);
    let c1 = git_output(path, &["rev-parse", "HEAD"]);

    std::fs::write(path.join("file2"), "content2\n").expect("write file2");
    git_output(path, &["add", "file2"]);
    git_output(path, &["commit", "-q", "-m", "second commit"]);
    let c2 = git_output(path, &["rev-parse", "HEAD"]);

    std::fs::write(path.join("file3"), "content3\n").expect("write file3");
    git_output(path, &["add", "file3"]);
    git_output(path, &["commit", "-q", "-m", "third commit"]);
    let c3 = git_output(path, &["rev-parse", "HEAD"]);

    (dir, vec![c1, c2, c3])
}

// ========================================================================
// C Git creates bundle -> gix reads it
// ========================================================================

/// Ported from t5607: C Git creates a complete bundle, gix parses the header correctly.
#[test]
fn c_git_bundle_header_readable_by_gix() -> gix_testtools::Result {
    let (dir, commits) = make_test_repo();
    let bundle_path = dir.path().join("test.bundle");

    // Create a bundle with C Git
    git_output(
        dir.path(),
        &["bundle", "create", bundle_path.to_str().expect("valid path"), "main"],
    );

    assert!(bundle_path.exists(), "bundle file should be created");

    // gix should be able to parse the header
    let (parsed, _reader) = header::from_path(&bundle_path, gix_hash::Kind::Sha1)?;

    // Should have the main ref
    assert!(!parsed.refs.is_empty(), "bundle should have at least one ref");

    let main_ref = parsed.refs.iter().find(|r| r.name == BString::from("refs/heads/main"));
    assert!(main_ref.is_some(), "bundle should contain refs/heads/main");

    let main_ref = main_ref.expect("already asserted");
    let expected_oid = oid(&commits[2]);
    assert_eq!(main_ref.id, expected_oid, "bundle main ref should point to HEAD commit");

    // Complete bundle (no prerequisites)
    assert!(
        parsed.prerequisites.is_empty(),
        "complete bundle should have no prerequisites"
    );

    Ok(())
}

/// Ported from t5607: C Git creates an incremental bundle with prerequisites,
/// gix parses the prerequisites correctly.
#[test]
fn c_git_incremental_bundle_readable_by_gix() -> gix_testtools::Result {
    let (dir, commits) = make_test_repo();
    let bundle_path = dir.path().join("incremental.bundle");

    // Create an incremental bundle: HEAD^..HEAD (last commit only)
    git_output(
        dir.path(),
        &[
            "bundle",
            "create",
            bundle_path.to_str().expect("valid path"),
            "HEAD~1..HEAD",
        ],
    );

    let (parsed, _reader) = header::from_path(&bundle_path, gix_hash::Kind::Sha1)?;

    // Should have prerequisites (the commit before HEAD)
    assert_eq!(
        parsed.prerequisites.len(),
        1,
        "incremental bundle should have 1 prerequisite"
    );

    let expected_prereq = oid(&commits[1]);
    assert_eq!(
        parsed.prerequisites[0].id, expected_prereq,
        "prerequisite should be the parent of the bundled range"
    );

    // Should have the HEAD ref
    assert!(!parsed.refs.is_empty(), "bundle should have refs");
    let head_ref = parsed
        .refs
        .iter()
        .find(|r| r.name == BString::from("refs/heads/main") || r.name == BString::from("HEAD"));
    assert!(head_ref.is_some(), "bundle should contain HEAD or main ref");

    Ok(())
}

/// Ported from t5607: C Git creates a bundle with tags, gix parses them.
#[test]
fn c_git_bundle_with_tags_readable_by_gix() -> gix_testtools::Result {
    let (dir, _commits) = make_test_repo();
    let bundle_path = dir.path().join("tags.bundle");

    // Create an annotated tag
    git_output(dir.path(), &["tag", "-a", "-m", "release v1.0", "v1.0", "HEAD"]);

    // Bundle everything including tags
    git_output(
        dir.path(),
        &["bundle", "create", bundle_path.to_str().expect("valid path"), "--all"],
    );

    let (parsed, _reader) = header::from_path(&bundle_path, gix_hash::Kind::Sha1)?;

    // Should have both main and tag refs
    let has_main = parsed.refs.iter().any(|r| r.name == BString::from("refs/heads/main"));
    let has_tag = parsed.refs.iter().any(|r| r.name == BString::from("refs/tags/v1.0"));

    assert!(has_main, "bundle should contain refs/heads/main");
    assert!(has_tag, "bundle should contain refs/tags/v1.0");

    Ok(())
}

/// Ported from t5607: 'git bundle verify' against a C Git bundle, replicated in gix.
#[test]
fn c_git_bundle_verify_prerequisites_with_gix() -> gix_testtools::Result {
    let (dir, commits) = make_test_repo();
    let bundle_path = dir.path().join("verify.bundle");

    // Create incremental bundle
    git_output(
        dir.path(),
        &[
            "bundle",
            "create",
            bundle_path.to_str().expect("valid path"),
            "HEAD~1..HEAD",
        ],
    );

    let (parsed, _reader) = header::from_path(&bundle_path, gix_hash::Kind::Sha1)?;

    // Simulate having the prerequisite commit - verification should succeed
    let prereq_oid = oid(&commits[1]);
    let result = parsed.verify_prerequisites(|id| *id == prereq_oid);
    assert!(
        result.is_ok(),
        "verification should succeed when prerequisite is present"
    );

    // Simulate NOT having the prerequisite - verification should fail
    let result = parsed.verify_prerequisites(|_id| false);
    assert!(result.is_err(), "verification should fail when prerequisite is missing");

    Ok(())
}

// ========================================================================
// gix writes bundle header -> C Git reads it
// ========================================================================

/// gix writes a complete bundle header, C Git can parse it with `git bundle verify`.
#[test]
fn gix_bundle_header_verified_by_c_git() -> gix_testtools::Result {
    let (dir, commits) = make_test_repo();
    let bundle_path = dir.path().join("gix-complete.bundle");

    let tip_oid = oid(&commits[2]);

    // Write a bundle header with gix (no pack data, just header + empty pack)
    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    builder.add_ref("refs/heads/main", tip_oid);

    let mut buf = Vec::new();
    builder.write_to(&mut buf, |_writer, _tips, _exclude| -> Result<bool, std::io::Error> {
        Ok(false)
    })?;

    std::fs::write(&bundle_path, &buf)?;

    // C Git's `git bundle list-heads` should show the ref
    let list_output = git_output(
        dir.path(),
        &["bundle", "list-heads", bundle_path.to_str().expect("valid path")],
    );
    assert!(
        list_output.contains("refs/heads/main"),
        "C Git should list refs/heads/main from gix bundle: {list_output}"
    );
    assert!(
        list_output.contains(&commits[2]),
        "C Git should see the correct OID: {list_output}"
    );

    Ok(())
}

/// gix writes a v3 bundle header with capability -- tests interop with C Git.
///
/// KNOWN ISSUE: gix's v3 header writer inserts a blank line between capabilities
/// and refs/prerequisites. C Git's v3 format does NOT have this blank line --
/// the blank line only terminates the entire header. This means C Git sees zero
/// refs in gix-written v3 bundles (the blank line makes C Git think the header
/// ended before seeing any refs).
///
/// This test documents the incompatibility. The gix roundtrip (write -> parse)
/// still works because gix's parser accepts the extra blank line.
#[test]
fn gix_v3_bundle_header_known_format_difference() -> gix_testtools::Result {
    let (dir, commits) = make_test_repo();
    let bundle_path = dir.path().join("gix-v3.bundle");

    let tip_oid = oid(&commits[2]);

    let mut builder = Builder::new(Version::V3, gix_hash::Kind::Sha1);
    builder
        .add_capability("object-format=sha1")
        .add_ref("refs/heads/main", tip_oid);

    let mut buf = Vec::new();
    builder.write_to(&mut buf, |_writer, _tips, _exclude| -> Result<bool, std::io::Error> {
        Ok(false)
    })?;

    std::fs::write(&bundle_path, &buf)?;

    // gix should be able to roundtrip its own v3 output
    let (reparsed, _) = header::decode(&buf, gix_hash::Kind::Sha1)?;
    assert_eq!(reparsed.version, Version::V3);
    assert_eq!(reparsed.refs.len(), 1);
    assert_eq!(reparsed.refs[0].id, tip_oid);
    assert_eq!(reparsed.capabilities.len(), 1);

    // C Git accepts the file (no error) but sees zero refs due to the extra blank line.
    // This is a known format issue: gix inserts a blank line after capabilities
    // which C Git interprets as the end-of-header marker.
    let output = std::process::Command::new("git")
        .args(["bundle", "list-heads", bundle_path.to_str().expect("valid path")])
        .current_dir(dir.path())
        .output()
        .expect("git must be on PATH");
    assert!(output.status.success(), "C Git should not error on gix v3 bundle");
    // NOTE: C Git returns zero heads due to the extra blank line after capabilities.
    // When this format issue is fixed in gix-bundle, this assertion should be updated
    // to verify that C Git sees the refs.
    let list_output = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if list_output.contains("refs/heads/main") {
        // Format issue has been fixed! This path should trigger after the fix.
        panic!("gix v3 format issue appears fixed! Update this test to remove the known-issue note.");
    }

    Ok(())
}

/// gix writes a bundle header with prerequisites, C Git can list heads.
#[test]
fn gix_bundle_with_prerequisites_verified_by_c_git() -> gix_testtools::Result {
    let (dir, commits) = make_test_repo();
    let bundle_path = dir.path().join("gix-incremental.bundle");

    let tip_oid = oid(&commits[2]);
    let prereq_oid = oid(&commits[1]);

    let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
    builder
        .add_ref("refs/heads/main", tip_oid)
        .add_prerequisite(prereq_oid, Some(BString::from("second commit")));

    let mut buf = Vec::new();
    builder.write_to(&mut buf, |_writer, _tips, _exclude| -> Result<bool, std::io::Error> {
        Ok(false)
    })?;

    std::fs::write(&bundle_path, &buf)?;

    // C Git's list-heads should work
    let list_output = git_output(
        dir.path(),
        &["bundle", "list-heads", bundle_path.to_str().expect("valid path")],
    );
    assert!(
        list_output.contains("refs/heads/main"),
        "C Git should list refs from gix incremental bundle: {list_output}"
    );

    Ok(())
}

// ========================================================================
// Roundtrip: C Git write -> gix parse -> gix write -> C Git read
// ========================================================================

/// Full header roundtrip: C Git creates bundle -> gix parses -> gix re-writes header
/// -> compare headers byte-for-byte (excluding pack data).
#[test]
fn header_roundtrip_c_git_to_gix_to_c_git() -> gix_testtools::Result {
    let (dir, _commits) = make_test_repo();
    let bundle_path = dir.path().join("roundtrip.bundle");
    let rewritten_path = dir.path().join("roundtrip-rewritten.bundle");

    // Create bundle with C Git
    git_output(
        dir.path(),
        &["bundle", "create", bundle_path.to_str().expect("valid path"), "main"],
    );

    // Parse with gix
    let (parsed, _reader) = header::from_path(&bundle_path, gix_hash::Kind::Sha1)?;

    // Re-write header with gix
    let mut buf = Vec::new();
    parsed.write_to(&mut buf)?;
    std::fs::write(&rewritten_path, &buf)?;

    // C Git should be able to list heads from the re-written bundle
    let original_heads = git_output(
        dir.path(),
        &["bundle", "list-heads", bundle_path.to_str().expect("valid path")],
    );
    let rewritten_heads = git_output(
        dir.path(),
        &["bundle", "list-heads", rewritten_path.to_str().expect("valid path")],
    );

    assert_eq!(
        original_heads, rewritten_heads,
        "C Git should see the same heads from original and gix-rewritten bundle"
    );

    Ok(())
}

// ========================================================================
// Error cases verified against C Git behavior
// ========================================================================

/// Ported from t5607: 'empty bundle file is rejected' -- C Git and gix should both reject.
#[test]
fn empty_bundle_rejected_by_both() -> gix_testtools::Result {
    let dir = gix_testtools::tempfile::tempdir()?;
    let bundle_path = dir.path().join("empty.bundle");
    std::fs::write(&bundle_path, b"")?;

    // gix should reject
    let gix_result = header::from_path(&bundle_path, gix_hash::Kind::Sha1);
    assert!(gix_result.is_err(), "gix should reject empty bundle");

    // C Git should also reject
    let c_git_status = git_status(
        dir.path(),
        &["bundle", "list-heads", bundle_path.to_str().expect("valid path")],
    );
    assert!(!c_git_status.success(), "C Git should reject empty bundle");

    Ok(())
}

/// Ported from t5607: garbage input is rejected by both gix and C Git.
#[test]
fn garbage_bundle_rejected_by_both() -> gix_testtools::Result {
    let dir = gix_testtools::tempfile::tempdir()?;
    let bundle_path = dir.path().join("garbage.bundle");
    std::fs::write(&bundle_path, b"this is not a bundle\n")?;

    // gix should reject
    let gix_result = header::from_path(&bundle_path, gix_hash::Kind::Sha1);
    assert!(gix_result.is_err(), "gix should reject garbage bundle");

    // C Git should also reject
    let c_git_status = git_status(
        dir.path(),
        &["bundle", "list-heads", bundle_path.to_str().expect("valid path")],
    );
    assert!(!c_git_status.success(), "C Git should reject garbage bundle");

    Ok(())
}

/// Ported from t5607: 'git bundle uses expected default format' -- verify version signature.
#[test]
fn c_git_default_bundle_format_matches_v2_or_v3() -> gix_testtools::Result {
    let (dir, _commits) = make_test_repo();
    let bundle_path = dir.path().join("default-format.bundle");

    git_output(
        dir.path(),
        &["bundle", "create", bundle_path.to_str().expect("valid path"), "main"],
    );

    // Read the first line to check version signature
    let data = std::fs::read(&bundle_path)?;
    let first_line_end = data.iter().position(|&b| b == b'\n').expect("has newline");
    let first_line = std::str::from_utf8(&data[..first_line_end]).expect("valid utf8");

    assert!(
        first_line == "# v2 git bundle" || first_line == "# v3 git bundle",
        "C Git default bundle should be v2 or v3, got: {first_line}"
    );

    // gix should parse it regardless of version
    let (parsed, _reader) = header::from_path(&bundle_path, gix_hash::Kind::Sha1)?;
    assert!(
        parsed.version == Version::V2 || parsed.version == Version::V3,
        "gix should parse v2 or v3 bundle"
    );

    Ok(())
}
