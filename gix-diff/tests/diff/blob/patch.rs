use gix_diff::blob::patch::{self, FileChange, Options};
use std::path::Path;

/// Normalize a patch string so that the `index` line is comparable between
/// C Git and gix output. C Git uses the actual abbreviated object IDs, while
/// our tests use synthetic IDs ("abcdef1"/"1234567") or none at all.
///
/// This replaces the `index <old>..<new>` portion with a fixed placeholder
/// while preserving the optional mode suffix.
fn normalize_index_line(patch: &str) -> String {
    let mut result = String::with_capacity(patch.len());
    for line in patch.lines() {
        if line.starts_with("index ") {
            // Parse: "index <old_hash>..<new_hash>" optionally followed by " <mode>"
            if let Some(rest) = line.strip_prefix("index ") {
                if let Some(dotdot_pos) = rest.find("..") {
                    let after_hashes = &rest[dotdot_pos + 2..];
                    // Check if there's a mode after the new hash
                    if let Some(space_pos) = after_hashes.find(' ') {
                        let mode = &after_hashes[space_pos..];
                        result.push_str(&format!("index NORMALIZED..NORMALIZED{mode}\n"));
                    } else {
                        result.push_str("index NORMALIZED..NORMALIZED\n");
                    }
                } else {
                    // Malformed index line, keep as-is
                    result.push_str(line);
                    result.push('\n');
                }
            }
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    // Remove the trailing newline that we may have added
    if result.ends_with('\n') && !patch.ends_with('\n') {
        result.pop();
    }
    result
}

/// Load a fixture's expected patch file.
fn load_expected_patch(fixture_dir: &Path, filename: &str) -> String {
    let path = fixture_dir.join(filename);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Helper to load old and new file contents from fixture repo commits.
/// Uses `git show` to retrieve file contents at specific revisions.
fn git_show(repo_dir: &Path, rev: &str, path: &str) -> Vec<u8> {
    let output = std::process::Command::new("git")
        .args(["show", &format!("{rev}:{path}")])
        .current_dir(repo_dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git show {rev}:{path}: {e}"));
    if !output.status.success() {
        // File might not exist at this revision (e.g., new file or deleted file)
        return Vec::new();
    }
    output.stdout
}

/// Get the abbreviated object hash for a blob at a specific revision.
fn git_blob_hash(repo_dir: &Path, rev: &str, path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", &format!("{rev}:{path}")])
        .current_dir(repo_dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git rev-parse: {e}"));
    if output.status.success() {
        Some(
            String::from_utf8(output.stdout)
                .expect("valid UTF-8")
                .trim()
                .to_string(),
        )
    } else {
        None
    }
}

/// Determine the branch name for a given scenario.
/// Each scenario creates its own branch in the fixture.
fn branch_for_scenario(scenario: &str) -> &str {
    match scenario {
        "simple_modify" => "main",
        "add_at_end" => "add-at-end",
        "remove_begin" => "remove-begin",
        "new_file" => "new-file",
        "deleted_file" => "deleted-file",
        "no_trailing_newline" => "no-newline",
        "both_no_newline" => "both-no-newline",
        "multi_hunk" => "multi-hunk",
        "binary_file" => "binary-file",
        "mode_change" => "mode-change",
        "func_name" => "func-name",
        "full_replace" => "full-replace",
        "add_newline" => "add-newline",
        "empty_to_content" => "empty-to-content",
        _ => panic!("unknown scenario: {scenario}"),
    }
}

/// File name for a given scenario.
fn file_for_scenario(scenario: &str) -> &str {
    match scenario {
        "simple_modify" => "simple.txt",
        "add_at_end" => "add_end.txt",
        "remove_begin" => "remove_begin.txt",
        "new_file" => "new_file.txt",
        "deleted_file" => "deleted.txt",
        "no_trailing_newline" => "no_nl.txt",
        "both_no_newline" => "both_no_nl.txt",
        "multi_hunk" => "multi.txt",
        "binary_file" => "binary.bin",
        "mode_change" => "script.sh",
        "func_name" => "func.c",
        "full_replace" => "replaced.txt",
        "add_newline" => "add_nl.txt",
        "empty_to_content" => "empty_to_content.txt",
        _ => panic!("unknown scenario: {scenario}"),
    }
}

fn fixture_dir() -> std::path::PathBuf {
    gix_testtools::scripted_fixture_read_only_standalone("make_diff_comparison_repo.sh").expect("valid fixture")
}

/// Generate a gix patch and compare it against C Git's output (with normalized index lines).
fn compare_patch_output(repo_dir: &Path, scenario: &str, expected_filename: &str, change: FileChange) {
    let branch = branch_for_scenario(scenario);
    let file_path = file_for_scenario(scenario);

    let old_rev = format!("{branch}~1");
    let new_rev = branch.to_string();

    let old_content = git_show(repo_dir, &old_rev, file_path);
    let new_content = git_show(repo_dir, &new_rev, file_path);

    // For new/deleted files, C Git uses "0000000" for the non-existent side.
    let old_id = git_blob_hash(repo_dir, &old_rev, file_path).or_else(|| Some("0000000".to_string()));
    let new_id = git_blob_hash(repo_dir, &new_rev, file_path).or_else(|| Some("0000000".to_string()));

    let mut out = Vec::new();
    patch::write_with_change(
        &mut out,
        old_id.as_deref(),
        new_id.as_deref(),
        file_path,
        file_path,
        &old_content,
        &new_content,
        change,
        Options::default(),
    )
    .expect("patch write should succeed");

    let gix_patch = String::from_utf8(out).expect("patch output should be UTF-8");
    let expected = load_expected_patch(repo_dir, expected_filename);

    let gix_normalized = normalize_index_line(&gix_patch);
    let expected_normalized = normalize_index_line(&expected);

    assert_eq!(
        gix_normalized, expected_normalized,
        "\n\nScenario: {scenario}\n\n--- gix output ---\n{gix_patch}\n--- expected (C Git) ---\n{expected}\n"
    );
}

mod git_comparison {
    use super::*;

    #[test]
    fn simple_modification() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "simple_modify",
            "expected_simple_modify.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }

    #[test]
    fn add_lines_at_end() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "add_at_end",
            "expected_add_at_end.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }

    #[test]
    fn remove_lines_from_beginning() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "remove_begin",
            "expected_remove_begin.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }

    #[test]
    fn new_file() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "new_file",
            "expected_new_file.patch",
            FileChange::Added { mode: 0o100644 },
        );
    }

    #[test]
    fn deleted_file() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "deleted_file",
            "expected_deleted_file.patch",
            FileChange::Deleted { mode: 0o100644 },
        );
    }

    #[test]
    fn no_trailing_newline() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "no_trailing_newline",
            "expected_no_trailing_newline.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }

    #[test]
    fn both_no_newline() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "both_no_newline",
            "expected_both_no_newline.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }

    #[test]
    fn multiple_hunks() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "multi_hunk",
            "expected_multi_hunk.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }

    #[test]
    fn binary_file() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "binary_file",
            "expected_binary_file.patch",
            FileChange::Added { mode: 0o100644 },
        );
    }

    #[test]
    fn mode_change() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "mode_change",
            "expected_mode_change.patch",
            FileChange::ModeChange {
                old_mode: 0o100644,
                new_mode: 0o100755,
            },
        );
    }

    #[test]
    fn function_name_in_hunk_header() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "func_name",
            "expected_func_name.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }

    #[test]
    fn full_content_replacement() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "full_replace",
            "expected_full_replace.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }

    #[test]
    fn add_trailing_newline() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "add_newline",
            "expected_add_newline.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }

    #[test]
    fn empty_to_content() {
        let dir = fixture_dir();
        compare_patch_output(
            &dir,
            "empty_to_content",
            "expected_empty_to_content.patch",
            FileChange::Modified { mode: Some(0o100644) },
        );
    }
}
