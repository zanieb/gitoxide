#[cfg(feature = "worktree-mutation")]
mod reset {
    use crate::util::repo_rw;
    use gix::repository::reset::ResetMode;

    fn repo_rw_reset() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
        repo_rw("make_reset_repo.sh")
    }

    fn commit_ids(repo: &gix::Repository) -> crate::Result<Vec<gix_hash::ObjectId>> {
        let mut ids = Vec::new();
        let mut current_id = repo.head_commit()?.id;
        loop {
            ids.push(current_id);
            let commit = repo.find_object(current_id)?.into_commit();
            let parent_id = commit.parent_ids().next();
            match parent_id {
                Some(pid) => {
                    current_id = pid.detach();
                }
                None => break,
            }
        }
        ids.reverse(); // oldest first: [c1, c2, c3]
        Ok(ids)
    }

    #[test]
    fn soft_reset_moves_head_only() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c3 = commits[2]; // current HEAD
        let c1 = commits[0];

        // Soft reset to c1
        repo.reset(c1, ResetMode::Soft)?;

        // HEAD should now point to c1
        let head_id = repo.head_id()?;
        assert_eq!(head_id.detach(), c1, "HEAD should point to c1 after soft reset");

        // ORIG_HEAD should be set to c3 (previous HEAD)
        let orig_head = repo.find_reference("ORIG_HEAD")?;
        assert_eq!(orig_head.id().detach(), c3, "ORIG_HEAD should be the old HEAD");

        // The index should still reflect c3 (not modified by soft reset)
        // so the working tree files should be unchanged
        let workdir = repo.workdir().expect("has worktree");
        assert!(
            workdir.join("second").exists(),
            "second file should still exist in worktree"
        );
        assert!(
            workdir.join("first").exists(),
            "first file should still exist in worktree"
        );

        // Read the first file - should still have the c3 content
        let content = std::fs::read_to_string(workdir.join("first"))?;
        assert_eq!(
            content, "modified first\n",
            "worktree content unchanged after soft reset"
        );

        Ok(())
    }

    #[test]
    fn mixed_reset_moves_head_and_resets_index() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        // Mixed reset to c1
        repo.reset(c1, ResetMode::Mixed)?;

        // HEAD should point to c1
        let head_id = repo.head_id()?;
        assert_eq!(head_id.detach(), c1, "HEAD should point to c1 after mixed reset");

        // The index should match c1's tree (only 'first' with original content)
        let index = repo.open_index()?;
        let entry_names: Vec<_> = index.entries().iter().map(|e| e.path(&index).to_string()).collect();
        assert_eq!(entry_names, vec!["first"], "index should only contain 'first' from c1");

        // But the working tree should still have the old files
        let workdir = repo.workdir().expect("has worktree");
        assert!(
            workdir.join("second").exists(),
            "second file should still be in worktree"
        );
        assert!(workdir.join("first").exists(), "first file should still be in worktree");

        Ok(())
    }

    #[test]
    fn hard_reset_moves_head_resets_index_and_worktree() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        // Hard reset to c1
        repo.reset(c1, ResetMode::Hard)?;

        // HEAD should point to c1
        let head_id = repo.head_id()?;
        assert_eq!(head_id.detach(), c1, "HEAD should point to c1 after hard reset");

        // The index should match c1's tree
        let index = repo.open_index()?;
        let entry_names: Vec<_> = index.entries().iter().map(|e| e.path(&index).to_string()).collect();
        assert_eq!(entry_names, vec!["first"], "index should only contain 'first' from c1");

        // The working tree should also match c1
        let workdir = repo.workdir().expect("has worktree");
        let content = std::fs::read_to_string(workdir.join("first"))?;
        assert_eq!(content, "first file\n", "worktree file should match c1 content");

        // 'second' should not exist in the worktree anymore after hard reset to c1
        assert!(
            !workdir.join("second").exists(),
            "'second' should be removed from worktree after hard reset to c1"
        );

        Ok(())
    }

    #[test]
    fn reset_to_head_is_noop_for_soft() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head_before = repo.head_id()?.detach();

        repo.reset(head_before, ResetMode::Soft)?;

        let head_after = repo.head_id()?.detach();
        assert_eq!(head_before, head_after, "soft reset to HEAD should be a no-op");

        Ok(())
    }

    #[test]
    fn reset_sets_orig_head() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c3 = commits[2];
        let c2 = commits[1];

        repo.reset(c2, ResetMode::Soft)?;

        let orig_head = repo.find_reference("ORIG_HEAD")?;
        assert_eq!(
            orig_head.id().detach(),
            c3,
            "ORIG_HEAD should be set to previous HEAD (c3)"
        );

        // Reset again - ORIG_HEAD should be updated to c2
        repo.reset(commits[0], ResetMode::Soft)?;
        let orig_head = repo.find_reference("ORIG_HEAD")?;
        assert_eq!(
            orig_head.id().detach(),
            c2,
            "ORIG_HEAD should be updated to c2 after second reset"
        );

        Ok(())
    }

    #[test]
    fn hard_reset_clears_merge_state() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head = repo.head_id()?.detach();

        // Simulate merge state
        let git_dir = repo.path();
        std::fs::write(git_dir.join("MERGE_HEAD"), "abc\n")?;
        std::fs::write(git_dir.join("MERGE_MSG"), "merge message\n")?;

        assert!(git_dir.join("MERGE_HEAD").exists());

        repo.reset(head, ResetMode::Hard)?;

        assert!(
            !git_dir.join("MERGE_HEAD").exists(),
            "MERGE_HEAD should be removed after hard reset"
        );
        assert!(
            !git_dir.join("MERGE_MSG").exists(),
            "MERGE_MSG should be removed after hard reset"
        );

        Ok(())
    }

    #[test]
    fn soft_reset_during_merge_fails() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head = repo.head_id()?.detach();

        // Create MERGE_HEAD to simulate in-progress merge
        std::fs::write(repo.path().join("MERGE_HEAD"), "abc\n")?;

        let result = repo.reset(head, ResetMode::Soft);
        assert!(result.is_err(), "soft reset during merge should fail");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("soft reset in the middle of a merge"),
            "error message should mention merge: {}",
            err
        );

        Ok(())
    }

    /// Ported from t7102: 'resetting to HEAD with no changes should succeed and do nothing'
    #[test]
    fn reset_to_head_is_noop_for_all_modes() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head_before = repo.head_id()?.detach();
        let workdir = repo.workdir().expect("has worktree").to_owned();

        // Soft reset to HEAD
        repo.reset(head_before, ResetMode::Soft)?;
        assert_eq!(repo.head_id()?.detach(), head_before);

        // Mixed reset to HEAD
        repo.reset(head_before, ResetMode::Mixed)?;
        assert_eq!(repo.head_id()?.detach(), head_before);

        // Hard reset to HEAD
        repo.reset(head_before, ResetMode::Hard)?;
        assert_eq!(repo.head_id()?.detach(), head_before);

        // Files should still be intact
        assert!(workdir.join("first").exists());
        assert!(workdir.join("second").exists());

        Ok(())
    }

    /// Ported from t7102: hard reset clears all merge state files including SQUASH_MSG
    #[test]
    fn hard_reset_clears_squash_msg() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head = repo.head_id()?.detach();

        let git_dir = repo.path();
        std::fs::write(git_dir.join("SQUASH_MSG"), "squash message\n")?;
        std::fs::write(git_dir.join("MERGE_MODE"), "")?;
        assert!(git_dir.join("SQUASH_MSG").exists());
        assert!(git_dir.join("MERGE_MODE").exists());

        repo.reset(head, ResetMode::Hard)?;

        assert!(
            !git_dir.join("SQUASH_MSG").exists(),
            "SQUASH_MSG should be removed after hard reset"
        );
        assert!(
            !git_dir.join("MERGE_MODE").exists(),
            "MERGE_MODE should be removed after hard reset"
        );

        Ok(())
    }

    /// Ported from t7102: mixed/hard reset during merge is allowed (only soft is blocked)
    #[test]
    fn mixed_reset_during_merge_succeeds() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head = repo.head_id()?.detach();

        // Create MERGE_HEAD to simulate in-progress merge
        std::fs::write(repo.path().join("MERGE_HEAD"), "abc\n")?;

        // Mixed reset should succeed (unlike soft)
        let result = repo.reset(head, ResetMode::Mixed);
        assert!(result.is_ok(), "mixed reset during merge should succeed");

        // MERGE_HEAD should be cleaned up
        assert!(
            !repo.path().join("MERGE_HEAD").exists(),
            "MERGE_HEAD should be removed after mixed reset"
        );

        Ok(())
    }

    /// Ported from t7102: hard reset should undo commits permanently
    /// Tests resetting backwards by multiple commits (HEAD~2 equivalent)
    #[test]
    fn hard_reset_backwards_multiple_commits() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];
        let c3 = commits[2];

        // Hard reset from c3 to c1 (going back 2 commits)
        repo.reset(c1, ResetMode::Hard)?;

        // HEAD should be at c1
        assert_eq!(repo.head_id()?.detach(), c1);

        // ORIG_HEAD should be c3
        let orig_head = repo.find_reference("ORIG_HEAD")?;
        assert_eq!(orig_head.id().detach(), c3);

        // Index should only have 'first'
        let index = repo.open_index()?;
        let entry_names: Vec<_> = index.entries().iter().map(|e| e.path(&index).to_string()).collect();
        assert_eq!(entry_names, vec!["first"]);

        // Worktree 'first' should have c1 content
        let workdir = repo.workdir().expect("has worktree");
        let content = std::fs::read_to_string(workdir.join("first"))?;
        assert_eq!(content, "first file\n");

        // 'second' should be removed from worktree (it was added in c2)
        assert!(
            !workdir.join("second").exists(),
            "'second' should be removed from worktree after hard reset to c1"
        );

        Ok(())
    }

    /// Ported from t7102: `git reset HEAD -- <path>` resets a single file in the index
    /// to match the given commit without moving HEAD or modifying the worktree.
    #[test]
    fn reset_paths_resets_single_file_in_index() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        // At HEAD (c3), the index has: first (modified), second
        // reset_paths to c1 for "first" should revert only "first" in the index to c1's version

        // HEAD should NOT move
        let head_before = repo.head_id()?.detach();
        repo.reset_paths(c1, ["first"])?;
        let head_after = repo.head_id()?.detach();
        assert_eq!(head_before, head_after, "reset_paths should not move HEAD");

        // The index entry for "first" should have c1's blob
        let index = repo.open_index()?;
        let first_entry = index
            .entries()
            .iter()
            .find(|e| e.path(&index).to_string() == "first")
            .expect("first should still be in the index");

        // Read c1's tree to get the expected blob id for "first"
        let c1_commit = repo.find_object(c1)?.try_into_commit().unwrap();
        let c1_tree_id = c1_commit.tree_id().unwrap();
        let c1_tree = repo.find_object(c1_tree_id)?.try_into_tree().unwrap();
        let c1_first_ref = c1_tree
            .iter()
            .find(|e| e.as_ref().expect("valid").filename() == "first")
            .unwrap()?;
        let c1_first_id = c1_first_ref.oid();

        assert_eq!(
            first_entry.id, c1_first_id,
            "index 'first' entry should match c1's tree blob"
        );

        // "second" should still be in the index (not affected)
        let second_exists = index.entries().iter().any(|e| e.path(&index).to_string() == "second");
        assert!(second_exists, "'second' should still be in the index");

        // Worktree should be unchanged
        let workdir = repo.workdir().expect("has worktree");
        let content = std::fs::read_to_string(workdir.join("first"))?;
        assert_eq!(content, "modified first\n", "worktree should not change");

        Ok(())
    }

    /// Ported from t7102: `git reset HEAD -- <path>` removes a file from the index
    /// if the path doesn't exist in the target tree.
    #[test]
    fn reset_paths_removes_file_not_in_target() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        // c1 only has "first", not "second". Resetting "second" against c1
        // should remove "second" from the index.
        repo.reset_paths(c1, ["second"])?;

        let index = repo.open_index()?;
        let entry_names: Vec<_> = index.entries().iter().map(|e| e.path(&index).to_string()).collect();
        assert!(
            !entry_names.contains(&"second".to_string()),
            "'second' should be removed from index since it doesn't exist in c1"
        );
        assert!(
            entry_names.contains(&"first".to_string()),
            "'first' should still be in the index"
        );

        // Worktree should be unchanged
        let workdir = repo.workdir().expect("has worktree");
        assert!(workdir.join("second").exists(), "worktree 'second' should still exist");

        Ok(())
    }

    /// Ported from t7102: reset_paths with multiple paths resets all specified paths.
    #[test]
    fn reset_paths_multiple_paths() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        // Reset both "first" and "second" to c1
        // "first" should get c1's version, "second" should be removed
        repo.reset_paths(c1, ["first", "second"])?;

        let index = repo.open_index()?;
        let entry_names: Vec<_> = index.entries().iter().map(|e| e.path(&index).to_string()).collect();

        assert_eq!(entry_names, vec!["first"], "only 'first' should remain in index");

        Ok(())
    }

    /// Ported from t7102: reset_paths does not move HEAD.
    #[test]
    fn reset_paths_does_not_set_orig_head() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        // Ensure ORIG_HEAD does not exist before
        let orig_head_before = repo.try_find_reference("ORIG_HEAD")?;

        repo.reset_paths(c1, ["first"])?;

        // ORIG_HEAD should not be created by reset_paths
        let orig_head_after = repo.try_find_reference("ORIG_HEAD")?;
        assert_eq!(
            orig_head_before.is_some(),
            orig_head_after.is_some(),
            "reset_paths should not create or modify ORIG_HEAD"
        );

        Ok(())
    }

    /// Verify reset_paths with a path that doesn't exist in target or current index.
    #[test]
    fn reset_paths_nonexistent_path_is_noop() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head = repo.head_id()?.detach();

        // Get index state before
        let index_before = repo.open_index()?;
        let names_before: Vec<_> = index_before
            .entries()
            .iter()
            .map(|e| e.path(&index_before).to_string())
            .collect();

        // Reset a nonexistent path
        repo.reset_paths(head, ["nonexistent-file"])?;

        // Index should be unchanged
        let index_after = repo.open_index()?;
        let names_after: Vec<_> = index_after
            .entries()
            .iter()
            .map(|e| e.path(&index_after).to_string())
            .collect();
        assert_eq!(
            names_before, names_after,
            "resetting nonexistent path should be a no-op"
        );

        Ok(())
    }

    /// Test reset with detached HEAD: HEAD should be updated directly (not through a branch).
    #[test]
    fn reset_with_detached_head() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c2 = commits[1];
        let c1 = commits[0];

        // Detach HEAD by pointing it directly to c2
        repo.edit_reference(gix_ref::transaction::RefEdit {
            change: gix_ref::transaction::Change::Update {
                log: gix_ref::transaction::LogChange {
                    mode: gix_ref::transaction::RefLog::AndReference,
                    force_create_reflog: false,
                    message: "detach HEAD".into(),
                },
                expected: gix_ref::transaction::PreviousValue::Any,
                new: gix_ref::Target::Object(c2),
            },
            name: "HEAD".try_into()?,
            deref: false,
        })?;

        // Verify HEAD is detached
        let head = repo.head()?;
        assert!(
            matches!(head.kind, gix::head::Kind::Detached { .. }),
            "HEAD should be detached"
        );

        // Now reset to c1
        repo.reset(c1, ResetMode::Hard)?;

        // HEAD should now point to c1
        assert_eq!(repo.head_id()?.detach(), c1, "detached HEAD should point to c1");

        // ORIG_HEAD should be c2
        let orig_head = repo.find_reference("ORIG_HEAD")?;
        assert_eq!(orig_head.id().detach(), c2, "ORIG_HEAD should be c2");

        Ok(())
    }

    /// Test that hard reset clears CHERRY_PICK_HEAD and REVERT_HEAD state files.
    #[test]
    fn hard_reset_clears_cherry_pick_and_revert_state() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head = repo.head_id()?.detach();

        let git_dir = repo.path();
        std::fs::write(git_dir.join("CHERRY_PICK_HEAD"), "abc\n")?;
        std::fs::write(git_dir.join("REVERT_HEAD"), "def\n")?;

        assert!(git_dir.join("CHERRY_PICK_HEAD").exists());
        assert!(git_dir.join("REVERT_HEAD").exists());

        repo.reset(head, ResetMode::Hard)?;

        assert!(
            !git_dir.join("CHERRY_PICK_HEAD").exists(),
            "CHERRY_PICK_HEAD should be removed after hard reset"
        );
        assert!(
            !git_dir.join("REVERT_HEAD").exists(),
            "REVERT_HEAD should be removed after hard reset"
        );

        Ok(())
    }

    /// Test that mixed reset also clears state files (same as hard reset).
    #[test]
    fn mixed_reset_clears_state_files() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head = repo.head_id()?.detach();

        let git_dir = repo.path();
        std::fs::write(git_dir.join("CHERRY_PICK_HEAD"), "abc\n")?;
        std::fs::write(git_dir.join("MERGE_HEAD"), "def\n")?;
        std::fs::write(git_dir.join("AUTO_MERGE"), "ghi\n")?;

        repo.reset(head, ResetMode::Mixed)?;

        assert!(
            !git_dir.join("CHERRY_PICK_HEAD").exists(),
            "CHERRY_PICK_HEAD should be removed after mixed reset"
        );
        assert!(
            !git_dir.join("MERGE_HEAD").exists(),
            "MERGE_HEAD should be removed after mixed reset"
        );
        assert!(
            !git_dir.join("AUTO_MERGE").exists(),
            "AUTO_MERGE should be removed after mixed reset"
        );

        Ok(())
    }

    /// Ported from t7102: forward reset (resetting from an earlier commit to a later one).
    /// After hard-resetting back, hard-reset forward to restore the original state.
    #[test]
    fn hard_reset_forward_restores_state() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];
        let c3 = commits[2];

        // Hard reset back to c1.
        repo.reset(c1, ResetMode::Hard)?;
        assert_eq!(repo.head_id()?.detach(), c1);

        // Now hard reset forward to c3.
        repo.reset(c3, ResetMode::Hard)?;
        assert_eq!(repo.head_id()?.detach(), c3, "HEAD should be at c3 after forward reset");

        // Worktree should have c3 content.
        let workdir = repo.workdir().expect("has worktree");
        assert!(
            workdir.join("second").exists(),
            "'second' should exist after forward reset to c3"
        );
        let content = std::fs::read_to_string(workdir.join("first"))?;
        assert_eq!(content, "modified first\n", "first should have c3 content");

        // Index should reflect c3's tree.
        let index = repo.open_index()?;
        let entry_names: Vec<_> = index.entries().iter().map(|e| e.path(&index).to_string()).collect();
        assert!(entry_names.contains(&"first".to_string()));
        assert!(entry_names.contains(&"second".to_string()));

        Ok(())
    }

    /// Ported from t7102: soft reset preserves index content when going forward.
    #[test]
    fn soft_reset_forward_preserves_index() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];
        let c3 = commits[2];

        // Soft reset back to c1.
        repo.reset(c1, ResetMode::Soft)?;

        // Index still reflects c3 (soft doesn't touch index).
        let index = repo.open_index()?;
        let entry_names: Vec<_> = index.entries().iter().map(|e| e.path(&index).to_string()).collect();
        assert!(
            entry_names.contains(&"second".to_string()),
            "index should still have 'second'"
        );

        // Now soft reset forward to c3.
        repo.reset(c3, ResetMode::Soft)?;
        assert_eq!(repo.head_id()?.detach(), c3);

        // Index should still be the same (soft reset only moves HEAD).
        let index2 = repo.open_index()?;
        let entry_names2: Vec<_> = index2.entries().iter().map(|e| e.path(&index2).to_string()).collect();
        assert_eq!(entry_names, entry_names2, "soft reset forward should not change index");

        Ok(())
    }

    /// Ported from t7102: hard reset clears sequencer state directory.
    #[test]
    fn hard_reset_clears_sequencer_dir() -> crate::Result {
        let (repo, _tmp) = repo_rw_reset()?;
        let head = repo.head_id()?.detach();

        // Simulate sequencer state (left by cherry-pick/revert --continue).
        let sequencer_dir = repo.path().join("sequencer");
        std::fs::create_dir_all(&sequencer_dir)?;
        std::fs::write(sequencer_dir.join("todo"), "pick abc\n")?;
        assert!(sequencer_dir.exists());

        repo.reset(head, ResetMode::Hard)?;

        assert!(
            !sequencer_dir.exists(),
            "sequencer directory should be removed after hard reset"
        );

        Ok(())
    }

    // ========================================================================
    // C Git interop tests
    // Ported from t7102-reset.sh: verify gix reset results with C Git commands
    // ========================================================================

    /// Helper to run a C Git command in the worktree and return stdout.
    fn git_output(workdir: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(workdir)
            .output()
            .expect("git must be on PATH for interop tests");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Ported from t7102: After hard reset, C Git's `git log` should show the correct HEAD.
    /// Reset with gix, verify with `git log --oneline`.
    #[test]
    fn interop_hard_reset_verified_by_git_log() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        repo.reset(c1, ResetMode::Hard)?;

        let head_hex = git_output(&workdir, &["rev-parse", "HEAD"]);
        assert_eq!(head_hex, c1.to_string(), "C Git HEAD should match gix reset target");

        let log = git_output(&workdir, &["log", "--oneline"]);
        let log_lines: Vec<&str> = log.lines().collect();
        assert_eq!(
            log_lines.len(),
            1,
            "after reset to c1, git log should show 1 commit, got: {log}"
        );

        Ok(())
    }

    /// Ported from t7102: After hard reset, `git status` should show clean worktree.
    #[test]
    fn interop_hard_reset_clean_status() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        repo.reset(c1, ResetMode::Hard)?;

        let status = git_output(&workdir, &["status", "--porcelain"]);
        assert!(
            status.is_empty(),
            "worktree should be clean after hard reset, got: {status}"
        );

        Ok(())
    }

    /// Ported from t7102: After soft reset, `git diff --cached` should show staged changes.
    #[test]
    fn interop_soft_reset_shows_cached_diff() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        repo.reset(c1, ResetMode::Soft)?;

        let head_hex = git_output(&workdir, &["rev-parse", "HEAD"]);
        assert_eq!(head_hex, c1.to_string());

        let cached_diff = git_output(&workdir, &["diff", "--cached", "--name-only"]);
        assert!(
            !cached_diff.is_empty(),
            "soft reset should leave staged changes visible to C Git"
        );
        assert!(
            cached_diff.contains("second"),
            "staged diff should show 'second': {cached_diff}"
        );

        Ok(())
    }

    /// Ported from t7102: After mixed reset, `git diff` shows unstaged changes and
    /// `git diff --cached` is empty.
    #[test]
    fn interop_mixed_reset_shows_unstaged_diff() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        repo.reset(c1, ResetMode::Mixed)?;

        let head_hex = git_output(&workdir, &["rev-parse", "HEAD"]);
        assert_eq!(head_hex, c1.to_string());

        let cached = git_output(&workdir, &["diff", "--cached", "--name-only"]);
        assert!(
            cached.is_empty(),
            "mixed reset should have no staged changes, got: {cached}"
        );

        let unstaged = git_output(&workdir, &["diff", "--name-only"]);
        assert!(
            unstaged.contains("first"),
            "mixed reset should show unstaged 'first' diff: {unstaged}"
        );

        Ok(())
    }

    /// Ported from t7102: After reset, the branch reflog should record the reset operation.
    /// gix writes the reflog entry to the branch (refs/heads/main), not HEAD directly,
    /// since HEAD is symbolic.
    #[test]
    fn interop_reflog_records_reset() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        repo.reset(c1, ResetMode::Hard)?;

        // gix writes the reflog to the branch ref (refs/heads/main) when HEAD is symbolic.
        // Check the branch reflog for the reset entry.
        let reflog = git_output(&workdir, &["reflog", "show", "--format=%gs", "refs/heads/main"]);
        let lines: Vec<&str> = reflog.lines().collect();
        assert!(!lines.is_empty(), "branch reflog should have entries after reset");
        // The most recent entry should be the reset
        assert!(
            lines[0].to_lowercase().contains("reset"),
            "most recent branch reflog entry should reference reset: {:?}",
            lines[0]
        );

        Ok(())
    }

    /// Ported from t7102: ORIG_HEAD after reset should be visible to C Git.
    #[test]
    fn interop_orig_head_visible_to_c_git() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let commits = commit_ids(&repo)?;
        let c4 = *commits.last().expect("at least one commit");
        let c1 = commits[0];

        repo.reset(c1, ResetMode::Hard)?;

        let orig_head_hex = git_output(&workdir, &["rev-parse", "ORIG_HEAD"]);
        assert_eq!(
            orig_head_hex,
            c4.to_string(),
            "C Git ORIG_HEAD should match the pre-reset HEAD"
        );

        Ok(())
    }

    /// Ported from t7102: Detached HEAD reset, verified by C Git.
    #[test]
    fn interop_detached_head_reset() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let commits = commit_ids(&repo)?;
        let c2 = commits[1];
        let c1 = commits[0];

        // Detach HEAD at c2
        repo.edit_reference(gix_ref::transaction::RefEdit {
            change: gix_ref::transaction::Change::Update {
                log: gix_ref::transaction::LogChange {
                    mode: gix_ref::transaction::RefLog::AndReference,
                    force_create_reflog: false,
                    message: "detach HEAD".into(),
                },
                expected: gix_ref::transaction::PreviousValue::Any,
                new: gix_ref::Target::Object(c2),
            },
            name: "HEAD".try_into()?,
            deref: false,
        })?;

        repo.reset(c1, ResetMode::Hard)?;

        let head_hex = git_output(&workdir, &["rev-parse", "HEAD"]);
        assert_eq!(head_hex, c1.to_string());

        let sym_result = std::process::Command::new("git")
            .args(["symbolic-ref", "HEAD"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH");
        assert!(
            !sym_result.status.success(),
            "HEAD should still be detached after reset"
        );

        Ok(())
    }

    /// Ported from t7102: Hard reset clears merge state, verified by C Git status.
    #[test]
    fn interop_hard_reset_clears_merge_state_verified_by_status() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let head = repo.head_id()?.detach();

        let git_dir = repo.path();
        std::fs::write(git_dir.join("MERGE_HEAD"), format!("{}\n", head))?;
        std::fs::write(git_dir.join("MERGE_MSG"), "merge message\n")?;

        repo.reset(head, ResetMode::Hard)?;

        let status_output = std::process::Command::new("git")
            .args(["status"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH");
        let status_str = String::from_utf8_lossy(&status_output.stdout);
        assert!(
            !status_str.contains("merging") && !status_str.contains("All conflicts fixed"),
            "C Git should not see merge state after hard reset: {status_str}"
        );

        Ok(())
    }

    /// Ported from t7102: Hard reset forward (from c1 to c4) restores all files,
    /// verified by C Git's `git ls-files`.
    #[test]
    fn interop_hard_reset_forward_verified_by_ls_files() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];
        let c4 = *commits.last().expect("at least 4 commits");

        repo.reset(c1, ResetMode::Hard)?;
        repo.reset(c4, ResetMode::Hard)?;

        let ls_files = git_output(&workdir, &["ls-files"]);
        let files: Vec<&str> = ls_files.lines().collect();
        assert!(files.contains(&"first"), "ls-files should contain 'first': {ls_files}");
        assert!(
            files.contains(&"second"),
            "ls-files should contain 'second': {ls_files}"
        );
        assert!(files.contains(&"third"), "ls-files should contain 'third': {ls_files}");

        let status = git_output(&workdir, &["status", "--porcelain"]);
        assert!(
            status.is_empty(),
            "status should be clean after forward reset: {status}"
        );

        Ok(())
    }

    /// Ported from t7102: path-specific reset verified by `git diff --cached`.
    #[test]
    fn interop_reset_paths_verified_by_cached_diff() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_interop_repo.sh")?;
        let workdir = tmp.path().to_owned();
        let commits = commit_ids(&repo)?;
        let c1 = commits[0];

        repo.reset_paths(c1, ["first"])?;

        let cached = git_output(&workdir, &["diff", "--cached", "--name-only"]);
        assert!(
            cached.contains("first"),
            "C Git cached diff should show 'first' after path reset: {cached}"
        );
        assert!(
            !cached.contains("second"),
            "'second' should not be in cached diff: {cached}"
        );
        assert!(
            !cached.contains("third"),
            "'third' should not be in cached diff: {cached}"
        );

        Ok(())
    }
}
