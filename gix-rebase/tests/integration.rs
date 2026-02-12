//! End-to-end integration tests for the rebase driver.
//!
//! These tests create real Git repositories (via the `git` CLI) and drive
//! `MergeState::step()` through a `GitCliDriver` that performs actual
//! cherry-picks against the repository. This validates that the rebase state
//! machine correctly interacts with a real repository, not just a mock.

use std::path::{Path, PathBuf};
use std::process::Command;

use gix_hash::{Kind, ObjectId};
use gix_rebase::{CherryPickError, CherryPickOutcome, Driver, MergeState, StepOutcome};
use gix_sequencer::todo::{Operation, TodoList};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_oid(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex.as_bytes()).expect("valid hex for ObjectId")
}

/// Run a git command in `dir` and return its stdout (trimmed).
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test Author")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test Committer")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {}: {e}", args.join(" ")));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "git {} failed (exit {:?}):\nstdout: {stdout}\nstderr: {stderr}",
            args.join(" "),
            output.status.code()
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Get the HEAD commit id in `dir`.
fn head_oid(dir: &Path) -> ObjectId {
    let hex = git(dir, &["rev-parse", "HEAD"]);
    make_oid(&hex)
}

/// Get the commit message for a given commit.
fn commit_message(dir: &Path, rev: &str) -> String {
    git(dir, &["log", "-1", "--format=%B", rev])
}

/// Get the number of commits on the current branch.
fn commit_count(dir: &Path) -> usize {
    let output = git(dir, &["rev-list", "--count", "HEAD"]);
    output.parse::<usize>().expect("valid commit count")
}

// ---------------------------------------------------------------------------
// GitCliDriver -- a real Driver backed by git CLI
// ---------------------------------------------------------------------------

/// A `Driver` implementation that uses the `git` CLI to perform real
/// cherry-pick operations against a repository on disk.
struct GitCliDriver {
    workdir: PathBuf,
}

impl GitCliDriver {
    fn new(workdir: &Path) -> Self {
        Self {
            workdir: workdir.to_owned(),
        }
    }
}

impl Driver for GitCliDriver {
    fn resolve_commit(&self, prefix: &gix_hash::Prefix) -> Result<ObjectId, Box<dyn std::error::Error + Send + Sync>> {
        let hex = prefix.as_oid().to_hex().to_string();
        let output = Command::new("git")
            .args(["rev-parse", &hex])
            .current_dir(&self.workdir)
            .output()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("resolve_commit failed for {hex}: {stderr}").into());
        }
        let full_hex = String::from_utf8_lossy(&output.stdout).trim().to_string();
        ObjectId::from_hex(full_hex.as_bytes()).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
    }

    fn cherry_pick(&self, commit_id: ObjectId, message: Option<&[u8]>) -> Result<CherryPickOutcome, CherryPickError> {
        let hex = commit_id.to_hex().to_string();

        // Use git cherry-pick --no-commit to apply changes, then commit manually.
        // This lets us override the message for squash/fixup.
        let output = Command::new("git")
            .args(["cherry-pick", "--no-commit", &hex])
            .current_dir(&self.workdir)
            .env("GIT_AUTHOR_NAME", "Test Author")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test Committer")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .map_err(|e| CherryPickError::Other {
                message: format!("failed to run git cherry-pick: {e}"),
                source: Box::new(e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Check if this is a conflict.
            if stderr.contains("conflict") || stderr.contains("CONFLICT") {
                // Abort the cherry-pick state.
                let _ = Command::new("git")
                    .args(["cherry-pick", "--abort"])
                    .current_dir(&self.workdir)
                    .output();
                return Err(CherryPickError::Conflict { commit_id });
            }
            return Err(CherryPickError::Other {
                message: format!("cherry-pick failed: {stderr}"),
                source: format!("cherry-pick failed").into(),
            });
        }

        // Determine commit message.
        let msg = if let Some(m) = message {
            String::from_utf8_lossy(m).to_string()
        } else {
            // Use the original commit's message.
            let msg_output = Command::new("git")
                .args(["log", "-1", "--format=%B", &hex])
                .current_dir(&self.workdir)
                .output()
                .map_err(|e| CherryPickError::Other {
                    message: format!("failed to read commit message: {e}"),
                    source: Box::new(e),
                })?;
            String::from_utf8_lossy(&msg_output.stdout).trim_end().to_string()
        };

        // Create or amend the commit.
        // When `message` is provided (squash/fixup), amend the previous commit
        // to fold changes into it, matching C Git's sequencer behavior.
        let commit_args = if message.is_some() {
            vec!["commit", "--allow-empty", "--amend", "-m", &msg]
        } else {
            vec!["commit", "--allow-empty", "-m", &msg]
        };
        let commit_output = Command::new("git")
            .args(&commit_args)
            .current_dir(&self.workdir)
            .env("GIT_AUTHOR_NAME", "Test Author")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test Committer")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .map_err(|e| CherryPickError::Other {
                message: format!("failed to commit: {e}"),
                source: Box::new(e),
            })?;

        if !commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&commit_output.stderr);
            return Err(CherryPickError::Other {
                message: format!("commit failed: {stderr}"),
                source: format!("commit failed").into(),
            });
        }

        let new_head = head_oid(&self.workdir);
        Ok(CherryPickOutcome {
            new_commit_id: new_head,
        })
    }

    fn read_commit_message(&self, commit_id: ObjectId) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let hex = commit_id.to_hex().to_string();
        let output = Command::new("git")
            .args(["log", "-1", "--format=%B", &hex])
            .current_dir(&self.workdir)
            .output()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("read_commit_message failed: {stderr}").into());
        }
        // Trim trailing newline to match git log behavior.
        let msg = output.stdout;
        Ok(msg)
    }

    fn update_head(&self, commit_id: ObjectId) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let hex = commit_id.to_hex().to_string();
        let output = Command::new("git")
            .args(["reset", "--hard", &hex])
            .current_dir(&self.workdir)
            .output()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("update_head failed: {stderr}").into());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Fixture: create a test repository suitable for rebase tests
// ---------------------------------------------------------------------------

/// Create a repository with this structure:
///
/// ```text
///   A -- B -- C (main)
///    \
///     D -- E -- F (feature)
/// ```
///
/// Where:
///   A: initial commit (file.txt = "A\n")
///   B: main: update file.txt to "B\n"
///   C: main: add main_only.txt
///   D: feature: update file.txt to "D\n"    (conflicts with B)
///   E: feature: add feature.txt
///   F: feature: update file.txt to "F\n"    (conflicts with B)
///
/// Returns (tmpdir, path to workdir, onto=C, orig_head=F, commit_ids for A-F).
struct RebaseFixture {
    _tmpdir: tempfile::TempDir,
    workdir: PathBuf,
    commits: std::collections::HashMap<String, ObjectId>,
}

impl RebaseFixture {
    fn new_basic() -> Self {
        let tmpdir = tempfile::tempdir().expect("create tempdir");
        let workdir = tmpdir.path().to_owned();

        // Initialize repo.
        git(&workdir, &["init", "-b", "main"]);
        git(&workdir, &["config", "user.name", "Test"]);
        git(&workdir, &["config", "user.email", "test@test.com"]);

        // Commit A: initial
        std::fs::write(workdir.join("file.txt"), "A\n").expect("write file.txt");
        git(&workdir, &["add", "file.txt"]);
        git(&workdir, &["commit", "-m", "A: initial commit"]);
        let a = head_oid(&workdir);

        // Commit B: modify file.txt on main
        std::fs::write(workdir.join("file.txt"), "B\n").expect("write file.txt");
        git(&workdir, &["add", "file.txt"]);
        git(&workdir, &["commit", "-m", "B: update file.txt on main"]);
        let b = head_oid(&workdir);

        // Commit C: add main_only.txt
        std::fs::write(workdir.join("main_only.txt"), "main content\n").expect("write main_only.txt");
        git(&workdir, &["add", "main_only.txt"]);
        git(&workdir, &["commit", "-m", "C: add main_only.txt"]);
        let c = head_oid(&workdir);

        // Create feature branch from A.
        git(&workdir, &["checkout", "-b", "feature", &a.to_hex().to_string()]);

        // Commit D: modify file.txt on feature (will conflict with B when rebasing)
        std::fs::write(workdir.join("file.txt"), "D\n").expect("write file.txt");
        git(&workdir, &["add", "file.txt"]);
        git(&workdir, &["commit", "-m", "D: update file.txt on feature"]);
        let d = head_oid(&workdir);

        // Commit E: add feature.txt (no conflict)
        std::fs::write(workdir.join("feature.txt"), "feature content\n").expect("write feature.txt");
        git(&workdir, &["add", "feature.txt"]);
        git(&workdir, &["commit", "-m", "E: add feature.txt"]);
        let e = head_oid(&workdir);

        // Commit F: modify file.txt again
        std::fs::write(workdir.join("file.txt"), "F\n").expect("write file.txt");
        git(&workdir, &["add", "file.txt"]);
        git(&workdir, &["commit", "-m", "F: update file.txt again"]);
        let f = head_oid(&workdir);

        let mut commits = std::collections::HashMap::new();
        commits.insert("A".to_string(), a);
        commits.insert("B".to_string(), b);
        commits.insert("C".to_string(), c);
        commits.insert("D".to_string(), d);
        commits.insert("E".to_string(), e);
        commits.insert("F".to_string(), f);

        Self {
            _tmpdir: tmpdir,
            workdir,
            commits,
        }
    }

    /// Create a repository for non-conflicting rebase:
    ///
    /// ```text
    ///   A -- B (main)
    ///    \
    ///     C -- D (feature)
    /// ```
    ///
    /// Where files touched by C/D don't overlap with B.
    fn new_no_conflict() -> Self {
        let tmpdir = tempfile::tempdir().expect("create tempdir");
        let workdir = tmpdir.path().to_owned();

        git(&workdir, &["init", "-b", "main"]);
        git(&workdir, &["config", "user.name", "Test"]);
        git(&workdir, &["config", "user.email", "test@test.com"]);

        // Commit A
        std::fs::write(workdir.join("base.txt"), "base\n").expect("write base.txt");
        git(&workdir, &["add", "base.txt"]);
        git(&workdir, &["commit", "-m", "A: initial commit"]);
        let a = head_oid(&workdir);

        // Commit B on main: touch main_file.txt
        std::fs::write(workdir.join("main_file.txt"), "main\n").expect("write main_file.txt");
        git(&workdir, &["add", "main_file.txt"]);
        git(&workdir, &["commit", "-m", "B: add main_file.txt"]);
        let b = head_oid(&workdir);

        // Feature branch from A.
        git(&workdir, &["checkout", "-b", "feature", &a.to_hex().to_string()]);

        // Commit C: add feature1.txt
        std::fs::write(workdir.join("feature1.txt"), "feature1\n").expect("write");
        git(&workdir, &["add", "feature1.txt"]);
        git(&workdir, &["commit", "-m", "C: add feature1.txt"]);
        let c = head_oid(&workdir);

        // Commit D: add feature2.txt
        std::fs::write(workdir.join("feature2.txt"), "feature2\n").expect("write");
        git(&workdir, &["add", "feature2.txt"]);
        git(&workdir, &["commit", "-m", "D: add feature2.txt"]);
        let d = head_oid(&workdir);

        let mut commits = std::collections::HashMap::new();
        commits.insert("A".to_string(), a);
        commits.insert("B".to_string(), b);
        commits.insert("C".to_string(), c);
        commits.insert("D".to_string(), d);

        Self {
            _tmpdir: tmpdir,
            workdir,
            commits,
        }
    }

    /// Create a repository for squash/fixup tests:
    ///
    /// ```text
    ///   A (main)
    ///    \
    ///     B -- C -- D (feature)
    /// ```
    ///
    /// All feature commits touch different files.
    fn new_squash() -> Self {
        let tmpdir = tempfile::tempdir().expect("create tempdir");
        let workdir = tmpdir.path().to_owned();

        git(&workdir, &["init", "-b", "main"]);
        git(&workdir, &["config", "user.name", "Test"]);
        git(&workdir, &["config", "user.email", "test@test.com"]);

        // Commit A
        std::fs::write(workdir.join("base.txt"), "base\n").expect("write");
        git(&workdir, &["add", "base.txt"]);
        git(&workdir, &["commit", "-m", "A: initial"]);
        let a = head_oid(&workdir);

        // Feature branch from A.
        git(&workdir, &["checkout", "-b", "feature"]);

        // Commit B
        std::fs::write(workdir.join("b.txt"), "B content\n").expect("write");
        git(&workdir, &["add", "b.txt"]);
        git(&workdir, &["commit", "-m", "B: add b.txt"]);
        let b = head_oid(&workdir);

        // Commit C
        std::fs::write(workdir.join("c.txt"), "C content\n").expect("write");
        git(&workdir, &["add", "c.txt"]);
        git(&workdir, &["commit", "-m", "C: add c.txt"]);
        let c = head_oid(&workdir);

        // Commit D
        std::fs::write(workdir.join("d.txt"), "D content\n").expect("write");
        git(&workdir, &["add", "d.txt"]);
        git(&workdir, &["commit", "-m", "D: add d.txt"]);
        let d = head_oid(&workdir);

        let mut commits = std::collections::HashMap::new();
        commits.insert("A".to_string(), a);
        commits.insert("B".to_string(), b);
        commits.insert("C".to_string(), c);
        commits.insert("D".to_string(), d);

        Self {
            _tmpdir: tmpdir,
            workdir,
            commits,
        }
    }

    fn oid(&self, name: &str) -> ObjectId {
        *self
            .commits
            .get(name)
            .unwrap_or_else(|| panic!("commit {name} not found"))
    }

    fn prefix(&self, name: &str) -> gix_hash::Prefix {
        self.oid(name).into()
    }

    fn rebase_dir(&self) -> PathBuf {
        self.workdir.join(".git").join("rebase-merge")
    }

    fn driver(&self) -> GitCliDriver {
        GitCliDriver::new(&self.workdir)
    }

    /// Reset HEAD (detached) to the given commit, simulating the start of a rebase.
    fn detach_head_to(&self, name: &str) {
        let hex = self.oid(name).to_hex().to_string();
        git(&self.workdir, &["checkout", "--detach", &hex]);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod pick {
    use super::*;

    #[test]
    fn pick_two_commits_onto_main() -> Result<(), Box<dyn std::error::Error>> {
        // Rebase feature (C, D) onto main (B) with no conflicts.
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        // Detach HEAD to the "onto" target (B on main).
        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Step 1: pick C.
        let outcome = state.step(&driver, &rebase_dir)?;
        assert!(
            matches!(outcome, StepOutcome::Applied { .. }),
            "first pick should succeed"
        );

        // Verify C was applied: feature1.txt should exist.
        assert!(
            fix.workdir.join("feature1.txt").exists(),
            "feature1.txt should exist after picking C"
        );
        let msg = commit_message(&fix.workdir, "HEAD");
        assert!(
            msg.contains("C: add feature1.txt"),
            "HEAD commit message should be from C: {msg}"
        );

        // Step 2: pick D.
        let outcome = state.step(&driver, &rebase_dir)?;
        assert!(
            matches!(outcome, StepOutcome::Applied { .. }),
            "second pick should succeed"
        );

        // Verify D was applied: feature2.txt should exist.
        assert!(
            fix.workdir.join("feature2.txt").exists(),
            "feature2.txt should exist after picking D"
        );

        // Verify we're done.
        let outcome = state.step(&driver, &rebase_dir)?;
        assert_eq!(outcome, StepOutcome::Done);

        // Verify commit count: A + B + C' + D' = 4 commits total.
        assert_eq!(commit_count(&fix.workdir), 4, "should have 4 commits: A, B, C', D'");

        // Both feature files and main_file.txt should exist.
        assert!(fix.workdir.join("main_file.txt").exists());
        assert!(fix.workdir.join("feature1.txt").exists());
        assert!(fix.workdir.join("feature2.txt").exists());

        // The rebase state should be persisted on disk.
        assert!(rebase_dir.exists(), "rebase-merge dir should exist (caller cleans up)");

        Ok(())
    }

    #[test]
    fn pick_preserves_commit_messages() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.step(&driver, &rebase_dir)?;
        state.step(&driver, &rebase_dir)?;

        // Check that commit messages are preserved in the rebased commits.
        let log = git(&fix.workdir, &["log", "--format=%s", "--reverse"]);
        let subjects: Vec<&str> = log.lines().collect();
        assert_eq!(subjects.len(), 4);
        assert_eq!(subjects[0], "A: initial commit");
        assert_eq!(subjects[1], "B: add main_file.txt");
        assert_eq!(subjects[2], "C: add feature1.txt");
        assert_eq!(subjects[3], "D: add feature2.txt");

        Ok(())
    }

    #[test]
    fn pick_produces_new_commit_ids() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![Operation::Pick {
                    commit: fix.prefix("C"),
                    summary: "C: add feature1.txt".into(),
                }]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 1,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        let outcome = state.step(&driver, &rebase_dir)?;
        if let StepOutcome::Applied { new_commit } = outcome {
            // The new commit should differ from the original C (different parent).
            assert_ne!(
                new_commit,
                fix.oid("C"),
                "rebased commit should have a different OID than original"
            );
            // It should be the current HEAD.
            assert_eq!(new_commit, head_oid(&fix.workdir));
        } else {
            panic!("expected Applied outcome");
        }

        Ok(())
    }
}

mod drop_op {
    use super::*;

    #[test]
    fn drop_skips_commit() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Drop {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Drop C.
        let outcome = state.step(&driver, &rebase_dir)?;
        assert_eq!(outcome, StepOutcome::Skipped);

        // Pick D.
        let outcome = state.step(&driver, &rebase_dir)?;
        assert!(matches!(outcome, StepOutcome::Applied { .. }));

        // Verify: feature1.txt should NOT exist (C was dropped), but feature2.txt should.
        assert!(
            !fix.workdir.join("feature1.txt").exists(),
            "feature1.txt should not exist (commit C was dropped)"
        );
        assert!(
            fix.workdir.join("feature2.txt").exists(),
            "feature2.txt should exist (commit D was picked)"
        );

        // Should be A, B, D' = 3 commits.
        assert_eq!(commit_count(&fix.workdir), 3, "should have 3 commits: A, B, D'");

        Ok(())
    }
}

mod squash_fixup {
    use super::*;

    #[test]
    fn squash_combines_commits_and_messages() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_squash();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        // Detach HEAD at A (rebasing feature onto A itself, effectively replaying).
        fix.detach_head_to("A");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("A"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("B"),
                        summary: "B: add b.txt".into(),
                    },
                    Operation::Squash {
                        commit: fix.prefix("C"),
                        summary: "C: add c.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Pick B.
        state.step(&driver, &rebase_dir)?;
        // Squash C into B.
        state.step(&driver, &rebase_dir)?;

        // The resulting commit's message should contain both B and C messages.
        let msg = commit_message(&fix.workdir, "HEAD");
        assert!(
            msg.contains("B: add b.txt"),
            "squashed message should contain B's message: {msg}"
        );
        assert!(
            msg.contains("C: add c.txt"),
            "squashed message should contain C's message: {msg}"
        );

        // Both files should exist.
        assert!(fix.workdir.join("b.txt").exists());
        assert!(fix.workdir.join("c.txt").exists());

        // Should be A + squashed(B+C) = 2 commits.
        assert_eq!(commit_count(&fix.workdir), 2, "squash should result in 2 commits");

        Ok(())
    }

    #[test]
    fn fixup_discards_message() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_squash();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("A");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("A"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("B"),
                        summary: "B: add b.txt".into(),
                    },
                    Operation::Fixup {
                        commit: fix.prefix("C"),
                        summary: "C: add c.txt".into(),
                        amend_message: gix_sequencer::todo::AmendMessage::No,
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.step(&driver, &rebase_dir)?;
        state.step(&driver, &rebase_dir)?;

        // The fixup commit message should be B's message only (C's discarded).
        let msg = commit_message(&fix.workdir, "HEAD");
        assert!(
            msg.contains("B: add b.txt"),
            "fixup result should have B's message: {msg}"
        );
        assert!(!msg.contains("C: add c.txt"), "fixup should discard C's message: {msg}");

        // Both files should exist (the changes are applied, just not the message).
        assert!(fix.workdir.join("b.txt").exists());
        assert!(fix.workdir.join("c.txt").exists());

        Ok(())
    }

    #[test]
    fn pick_squash_squash_three_into_one() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_squash();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("A");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("A"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("B"),
                        summary: "B: add b.txt".into(),
                    },
                    Operation::Squash {
                        commit: fix.prefix("C"),
                        summary: "C: add c.txt".into(),
                    },
                    Operation::Squash {
                        commit: fix.prefix("D"),
                        summary: "D: add d.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 3,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.step(&driver, &rebase_dir)?;
        state.step(&driver, &rebase_dir)?;
        state.step(&driver, &rebase_dir)?;

        // All three files should exist.
        assert!(fix.workdir.join("b.txt").exists());
        assert!(fix.workdir.join("c.txt").exists());
        assert!(fix.workdir.join("d.txt").exists());

        // Should be A + squashed(B+C+D) = 2 commits.
        assert_eq!(
            commit_count(&fix.workdir),
            2,
            "squashing 3 commits should produce 2 total"
        );

        Ok(())
    }
}

mod edit_and_break {
    use super::*;

    #[test]
    fn edit_pauses_then_continues() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Edit {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Step: edit should pause.
        let outcome = state.step(&driver, &rebase_dir)?;
        assert!(
            matches!(outcome, StepOutcome::Paused { commit_id: Some(_), .. }),
            "edit should pause with a commit_id"
        );

        // Verify: stopped_sha is set and persisted.
        assert!(state.stopped_sha.is_some());
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1)?;
        assert!(on_disk.stopped_sha.is_some());

        // feature1.txt should exist (the edit commit was applied).
        assert!(fix.workdir.join("feature1.txt").exists());

        // Continue: should pick D.
        let outcome = state.continue_rebase(&driver, &rebase_dir)?;
        assert!(
            matches!(outcome, StepOutcome::Applied { .. }),
            "continue should apply D"
        );

        // feature2.txt should now exist.
        assert!(fix.workdir.join("feature2.txt").exists());

        // Should be done.
        let outcome = state.step(&driver, &rebase_dir)?;
        assert_eq!(outcome, StepOutcome::Done);

        Ok(())
    }

    #[test]
    fn break_pauses_without_commit() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");
        let head_before = head_oid(&fix.workdir);

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Break,
                    Operation::Pick {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Break should pause.
        let outcome = state.step(&driver, &rebase_dir)?;
        assert_eq!(
            outcome,
            StepOutcome::Paused {
                commit_id: None,
                original_message: None,
            },
            "break should pause without commit_id"
        );

        // HEAD should be unchanged.
        assert_eq!(head_oid(&fix.workdir), head_before);

        // Continue should pick C.
        let outcome = state.continue_rebase(&driver, &rebase_dir)?;
        assert!(matches!(outcome, StepOutcome::Applied { .. }));
        assert!(fix.workdir.join("feature1.txt").exists());

        Ok(())
    }
}

mod abort_rebase {
    use super::*;

    #[test]
    fn abort_restores_head_and_cleans_state() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        // Start rebasing: detach at B.
        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Write state and apply first pick.
        state.write_to(&rebase_dir)?;
        state.step(&driver, &rebase_dir)?;

        // Now abort.
        state.abort(&driver, &rebase_dir)?;

        // HEAD should be restored to orig_head (D on feature).
        assert_eq!(
            head_oid(&fix.workdir),
            fix.oid("D"),
            "HEAD should be restored to orig_head after abort"
        );

        // Rebase state directory should be removed.
        assert!(!rebase_dir.exists(), "rebase-merge dir should be removed");

        Ok(())
    }

    #[test]
    fn abort_after_edit_pause() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Edit {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir)?;
        state.step(&driver, &rebase_dir)?; // Edit pauses.
        assert!(state.stopped_sha.is_some());

        // Abort during a pause.
        state.abort(&driver, &rebase_dir)?;

        assert_eq!(
            head_oid(&fix.workdir),
            fix.oid("D"),
            "HEAD should be restored after aborting a paused rebase"
        );
        assert!(!rebase_dir.exists());

        Ok(())
    }
}

mod conflict {
    use super::*;

    #[test]
    fn conflict_during_pick_returns_error() -> Result<(), Box<dyn std::error::Error>> {
        // Use the basic fixture where D modifies file.txt which conflicts with B.
        let fix = RebaseFixture::new_basic();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        // Detach at C (main tip). Rebasing D onto C will conflict because
        // D changes file.txt which B already changed.
        fix.detach_head_to("C");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("C"),
            orig_head: fix.oid("F"),
            interactive: true,
            todo: TodoList {
                operations: vec![Operation::Pick {
                    commit: fix.prefix("D"),
                    summary: "D: update file.txt on feature".into(),
                }]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 1,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err(), "picking a conflicting commit should error");
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("cherry-pick")
                || err_str.contains("Cherry-pick")
                || err_str.contains("conflict")
                || err_str.contains("Conflict"),
            "error should mention cherry-pick or conflict: {err_str}"
        );

        Ok(())
    }
}

mod state_persistence {
    use super::*;

    #[test]
    fn state_is_persisted_after_each_step() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // After step 1, read from disk and verify.
        state.step(&driver, &rebase_dir)?;
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1)?;
        assert_eq!(on_disk.todo.operations.len(), 1, "one op remaining on disk");
        assert_eq!(on_disk.done.operations.len(), 1, "one op done on disk");
        assert_eq!(on_disk.current_step, 1);
        assert_eq!(on_disk.head_name.to_string(), "refs/heads/feature");
        assert_eq!(on_disk.onto, fix.oid("B"));
        assert_eq!(on_disk.orig_head, fix.oid("D"));
        assert!(on_disk.interactive);

        // After step 2.
        state.step(&driver, &rebase_dir)?;
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1)?;
        assert_eq!(on_disk.todo.operations.len(), 0);
        assert_eq!(on_disk.done.operations.len(), 2);
        assert_eq!(on_disk.current_step, 2);

        Ok(())
    }

    #[test]
    fn stopped_sha_persisted_and_cleaned_on_continue() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Edit {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Edit pauses.
        state.step(&driver, &rebase_dir)?;

        // stopped-sha should be on disk.
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1)?;
        assert!(on_disk.stopped_sha.is_some(), "stopped_sha should be persisted on disk");
        assert!(rebase_dir.join("stopped-sha").exists(), "stopped-sha file should exist");

        // Continue.
        state.continue_rebase(&driver, &rebase_dir)?;

        // stopped-sha should be cleaned up.
        assert!(
            !rebase_dir.join("stopped-sha").exists(),
            "stopped-sha file should be removed after continue"
        );
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1)?;
        assert!(on_disk.stopped_sha.is_none());

        Ok(())
    }
}

mod mixed_operations {
    use super::*;

    #[test]
    fn pick_drop_squash_edit_sequence() -> Result<(), Box<dyn std::error::Error>> {
        // Test a complex sequence: pick B, drop C, pick D (remaining becomes squash-like).
        let fix = RebaseFixture::new_squash();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("A");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("A"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("B"),
                        summary: "B: add b.txt".into(),
                    },
                    Operation::Drop {
                        commit: fix.prefix("C"),
                        summary: "C: add c.txt".into(),
                    },
                    Operation::Edit {
                        commit: fix.prefix("D"),
                        summary: "D: add d.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 3,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Pick B.
        let r = state.step(&driver, &rebase_dir)?;
        assert!(matches!(r, StepOutcome::Applied { .. }));
        assert!(fix.workdir.join("b.txt").exists());

        // Drop C (no cherry-pick, just skip).
        let r = state.step(&driver, &rebase_dir)?;
        assert_eq!(r, StepOutcome::Skipped);
        assert!(
            !fix.workdir.join("c.txt").exists(),
            "c.txt should not exist (C was dropped)"
        );

        // Edit D (applies then pauses).
        let r = state.step(&driver, &rebase_dir)?;
        assert!(matches!(r, StepOutcome::Paused { commit_id: Some(_), .. }));
        assert!(fix.workdir.join("d.txt").exists());

        // Continue (no more ops, should be done).
        let r = state.continue_rebase(&driver, &rebase_dir)?;
        assert_eq!(r, StepOutcome::Done);

        // Final state: A + B' + D' = 3 commits.
        assert_eq!(commit_count(&fix.workdir), 3);
        assert!(fix.workdir.join("b.txt").exists());
        assert!(!fix.workdir.join("c.txt").exists());
        assert!(fix.workdir.join("d.txt").exists());

        Ok(())
    }

    #[test]
    fn noop_is_skipped_in_real_repo() -> Result<(), Box<dyn std::error::Error>> {
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");
        let head_before = head_oid(&fix.workdir);

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Noop,
                    Operation::Pick {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Noop should skip.
        let r = state.step(&driver, &rebase_dir)?;
        assert_eq!(r, StepOutcome::Skipped);
        // HEAD should be unchanged after noop.
        assert_eq!(head_oid(&fix.workdir), head_before);

        // Then pick C.
        let r = state.step(&driver, &rebase_dir)?;
        assert!(matches!(r, StepOutcome::Applied { .. }));
        assert!(fix.workdir.join("feature1.txt").exists());

        Ok(())
    }
}

mod reword {
    use super::*;

    #[test]
    fn reword_applies_commit() -> Result<(), Box<dyn std::error::Error>> {
        // Reword currently applies the commit with the original message.
        // A full implementation would open an editor, but the driver
        // treats it like a pick.
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![Operation::Reword {
                    commit: fix.prefix("C"),
                    summary: "C: add feature1.txt".into(),
                }]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 1,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        let outcome = state.step(&driver, &rebase_dir)?;
        assert!(
            matches!(outcome, StepOutcome::Paused { .. }),
            "reword should produce Paused so the caller can amend the message"
        );

        // The commit message should be preserved (reword without editor keeps original).
        let msg = commit_message(&fix.workdir, "HEAD");
        assert!(
            msg.contains("C: add feature1.txt"),
            "reword should preserve the original message: {msg}"
        );

        assert!(fix.workdir.join("feature1.txt").exists());

        Ok(())
    }
}

mod complete_rebase_flow {
    use super::*;

    #[test]
    fn full_rebase_flow_pick_all_then_cleanup() -> Result<(), Box<dyn std::error::Error>> {
        // Simulate a complete rebase lifecycle:
        // 1. Create state, write to disk.
        // 2. Step through all operations.
        // 3. Clean up rebase state.
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Write initial state.
        state.write_to(&rebase_dir)?;
        assert!(rebase_dir.exists());

        // Step through all operations.
        loop {
            let outcome = state.step(&driver, &rebase_dir)?;
            match outcome {
                StepOutcome::Done => break,
                StepOutcome::Applied { .. } | StepOutcome::Skipped => continue,
                StepOutcome::Paused { .. } => {
                    panic!("unexpected pause in pick-only rebase");
                }
            }
        }

        // Verify final state.
        assert_eq!(state.done.operations.len(), 2);
        assert!(state.todo.operations.is_empty());

        // Clean up rebase state (simulating what git rebase does on completion).
        MergeState::remove(&rebase_dir)?;
        assert!(!rebase_dir.exists(), "rebase-merge should be cleaned up");

        // Verify the repository has the correct commit history.
        let log = git(&fix.workdir, &["log", "--format=%s", "--reverse"]);
        let subjects: Vec<&str> = log.lines().collect();
        assert_eq!(subjects.len(), 4);
        assert_eq!(subjects[0], "A: initial commit");
        assert_eq!(subjects[1], "B: add main_file.txt");
        assert_eq!(subjects[2], "C: add feature1.txt");
        assert_eq!(subjects[3], "D: add feature2.txt");

        // All files should be present.
        assert!(fix.workdir.join("base.txt").exists());
        assert!(fix.workdir.join("main_file.txt").exists());
        assert!(fix.workdir.join("feature1.txt").exists());
        assert!(fix.workdir.join("feature2.txt").exists());

        Ok(())
    }

    #[test]
    fn rebase_state_resumable_from_disk() -> Result<(), Box<dyn std::error::Error>> {
        // Simulate crash recovery: write state, then read it back and continue.
        let fix = RebaseFixture::new_no_conflict();
        let driver = fix.driver();
        let rebase_dir = fix.rebase_dir();

        fix.detach_head_to("B");

        let mut state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: fix.oid("B"),
            orig_head: fix.oid("D"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Pick {
                        commit: fix.prefix("C"),
                        summary: "C: add feature1.txt".into(),
                    },
                    Operation::Pick {
                        commit: fix.prefix("D"),
                        summary: "D: add feature2.txt".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        // Apply first pick.
        state.step(&driver, &rebase_dir)?;

        // "Crash" -- read state back from disk.
        let mut resumed = MergeState::read_from(&rebase_dir, Kind::Sha1)?;
        assert_eq!(resumed.todo.operations.len(), 1);
        assert_eq!(resumed.done.operations.len(), 1);

        // Apply second pick from the resumed state.
        let outcome = resumed.step(&driver, &rebase_dir)?;
        assert!(matches!(outcome, StepOutcome::Applied { .. }));

        // Done.
        let outcome = resumed.step(&driver, &rebase_dir)?;
        assert_eq!(outcome, StepOutcome::Done);

        // Verify.
        assert!(fix.workdir.join("feature1.txt").exists());
        assert!(fix.workdir.join("feature2.txt").exists());
        assert_eq!(commit_count(&fix.workdir), 4);

        Ok(())
    }
}
