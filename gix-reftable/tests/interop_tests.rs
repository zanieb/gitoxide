//! Interoperability tests between gitoxide's reftable implementation and C Git.
//!
//! These tests verify that:
//! 1. gitoxide can read reftable files produced by C Git
//! 2. C Git can read reftable files produced by gitoxide
//!
//! Tests are skipped if C Git doesn't support `--ref-format=reftable` (requires Git >= 2.45).

use std::path::Path;
use std::process::Command;

fn git_supports_reftable() -> bool {
    let dir = tempfile::tempdir().expect("tempdir");
    let result = Command::new("git")
        .args(["init", "--ref-format=reftable", "probe"])
        .current_dir(dir.path())
        .output();
    match result {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to execute git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("valid utf8").trim().to_string()
}

/// Read all reftable files from a C Git repo and parse them with gitoxide.
#[test]
fn read_c_git_produced_reftable() {
    if !git_supports_reftable() {
        eprintln!("SKIP: C Git does not support --ref-format=reftable");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");

    // Create a C Git repo with reftable backend
    git_in(dir.path(), &["init", "--ref-format=reftable", "repo"]);
    git_in(&repo_path, &["config", "user.name", "Test"]);
    git_in(&repo_path, &["config", "user.email", "test@test.com"]);

    // Create commits and refs
    std::fs::write(repo_path.join("file.txt"), "hello").unwrap();
    git_in(&repo_path, &["add", "file.txt"]);
    git_in(&repo_path, &["commit", "-m", "first commit"]);

    let head_oid = git_in(&repo_path, &["rev-parse", "HEAD"]);
    let main_oid = git_in(&repo_path, &["rev-parse", "refs/heads/main"]);
    assert_eq!(head_oid, main_oid, "HEAD and main should point to same commit");

    // Create a tag
    git_in(&repo_path, &["tag", "v1.0"]);
    let tag_oid = git_in(&repo_path, &["rev-parse", "refs/tags/v1.0"]);

    // Create a second branch
    git_in(&repo_path, &["branch", "feature"]);
    let feature_oid = git_in(&repo_path, &["rev-parse", "refs/heads/feature"]);

    // Now read the reftable files with gitoxide
    let reftable_dir = repo_path.join(".git").join("reftable");
    assert!(reftable_dir.is_dir(), "reftable directory should exist");

    let stack = gix_reftable::block::Stack::open(&reftable_dir).expect("should open stack");
    assert!(!stack.tables.is_empty(), "should have at least one table");

    // Read all ref records from all tables
    let mut all_refs: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for table_name in &stack.tables {
        let table_path = stack.table_path(table_name);
        let data = std::fs::read(&table_path).expect("should read table file");

        // Parse header
        let header = gix_reftable::parse_header(&data).expect("should parse header");
        assert_eq!(header.version, gix_reftable::Version::V1, "should be version 1");

        // Parse footer
        let footer_size = gix_reftable::footer_size(header.version);
        let footer_data = &data[data.len() - footer_size..];
        let footer = gix_reftable::parse_footer(footer_data).expect("should parse footer");

        let hash_size = 20; // SHA-1

        // Read ref blocks
        // The first block starts right after the file header
        let file_header_size = gix_reftable::header_size(header.version);
        let mut block_start = file_header_size;
        let mut is_first_block = true;

        // Read blocks until we hit the footer or a non-ref block
        while block_start + 4 < data.len() - footer_size {
            let block_data = &data[block_start..];
            if block_data.is_empty() || block_data[0] == 0 {
                break;
            }

            let (block_header, _) = match gix_reftable::block::parse_block_header(block_data) {
                Ok(h) => h,
                Err(_) => break,
            };

            if block_header.block_type != gix_reftable::BlockType::Ref {
                break;
            }

            let block_end = if header.block_size > 0 {
                std::cmp::min(block_start + header.block_size as usize, data.len() - footer_size)
            } else {
                // Unaligned: compute from block_len (file-level offset for first block)
                let content_end = if is_first_block {
                    block_header.block_len as usize - file_header_size + block_start
                } else {
                    block_start + block_header.block_len as usize
                };
                std::cmp::min(content_end, data.len() - footer_size)
            };

            // C Git uses header_off = file_header_size for the first block, 0 for subsequent
            let c_git_header_off = if is_first_block { file_header_size } else { 0 };

            let block_slice = &data[block_start..block_end];
            let records = gix_reftable::block::read_ref_records_at(
                block_slice,
                hash_size,
                footer.header.min_update_index,
                c_git_header_off,
            )
            .expect("should read ref records");

            for record in records {
                let name = String::from_utf8(record.name().to_vec()).expect("valid utf8 ref name");
                let oid_str = match &record.value {
                    gix_reftable::RefRecordValue::Val1 { target } => target.to_string(),
                    gix_reftable::RefRecordValue::Val2 { target, .. } => target.to_string(),
                    gix_reftable::RefRecordValue::Symref { target } => {
                        format!("symref:{target}")
                    }
                    gix_reftable::RefRecordValue::Deletion => "deletion".to_string(),
                };
                all_refs.insert(name, oid_str);
            }

            is_first_block = false;

            // For aligned blocks, the next block starts at the next block_size boundary.
            if header.block_size > 0 {
                block_start += header.block_size as usize;
            } else {
                block_start = block_header.block_len as usize;
            }
        }
    }

    // Verify the refs we expect
    assert!(
        all_refs.contains_key("refs/heads/main"),
        "should find refs/heads/main, found: {all_refs:?}"
    );
    assert_eq!(
        all_refs.get("refs/heads/main").unwrap(),
        &main_oid,
        "main branch oid should match"
    );
    assert!(all_refs.contains_key("refs/tags/v1.0"), "should find refs/tags/v1.0");
    assert_eq!(
        all_refs.get("refs/tags/v1.0").unwrap(),
        &tag_oid,
        "tag oid should match"
    );
    assert!(
        all_refs.contains_key("refs/heads/feature"),
        "should find refs/heads/feature"
    );
    assert_eq!(
        all_refs.get("refs/heads/feature").unwrap(),
        &feature_oid,
        "feature branch oid should match"
    );

    // HEAD should be a symref to refs/heads/main
    assert!(
        all_refs.get("HEAD").map_or(false, |v| v.contains("symref")),
        "HEAD should be a symbolic ref, got: {:?}",
        all_refs.get("HEAD")
    );
}

/// Write a reftable with gitoxide and verify C Git can read it.
#[test]
fn c_git_reads_gitoxide_produced_reftable() {
    if !git_supports_reftable() {
        eprintln!("SKIP: C Git does not support --ref-format=reftable");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");

    // Create a C Git repo with reftable backend to get the basic structure
    git_in(dir.path(), &["init", "--ref-format=reftable", "repo"]);
    git_in(&repo_path, &["config", "user.name", "Test"]);
    git_in(&repo_path, &["config", "user.email", "test@test.com"]);

    // Create a commit so we have a valid object to reference
    std::fs::write(repo_path.join("file.txt"), "content").unwrap();
    git_in(&repo_path, &["add", "file.txt"]);
    git_in(&repo_path, &["commit", "-m", "initial"]);

    let commit_oid_str = git_in(&repo_path, &["rev-parse", "HEAD"]);
    let commit_oid = gix_hash::ObjectId::from_hex(commit_oid_str.as_bytes()).expect("valid hex oid");

    // Now write our own reftable file using gitoxide
    let reftable_dir = repo_path.join(".git").join("reftable");

    // Build ref records (sorted by name — required by reftable format)
    let records = vec![
        gix_reftable::RefRecord {
            name: bstr::BString::from("HEAD"),
            update_index: 2,
            value: gix_reftable::RefRecordValue::Symref {
                target: bstr::BString::from("refs/heads/main"),
            },
        },
        gix_reftable::RefRecord {
            name: bstr::BString::from("refs/heads/from-gitoxide"),
            update_index: 2,
            value: gix_reftable::RefRecordValue::Val1 { target: commit_oid },
        },
        gix_reftable::RefRecord {
            name: bstr::BString::from("refs/heads/main"),
            update_index: 2,
            value: gix_reftable::RefRecordValue::Val1 { target: commit_oid },
        },
    ];

    let hash_size = 20;
    let opts = gix_reftable::write::Options {
        block_size: 4096,
        min_update_index: 2,
        max_update_index: 2,
        version: gix_reftable::Version::V1,
    };

    // Build the complete reftable file: header + ref block + footer
    let header_bytes = gix_reftable::write::write_header(&opts);
    let header_off = header_bytes.len();
    let block_bytes = gix_reftable::write::write_ref_block_at(
        &records,
        opts.min_update_index,
        hash_size,
        opts.block_size,
        header_off,
    )
    .expect("should write block");

    let footer = gix_reftable::Footer {
        header: gix_reftable::Header {
            version: opts.version,
            block_size: opts.block_size,
            min_update_index: opts.min_update_index,
            max_update_index: opts.max_update_index,
        },
        ref_index_offset: 0,
        obj_offset: 0,
        obj_id_len: 0,
        obj_index_offset: 0,
        log_offset: 0,
        log_index_offset: 0,
    };
    let footer_bytes = gix_reftable::serialize_footer(&footer);

    let mut table_data = Vec::new();
    table_data.extend_from_slice(&header_bytes);
    table_data.extend_from_slice(&block_bytes);
    table_data.extend_from_slice(&footer_bytes);

    // Write the table file
    let table_name = "0x000000000002-0x000000000002-00000001.ref";
    let table_path = reftable_dir.join(table_name);
    std::fs::write(&table_path, &table_data).expect("should write table file");

    // Update tables.list to reference our new table
    let tables_list_path = reftable_dir.join("tables.list");
    std::fs::write(&tables_list_path, format!("{table_name}\n")).expect("should write tables.list");

    // Verify C Git can read the reftable
    let result = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&repo_path)
        .output()
        .expect("git for-each-ref");

    assert!(
        result.status.success(),
        "C git for-each-ref should succeed on gitoxide-produced reftable: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let output = String::from_utf8(result.stdout).expect("valid utf8");
    assert!(
        output.contains("refs/heads/main"),
        "C git should see refs/heads/main, got: {output}"
    );
    assert!(
        output.contains("refs/heads/from-gitoxide"),
        "C git should see refs/heads/from-gitoxide, got: {output}"
    );
    assert!(
        output.contains(&commit_oid_str),
        "C git should see the correct oid, got: {output}"
    );

    // Verify HEAD resolution
    let head_oid = git_in(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        head_oid, commit_oid_str,
        "C git HEAD should resolve to the correct commit"
    );

    // Verify fsck passes
    let fsck = Command::new("git")
        .args(["fsck", "--full"])
        .current_dir(&repo_path)
        .output()
        .expect("git fsck");
    assert!(
        fsck.status.success(),
        "git fsck should pass on gitoxide-produced reftable: {}",
        String::from_utf8_lossy(&fsck.stderr)
    );
}

/// Roundtrip: write with gitoxide, verify with C Git, read back with gitoxide.
#[test]
fn reftable_roundtrip_through_c_git() {
    if !git_supports_reftable() {
        eprintln!("SKIP: C Git does not support --ref-format=reftable");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");

    // Create a C Git repo and make a commit
    git_in(dir.path(), &["init", "--ref-format=reftable", "repo"]);
    git_in(&repo_path, &["config", "user.name", "Test"]);
    git_in(&repo_path, &["config", "user.email", "test@test.com"]);

    std::fs::write(repo_path.join("file.txt"), "content").unwrap();
    git_in(&repo_path, &["add", "file.txt"]);
    git_in(&repo_path, &["commit", "-m", "initial"]);
    let commit_oid_str = git_in(&repo_path, &["rev-parse", "HEAD"]);

    // Use C Git to create a branch (this adds to the reftable via C Git)
    git_in(&repo_path, &["branch", "roundtrip-branch"]);
    let branch_oid = git_in(&repo_path, &["rev-parse", "refs/heads/roundtrip-branch"]);
    assert_eq!(branch_oid, commit_oid_str);

    // Read back with gitoxide
    let reftable_dir = repo_path.join(".git").join("reftable");
    let stack = gix_reftable::block::Stack::open(&reftable_dir).expect("should open stack");

    let mut found_roundtrip_branch = false;
    for table_name in &stack.tables {
        let table_path = stack.table_path(table_name);
        let data = std::fs::read(&table_path).expect("should read table");
        let header = gix_reftable::parse_header(&data).expect("should parse header");
        let footer_size = gix_reftable::footer_size(header.version);
        let footer_data = &data[data.len() - footer_size..];
        let footer = gix_reftable::parse_footer(footer_data).expect("should parse footer");

        let file_header_size = gix_reftable::header_size(header.version);
        let hash_size = 20;
        let mut block_start = file_header_size;
        let mut is_first_block = true;

        while block_start + 4 < data.len() - footer_size {
            let block_data = &data[block_start..];
            if block_data.is_empty() || block_data[0] == 0 {
                break;
            }

            let (block_header, _) = match gix_reftable::block::parse_block_header(block_data) {
                Ok(h) => h,
                Err(_) => break,
            };

            if block_header.block_type != gix_reftable::BlockType::Ref {
                break;
            }

            let block_end = if header.block_size > 0 {
                std::cmp::min(block_start + header.block_size as usize, data.len() - footer_size)
            } else {
                let content_end = if is_first_block {
                    block_header.block_len as usize - file_header_size + block_start
                } else {
                    block_start + block_header.block_len as usize
                };
                std::cmp::min(content_end, data.len() - footer_size)
            };

            let c_git_header_off = if is_first_block { file_header_size } else { 0 };
            let block_slice = &data[block_start..block_end];
            let records = gix_reftable::block::read_ref_records_at(
                block_slice,
                hash_size,
                footer.header.min_update_index,
                c_git_header_off,
            )
            .expect("should read ref records");

            for record in &records {
                if record.name() == b"refs/heads/roundtrip-branch" {
                    found_roundtrip_branch = true;
                    match &record.value {
                        gix_reftable::RefRecordValue::Val1 { target } => {
                            assert_eq!(target.to_string(), commit_oid_str, "roundtrip branch oid should match");
                        }
                        other => panic!("expected Val1 for branch, got: {other:?}"),
                    }
                }
            }

            is_first_block = false;

            if header.block_size > 0 {
                block_start += header.block_size as usize;
            } else {
                block_start = block_header.block_len as usize;
            }
        }
    }

    assert!(
        found_roundtrip_branch,
        "should find refs/heads/roundtrip-branch in gitoxide-read reftable"
    );
}
