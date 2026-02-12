#[cfg(all(feature = "merge", feature = "worktree-mutation"))]
mod cherry_pick {
    use crate::util::repo_rw;
    use gix::repository::cherry_pick::Options;

    fn repo_cherry_pick() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
        repo_rw("make_cherry_pick_test_repo.sh")
    }

    /// Resolve the OID of a branch tip by name.
    fn branch_tip(repo: &gix::Repository, branch: &str) -> gix_hash::ObjectId {
        let full_name = format!("refs/heads/{branch}");
        repo.find_reference(&full_name)
            .unwrap_or_else(|_| panic!("reference {full_name} should exist"))
            .into_fully_peeled_id()
            .unwrap_or_else(|_| panic!("reference {full_name} should peel"))
            .detach()
    }

    #[test]
    fn cherry_pick_adds_new_file() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        // The "feature: add new_file.txt" commit adds a new file that doesn't exist on main.
        let feature_tip = branch_tip(&repo, "feature");
        // feature_tip is "feature: add new_file.txt"
        // Its parent is "feature: modify file.txt"

        let head_before = repo.head_id()?.detach();
        let outcome = repo.cherry_pick(feature_tip, Options::default())?;

        // A new commit should have been created.
        assert!(outcome.commit_id.is_some(), "commit should have been created");
        let new_commit_id = outcome.commit_id.unwrap();
        assert_ne!(new_commit_id, head_before, "new commit should differ from old HEAD");

        // HEAD should now point to the new commit.
        let head_after = repo.head_id()?.detach();
        assert_eq!(head_after, new_commit_id, "HEAD should point to the new commit");

        // The new commit's parent should be the old HEAD.
        let new_commit = repo.find_object(new_commit_id)?.into_commit();
        let parents: Vec<_> = new_commit.parent_ids().map(|id| id.detach()).collect();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0], head_before);

        // The new commit should have the same message as the picked commit.
        let new_msg = new_commit.decode()?.message.to_string();
        assert!(
            new_msg.contains("feature: add new_file.txt"),
            "commit message should be preserved: {new_msg}"
        );

        // The working tree should contain new_file.txt.
        let workdir = _tmp.path();
        let new_file_content = std::fs::read_to_string(workdir.join("new_file.txt"))?;
        assert_eq!(new_file_content, "new file content\n");

        Ok(())
    }

    #[test]
    fn cherry_pick_modifies_existing_file() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        // Get the "feature: modify file.txt" commit (parent of feature tip).
        let feature_tip = branch_tip(&repo, "feature");
        let feature_tip_commit = repo.find_object(feature_tip)?.into_commit();
        let feature_modify_id = feature_tip_commit
            .parent_ids()
            .next()
            .expect("feature tip has a parent")
            .detach();

        let outcome = repo.cherry_pick(feature_modify_id, Options::default())?;
        assert!(outcome.commit_id.is_some());

        // The working tree should have the cherry-picked content.
        let workdir = _tmp.path();
        let file_content = std::fs::read_to_string(workdir.join("file.txt"))?;
        assert_eq!(file_content, "feature change\n");

        // other.txt should still be there (from main's commit).
        assert!(workdir.join("other.txt").exists(), "other.txt should still exist");

        Ok(())
    }

    #[test]
    fn cherry_pick_no_commit() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let head_before = repo.head_id()?.detach();

        let feature_tip = branch_tip(&repo, "feature");
        let opts = Options {
            no_commit: true,
            ..Default::default()
        };
        let outcome = repo.cherry_pick(feature_tip, opts)?;

        // No commit should have been created.
        assert!(
            outcome.commit_id.is_none(),
            "no commit should be created with no_commit"
        );

        // HEAD should not have changed.
        let head_after = repo.head_id()?.detach();
        assert_eq!(head_after, head_before, "HEAD should be unchanged with no_commit");

        // But the working tree should have the cherry-picked file.
        let workdir = _tmp.path();
        assert!(
            workdir.join("new_file.txt").exists(),
            "new_file.txt should exist in worktree even with no_commit"
        );

        Ok(())
    }

    #[test]
    fn cherry_pick_preserves_original_author() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        let feature_tip = branch_tip(&repo, "feature");
        // Read original author before cherry-pick.
        let orig_commit = repo.find_object(feature_tip)?.into_commit();
        let orig_author = orig_commit.author()?.name.to_owned();

        let outcome = repo.cherry_pick(feature_tip, Options::default())?;
        let new_commit_id = outcome.commit_id.unwrap();

        // The new commit should preserve the original author.
        let new_commit = repo.find_object(new_commit_id)?.into_commit();
        assert_eq!(
            new_commit.author()?.name,
            orig_author,
            "author should be preserved from original commit"
        );

        Ok(())
    }

    #[test]
    fn revert_undoes_last_commit() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        // First cherry-pick the "add new_file.txt" commit.
        let feature_tip = branch_tip(&repo, "feature");
        let pick_outcome = repo.cherry_pick(feature_tip, Options::default())?;
        let picked_commit_id = pick_outcome.commit_id.unwrap();

        // Verify new_file.txt exists.
        let workdir = _tmp.path();
        assert!(workdir.join("new_file.txt").exists());

        // Now revert the cherry-picked commit.
        let revert_outcome = repo.revert(picked_commit_id, Options::default())?;
        assert!(revert_outcome.commit_id.is_some());
        let revert_commit_id = revert_outcome.commit_id.unwrap();

        // HEAD should now point to the revert commit.
        let head_after = repo.head_id()?.detach();
        assert_eq!(head_after, revert_commit_id);

        // new_file.txt should no longer exist in the working tree.
        assert!(
            !workdir.join("new_file.txt").exists(),
            "new_file.txt should be removed after revert"
        );

        // The revert commit message should mention "Revert".
        let revert_commit = repo.find_object(revert_commit_id)?.into_commit();
        let revert_msg = revert_commit.decode()?.message.to_string();
        assert!(
            revert_msg.starts_with("Revert \""),
            "revert message should start with 'Revert \"': {revert_msg}"
        );

        Ok(())
    }

    #[test]
    fn revert_with_no_commit() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        // Cherry-pick first.
        let feature_tip = branch_tip(&repo, "feature");
        let pick_outcome = repo.cherry_pick(feature_tip, Options::default())?;
        let picked_commit_id = pick_outcome.commit_id.unwrap();
        let head_after_pick = repo.head_id()?.detach();

        // Revert with no_commit.
        let opts = Options {
            no_commit: true,
            ..Default::default()
        };
        let revert_outcome = repo.revert(picked_commit_id, opts)?;
        assert!(revert_outcome.commit_id.is_none());

        // HEAD should not have changed.
        let head_after_revert = repo.head_id()?.detach();
        assert_eq!(
            head_after_revert, head_after_pick,
            "HEAD unchanged with no_commit revert"
        );

        // But the worktree should reflect the revert.
        let workdir = _tmp.path();
        assert!(
            !workdir.join("new_file.txt").exists(),
            "new_file.txt should be removed even with no_commit"
        );

        Ok(())
    }

    #[test]
    fn revert_message_includes_original_hash() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        // Cherry-pick and then revert.
        let feature_tip = branch_tip(&repo, "feature");
        let pick_outcome = repo.cherry_pick(feature_tip, Options::default())?;
        let picked_commit_id = pick_outcome.commit_id.unwrap();

        let revert_outcome = repo.revert(picked_commit_id, Options::default())?;
        let revert_commit_id = revert_outcome.commit_id.unwrap();

        let revert_commit = repo.find_object(revert_commit_id)?.into_commit();
        let revert_msg = revert_commit.decode()?.message.to_string();

        // The message should contain the reverted commit's hash.
        let picked_hex = picked_commit_id.to_string();
        assert!(
            revert_msg.contains(&picked_hex),
            "revert message should contain original commit hash {picked_hex}: {revert_msg}"
        );

        Ok(())
    }

    #[test]
    fn cherry_pick_and_revert_roundtrip() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        let head_tree_before = {
            let head = repo.head_id()?;
            repo.find_object(head)?.into_commit().tree_id()?.detach()
        };

        // Cherry-pick adds new_file.txt.
        let feature_tip = branch_tip(&repo, "feature");
        let pick_outcome = repo.cherry_pick(feature_tip, Options::default())?;

        // The tree should now differ (new_file.txt added).
        assert_ne!(pick_outcome.tree_id, head_tree_before);

        // Revert the cherry-pick.
        let picked_id = pick_outcome.commit_id.unwrap();
        let revert_outcome = repo.revert(picked_id, Options::default())?;

        // The tree should be back to the original.
        assert_eq!(
            revert_outcome.tree_id, head_tree_before,
            "after cherry-pick + revert roundtrip, tree should match original"
        );

        Ok(())
    }

    /// Ported from t3505: cherry-picking a commit that conflicts should return an error.
    #[test]
    fn cherry_pick_conflict_returns_error() -> crate::Result {
        let (repo, _tmp) = repo_rw("make_cherry_pick_conflict_repo.sh")?;

        let conflict_tip = branch_tip(&repo, "conflict-feature");
        let result = repo.cherry_pick(conflict_tip, Options::default());
        match result {
            Err(err) => {
                let err_msg = err.to_string();
                assert!(err_msg.contains("conflict"), "error should mention conflict: {err_msg}");
            }
            Ok(_) => panic!("cherry-pick with conflict should fail"),
        }

        Ok(())
    }

    /// Cherry-pick should set committer to the current user, distinct from the
    /// original commit's author (which is preserved).
    #[test]
    fn cherry_pick_sets_committer_from_config() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        let feature_tip = branch_tip(&repo, "feature");
        let orig_commit = repo.find_object(feature_tip)?.into_commit();
        let orig_author = orig_commit.author()?.name.to_owned();

        let outcome = repo.cherry_pick(feature_tip, Options::default())?;
        let new_commit_id = outcome.commit_id.unwrap();
        let new_commit = repo.find_object(new_commit_id)?.into_commit();

        // Author should be preserved from original.
        assert_eq!(new_commit.author()?.name, orig_author);

        // Committer should be set (non-empty) - it comes from the repo config.
        let committer_name = new_commit.committer()?.name.to_owned();
        assert!(!committer_name.is_empty(), "committer name should be set from config");

        Ok(())
    }

    /// Revert of a commit that modified (not added/deleted) a file should
    /// restore the file to its pre-modification state.
    #[test]
    fn revert_restores_modified_file_content() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let workdir = _tmp.path();

        // Get the "feature: modify file.txt" commit (parent of feature tip).
        let feature_tip = branch_tip(&repo, "feature");
        let feature_tip_commit = repo.find_object(feature_tip)?.into_commit();
        let feature_modify_id = feature_tip_commit
            .parent_ids()
            .next()
            .expect("feature tip has a parent")
            .detach();

        // Cherry-pick the modification.
        let pick_outcome = repo.cherry_pick(feature_modify_id, Options::default())?;
        let picked_id = pick_outcome.commit_id.unwrap();

        // file.txt should now have "feature change".
        let content = std::fs::read_to_string(workdir.join("file.txt"))?;
        assert_eq!(content, "feature change\n");

        // Revert the cherry-pick.
        repo.revert(picked_id, Options::default())?;

        // file.txt should be back to what it was before the cherry-pick.
        let content_after = std::fs::read_to_string(workdir.join("file.txt"))?;
        assert_eq!(
            content_after, "initial\n",
            "file.txt should be restored to pre-cherry-pick content"
        );

        Ok(())
    }

    /// Resolve a tag to an ObjectId.
    fn resolve_tag(repo: &gix::Repository, name: &str) -> gix_hash::ObjectId {
        repo.find_reference(name)
            .unwrap_or_else(|_| panic!("reference {name} should exist"))
            .into_fully_peeled_id()
            .unwrap_or_else(|_| panic!("reference {name} should peel"))
            .detach()
    }

    // === State file tests (CHERRY_PICK_HEAD, REVERT_HEAD, MERGE_MSG) ===

    #[test]
    fn cherry_pick_head_written_and_cleaned_up_on_success() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let git_dir = repo.git_dir().to_owned();

        let feature_tip = branch_tip(&repo, "feature");
        let outcome = repo.cherry_pick(feature_tip, Options::default())?;
        assert!(outcome.commit_id.is_some());

        // After successful commit, CHERRY_PICK_HEAD should be cleaned up.
        assert!(
            !git_dir.join("CHERRY_PICK_HEAD").exists(),
            "CHERRY_PICK_HEAD should be removed after successful cherry-pick"
        );
        assert!(
            !git_dir.join("MERGE_MSG").exists(),
            "MERGE_MSG should be removed after successful cherry-pick"
        );

        Ok(())
    }

    #[test]
    fn cherry_pick_head_persists_with_no_commit() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let git_dir = repo.git_dir().to_owned();

        let feature_tip = branch_tip(&repo, "feature");
        let opts = Options {
            no_commit: true,
            ..Default::default()
        };
        let outcome = repo.cherry_pick(feature_tip, opts)?;
        assert!(outcome.commit_id.is_none());

        // With no_commit, state files should persist for "cherry-pick in progress".
        let cherry_pick_head = std::fs::read_to_string(git_dir.join("CHERRY_PICK_HEAD"))?;
        assert_eq!(
            cherry_pick_head.trim(),
            feature_tip.to_string(),
            "CHERRY_PICK_HEAD should contain the picked commit OID"
        );
        assert!(
            git_dir.join("MERGE_MSG").exists(),
            "MERGE_MSG should exist with no_commit"
        );

        Ok(())
    }

    #[test]
    fn cherry_pick_merge_msg_contains_original_message() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let git_dir = repo.git_dir().to_owned();

        let feature_tip = branch_tip(&repo, "feature");
        // Use no_commit so we can inspect MERGE_MSG before cleanup.
        let opts = Options {
            no_commit: true,
            ..Default::default()
        };
        repo.cherry_pick(feature_tip, opts)?;

        let merge_msg = std::fs::read_to_string(git_dir.join("MERGE_MSG"))?;
        assert!(
            merge_msg.contains("feature: add new_file.txt"),
            "MERGE_MSG should contain the original commit message: {merge_msg}"
        );

        Ok(())
    }

    #[test]
    fn revert_head_written_and_cleaned_up_on_success() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let git_dir = repo.git_dir().to_owned();

        // Cherry-pick first, then revert.
        let feature_tip = branch_tip(&repo, "feature");
        let pick = repo.cherry_pick(feature_tip, Options::default())?;
        let picked_id = pick.commit_id.unwrap();

        let revert = repo.revert(picked_id, Options::default())?;
        assert!(revert.commit_id.is_some());

        // After successful revert commit, REVERT_HEAD should be cleaned up.
        assert!(
            !git_dir.join("REVERT_HEAD").exists(),
            "REVERT_HEAD should be removed after successful revert"
        );
        assert!(
            !git_dir.join("MERGE_MSG").exists(),
            "MERGE_MSG should be removed after successful revert"
        );

        Ok(())
    }

    #[test]
    fn revert_head_persists_with_no_commit() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let git_dir = repo.git_dir().to_owned();

        // Cherry-pick first, then revert with no_commit.
        let feature_tip = branch_tip(&repo, "feature");
        let pick = repo.cherry_pick(feature_tip, Options::default())?;
        let picked_id = pick.commit_id.unwrap();

        let opts = Options {
            no_commit: true,
            ..Default::default()
        };
        let revert = repo.revert(picked_id, opts)?;
        assert!(revert.commit_id.is_none());

        // With no_commit, REVERT_HEAD should persist.
        let revert_head = std::fs::read_to_string(git_dir.join("REVERT_HEAD"))?;
        assert_eq!(
            revert_head.trim(),
            picked_id.to_string(),
            "REVERT_HEAD should contain the reverted commit OID"
        );

        Ok(())
    }

    #[test]
    fn revert_merge_msg_format() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let git_dir = repo.git_dir().to_owned();

        // Cherry-pick first, then revert with no_commit so we can inspect MERGE_MSG.
        let feature_tip = branch_tip(&repo, "feature");
        let pick = repo.cherry_pick(feature_tip, Options::default())?;
        let picked_id = pick.commit_id.unwrap();

        let opts = Options {
            no_commit: true,
            ..Default::default()
        };
        repo.revert(picked_id, opts)?;

        let merge_msg = std::fs::read_to_string(git_dir.join("MERGE_MSG"))?;
        assert!(
            merge_msg.starts_with("Revert \""),
            "MERGE_MSG should start with 'Revert \"': {merge_msg}"
        );
        let picked_hex = picked_id.to_string();
        assert!(
            merge_msg.contains(&format!("This reverts commit {picked_hex}.")),
            "MERGE_MSG should contain the reverted commit hash: {merge_msg}"
        );

        Ok(())
    }

    // === Mainline tests (merge commit cherry-pick/revert) ===

    #[test]
    fn cherry_pick_merge_commit_without_mainline_errors() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        let merge_id = resolve_tag(&repo, "merge-commit");
        let result = repo.cherry_pick(merge_id, Options::default());

        match result {
            Err(gix::repository::cherry_pick::Error::MergeCommitWithoutMainline { id }) => {
                assert_eq!(id, merge_id, "error should contain the merge commit OID");
            }
            other => panic!("expected MergeCommitWithoutMainline error, got: {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn cherry_pick_non_merge_with_mainline_errors() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        let feature_tip = branch_tip(&repo, "feature");
        let opts = Options {
            mainline: Some(1),
            ..Default::default()
        };
        let result = repo.cherry_pick(feature_tip, opts);

        match result {
            Err(gix::repository::cherry_pick::Error::MainlineForNonMerge { id }) => {
                assert_eq!(id, feature_tip, "error should contain the commit OID");
            }
            other => panic!("expected MainlineForNonMerge error, got: {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn cherry_pick_merge_commit_with_mainline_succeeds() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let workdir = _tmp.path();

        let merge_id = resolve_tag(&repo, "merge-commit");
        // mainline=1 means use first parent (merge-side commit) as ancestor.
        // The merge introduced merge-base.txt from the second parent.
        let opts = Options {
            mainline: Some(1),
            ..Default::default()
        };
        let outcome = repo.cherry_pick(merge_id, opts)?;
        assert!(outcome.commit_id.is_some(), "cherry-pick should create a commit");

        // The cherry-pick with mainline=1 should apply changes relative to parent 1.
        // Parent 1 is merge-side, parent 2 is merge-base.
        // Changes relative to merge-side = adding merge-base.txt.
        assert!(
            workdir.join("merge-base.txt").exists(),
            "merge-base.txt should exist after cherry-picking merge with mainline=1"
        );

        Ok(())
    }

    #[test]
    fn cherry_pick_merge_commit_mainline_out_of_range_errors() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        let merge_id = resolve_tag(&repo, "merge-commit");
        let opts = Options {
            mainline: Some(5),
            ..Default::default()
        };
        let result = repo.cherry_pick(merge_id, opts);

        match result {
            Err(gix::repository::cherry_pick::Error::MainlineOutOfRange {
                id,
                mainline,
                num_parents,
            }) => {
                assert_eq!(id, merge_id);
                assert_eq!(mainline, 5);
                assert_eq!(num_parents, 2);
            }
            other => panic!("expected MainlineOutOfRange error, got: {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn cherry_pick_merge_commit_mainline_zero_errors() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;

        let merge_id = resolve_tag(&repo, "merge-commit");
        let opts = Options {
            mainline: Some(0),
            ..Default::default()
        };
        let result = repo.cherry_pick(merge_id, opts);

        match result {
            Err(gix::repository::cherry_pick::Error::MainlineOutOfRange { .. }) => {}
            other => panic!("expected MainlineOutOfRange error for mainline=0, got: {other:?}"),
        }

        Ok(())
    }

    // === Root commit cherry-pick ===

    #[test]
    fn cherry_pick_root_commit_succeeds() -> crate::Result {
        let (repo, _tmp) = repo_cherry_pick()?;
        let workdir = _tmp.path();

        let root_id = resolve_tag(&repo, "root-commit");
        let outcome = repo.cherry_pick(root_id, Options::default())?;
        assert!(
            outcome.commit_id.is_some(),
            "cherry-pick of root commit should create a commit"
        );

        // The root commit adds orphan.txt.
        assert!(
            workdir.join("orphan.txt").exists(),
            "orphan.txt should exist after cherry-picking root commit"
        );
        let content = std::fs::read_to_string(workdir.join("orphan.txt"))?;
        assert_eq!(content, "orphan content\n");

        // Existing files from main should still be present.
        assert!(workdir.join("file.txt").exists(), "file.txt should still exist");
        assert!(workdir.join("other.txt").exists(), "other.txt should still exist");

        Ok(())
    }
}

/// Interop tests that verify gix cherry-pick and revert produce identical results to C Git.
#[cfg(all(feature = "merge", feature = "worktree-mutation"))]
mod cherry_pick_interop {
    use crate::util::repo_rw;
    use gix::repository::cherry_pick::Options;

    fn repo_interop() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
        repo_rw("make_cherry_pick_interop_repo.sh")
    }

    /// Run a git command in the given directory and return its stdout as a trimmed string.
    fn git(workdir: &std::path::Path, args: &[&str]) -> String {
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

    /// Resolve a tag/ref to an ObjectId via gix.
    fn resolve_tag(repo: &gix::Repository, name: &str) -> gix_hash::ObjectId {
        repo.find_reference(name)
            .unwrap_or_else(|_| panic!("reference {name} should exist"))
            .into_fully_peeled_id()
            .unwrap_or_else(|_| panic!("reference {name} should peel"))
            .detach()
    }

    /// Read the tree entries (file paths) of a commit as a sorted list.
    fn tree_paths(repo: &gix::Repository, commit_id: gix_hash::ObjectId) -> Vec<String> {
        let commit = repo.find_object(commit_id).expect("commit exists").into_commit();
        let tree_id = commit.tree_id().expect("has tree");
        let tree = repo.find_object(tree_id).expect("tree exists").into_tree();
        let mut paths: Vec<String> = tree
            .iter()
            .map(|e| {
                let entry = e.expect("valid entry");
                entry.filename().to_string()
            })
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn gix_cherry_pick_verified_by_c_git() -> crate::Result {
        let (repo, tmp) = repo_interop()?;
        let workdir = tmp.path();

        let feature_parent = resolve_tag(&repo, "feature-parent");
        let outcome = repo.cherry_pick(feature_parent, Options::default())?;
        let new_commit_id = outcome.commit_id.expect("cherry-pick should create a commit");

        let log = git(workdir, &["log", "--oneline", "-1"]);
        assert!(
            log.contains("feature: add feature.txt"),
            "C Git log should show the cherry-picked commit message, got: {log}"
        );

        let c_git_head = git(workdir, &["rev-parse", "HEAD"]);
        assert_eq!(
            c_git_head,
            new_commit_id.to_string(),
            "C Git HEAD should match gix's new commit"
        );

        let diff_output = std::process::Command::new("git")
            .args(["diff", "--exit-code", "HEAD"])
            .current_dir(workdir)
            .status()
            .expect("git must be on PATH");
        assert!(
            diff_output.success(),
            "C Git diff HEAD should be clean after gix cherry-pick"
        );

        let feature_content = git(workdir, &["show", "HEAD:feature.txt"]);
        assert_eq!(feature_content, "new feature file");

        Ok(())
    }

    #[test]
    fn c_git_cherry_pick_verified_by_gix() -> crate::Result {
        let (repo, tmp) = repo_interop()?;
        let workdir = tmp.path();

        let feature_parent_hex = resolve_tag(&repo, "feature-parent").to_string();
        git(workdir, &["cherry-pick", &feature_parent_hex]);

        let repo = gix::open_opts(
            workdir,
            gix::open::Options::isolated().config_overrides(["user.name=gitoxide", "user.email=gitoxide@localhost"]),
        )?;

        let head_id = repo.head_id()?.detach();
        let head_commit = repo.find_object(head_id)?.into_commit();

        let msg = head_commit.decode()?.message.to_string();
        assert!(
            msg.contains("feature: add feature.txt"),
            "gix should see the cherry-picked commit message: {msg}"
        );

        let paths = tree_paths(&repo, head_id);
        assert!(
            paths.contains(&"feature.txt".to_string()),
            "tree should contain feature.txt, got: {paths:?}"
        );

        let parents: Vec<_> = head_commit.parent_ids().map(|id| id.detach()).collect();
        assert_eq!(parents.len(), 1, "cherry-pick should produce a single-parent commit");

        Ok(())
    }

    #[test]
    fn gix_revert_verified_by_c_git() -> crate::Result {
        let (repo, tmp) = repo_interop()?;
        let workdir = tmp.path();

        let feature_parent = resolve_tag(&repo, "feature-parent");
        let pick = repo.cherry_pick(feature_parent, Options::default())?;
        let picked_id = pick.commit_id.expect("commit created");

        assert!(
            workdir.join("feature.txt").exists(),
            "feature.txt should exist after cherry-pick"
        );

        let revert = repo.revert(picked_id, Options::default())?;
        let revert_id = revert.commit_id.expect("revert commit created");

        let c_head = git(workdir, &["rev-parse", "HEAD"]);
        assert_eq!(
            c_head,
            revert_id.to_string(),
            "C Git HEAD should match gix revert commit"
        );

        let log = git(workdir, &["log", "--oneline", "-1"]);
        assert!(
            log.contains("Revert"),
            "C Git log should show 'Revert' in the message: {log}"
        );

        let diff_status = std::process::Command::new("git")
            .args(["diff", "--exit-code", "HEAD"])
            .current_dir(workdir)
            .status()
            .expect("git must be on PATH");
        assert!(diff_status.success(), "worktree should be clean after gix revert");

        assert!(
            !workdir.join("feature.txt").exists(),
            "feature.txt should be removed after revert"
        );

        Ok(())
    }

    #[test]
    fn c_git_revert_verified_by_gix() -> crate::Result {
        let (repo, tmp) = repo_interop()?;
        let workdir = tmp.path();

        let feature_parent_hex = resolve_tag(&repo, "feature-parent").to_string();
        git(workdir, &["cherry-pick", &feature_parent_hex]);
        git(workdir, &["revert", "--no-edit", "HEAD"]);

        let repo = gix::open_opts(
            workdir,
            gix::open::Options::isolated().config_overrides(["user.name=gitoxide", "user.email=gitoxide@localhost"]),
        )?;

        let head_id = repo.head_id()?.detach();
        let head_commit = repo.find_object(head_id)?.into_commit();
        let msg = head_commit.decode()?.message.to_string();

        assert!(
            msg.contains("Revert"),
            "gix should see the revert message from C Git: {msg}"
        );

        let paths = tree_paths(&repo, head_id);
        assert!(
            !paths.contains(&"feature.txt".to_string()),
            "tree should not contain feature.txt after revert, got: {paths:?}"
        );

        Ok(())
    }

    #[test]
    fn revert_message_format_matches_c_git() -> crate::Result {
        let (repo, _tmp) = repo_interop()?;

        let feature_parent = resolve_tag(&repo, "feature-parent");
        let pick = repo.cherry_pick(feature_parent, Options::default())?;
        let picked_id = pick.commit_id.expect("commit created");
        let gix_revert = repo.revert(picked_id, Options::default())?;
        let gix_revert_id = gix_revert.commit_id.expect("revert created");
        let gix_revert_commit = repo.find_object(gix_revert_id)?.into_commit();
        let gix_msg = gix_revert_commit.decode()?.message.to_string();

        assert!(
            gix_msg.starts_with("Revert \"feature: add feature.txt\""),
            "gix revert message title mismatch: {gix_msg}"
        );
        let picked_hex = picked_id.to_string();
        assert!(
            gix_msg.contains(&format!("This reverts commit {picked_hex}.")),
            "gix revert message should contain commit hash: {gix_msg}"
        );

        Ok(())
    }

    #[test]
    fn conflict_detection_matches_c_git() -> crate::Result {
        let (repo, tmp) = repo_interop()?;
        let workdir = tmp.path();

        let conflict_tip = resolve_tag(&repo, "conflict-tip");

        let gix_result = repo.cherry_pick(conflict_tip, Options::default());
        assert!(gix_result.is_err(), "gix cherry-pick should fail on conflicting commit");

        let c_git_status = std::process::Command::new("git")
            .args(["cherry-pick", &conflict_tip.to_string()])
            .current_dir(workdir)
            .status()
            .expect("git must be on PATH");
        assert!(
            !c_git_status.success(),
            "C Git cherry-pick should also fail on the same conflict"
        );

        Ok(())
    }

    #[test]
    fn sequential_cherry_pick_matches_c_git() -> crate::Result {
        let (repo, tmp) = repo_interop()?;
        let workdir = tmp.path();

        let multi_ids = [
            resolve_tag(&repo, "multi-1"),
            resolve_tag(&repo, "multi-2"),
            resolve_tag(&repo, "multi-3"),
        ];

        for id in &multi_ids {
            repo.cherry_pick(*id, Options::default())
                .unwrap_or_else(|e| panic!("cherry-pick {id} should succeed: {e}"));
        }

        let gix_head = repo.head_id()?.detach();
        let gix_files = tree_paths(&repo, gix_head);

        let c_git_files = git(workdir, &["ls-tree", "--name-only", "HEAD"]);
        let mut c_git_list: Vec<&str> = c_git_files.lines().collect();
        c_git_list.sort();

        assert_eq!(
            gix_files,
            c_git_list.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "gix and C Git should agree on the tree contents after sequential cherry-pick"
        );

        for name in &["multi1.txt", "multi2.txt", "multi3.txt"] {
            assert!(
                workdir.join(name).exists(),
                "{name} should exist in worktree after sequential cherry-pick"
            );
        }

        let log = git(workdir, &["log", "--oneline", "-3"]);
        assert!(
            log.contains("multi: add multi3.txt"),
            "log should contain multi3: {log}"
        );
        assert!(
            log.contains("multi: add multi2.txt"),
            "log should contain multi2: {log}"
        );
        assert!(
            log.contains("multi: add multi1.txt"),
            "log should contain multi1: {log}"
        );

        Ok(())
    }

    #[test]
    fn cherry_pick_tree_identical_to_c_git() -> crate::Result {
        let (_repo_gix, tmp_gix) = repo_interop()?;
        let (_repo_cgit, tmp_cgit) = repo_interop()?;
        let gix_dir = tmp_gix.path();
        let cgit_dir = tmp_cgit.path();

        let repo_gix = gix::open_opts(
            gix_dir,
            gix::open::Options::isolated().config_overrides(["user.name=gitoxide", "user.email=gitoxide@localhost"]),
        )?;
        let feature_parent = resolve_tag(&repo_gix, "feature-parent");

        let gix_outcome = repo_gix.cherry_pick(feature_parent, Options::default())?;
        let gix_tree_id = gix_outcome.tree_id;

        git(cgit_dir, &["cherry-pick", &feature_parent.to_string()]);
        let c_git_tree_hex = git(cgit_dir, &["rev-parse", "HEAD^{tree}"]);

        assert_eq!(
            gix_tree_id.to_string(),
            c_git_tree_hex,
            "gix and C Git cherry-pick should produce identical trees"
        );

        Ok(())
    }

    #[test]
    fn no_commit_index_matches_c_git() -> crate::Result {
        let (_repo_gix, tmp_gix) = repo_interop()?;
        let (_repo_cgit, tmp_cgit) = repo_interop()?;
        let gix_dir = tmp_gix.path();
        let cgit_dir = tmp_cgit.path();

        let repo_gix = gix::open_opts(
            gix_dir,
            gix::open::Options::isolated().config_overrides(["user.name=gitoxide", "user.email=gitoxide@localhost"]),
        )?;
        let feature_parent = resolve_tag(&repo_gix, "feature-parent");

        let gix_outcome = repo_gix.cherry_pick(
            feature_parent,
            Options {
                no_commit: true,
                ..Default::default()
            },
        )?;
        assert!(gix_outcome.commit_id.is_none(), "no_commit should not create a commit");

        git(cgit_dir, &["cherry-pick", "--no-commit", &feature_parent.to_string()]);

        let gix_tree = git(gix_dir, &["write-tree"]);
        let cgit_tree = git(cgit_dir, &["write-tree"]);

        assert_eq!(
            gix_tree, cgit_tree,
            "gix and C Git --no-commit should produce identical index trees"
        );

        let gix_head = git(gix_dir, &["rev-parse", "HEAD"]);
        let cgit_head = git(cgit_dir, &["rev-parse", "HEAD"]);
        assert_eq!(gix_head, cgit_head, "HEAD should not move in either case");

        Ok(())
    }

    #[test]
    fn roundtrip_restores_tree_verified_by_c_git() -> crate::Result {
        let (repo, tmp) = repo_interop()?;
        let workdir = tmp.path();

        let original_tree = git(workdir, &["rev-parse", "HEAD^{tree}"]);

        let feature_parent = resolve_tag(&repo, "feature-parent");
        let pick = repo.cherry_pick(feature_parent, Options::default())?;
        let picked_id = pick.commit_id.expect("commit created");
        repo.revert(picked_id, Options::default())?;

        let final_tree = git(workdir, &["rev-parse", "HEAD^{tree}"]);
        assert_eq!(
            original_tree, final_tree,
            "roundtrip cherry-pick + revert should restore the original tree"
        );

        let diff_status = std::process::Command::new("git")
            .args(["diff", "--exit-code", "HEAD"])
            .current_dir(workdir)
            .status()
            .expect("git must be on PATH");
        assert!(diff_status.success(), "worktree should be clean after roundtrip");

        Ok(())
    }

    #[test]
    fn author_preserved_like_c_git() -> crate::Result {
        let (_repo_gix, tmp_gix) = repo_interop()?;
        let (_repo_cgit, tmp_cgit) = repo_interop()?;
        let gix_dir = tmp_gix.path();
        let cgit_dir = tmp_cgit.path();

        let repo_gix = gix::open_opts(
            gix_dir,
            gix::open::Options::isolated().config_overrides(["user.name=gitoxide", "user.email=gitoxide@localhost"]),
        )?;
        let feature_parent = resolve_tag(&repo_gix, "feature-parent");

        repo_gix.cherry_pick(feature_parent, Options::default())?;

        git(
            cgit_dir,
            &[
                "-c",
                "user.name=C Git User",
                "-c",
                "user.email=cgit@example.com",
                "cherry-pick",
                &feature_parent.to_string(),
            ],
        );

        let orig_author = git(
            gix_dir,
            &["log", "-1", "--format=%an <%ae>", &feature_parent.to_string()],
        );

        let gix_author = git(gix_dir, &["log", "-1", "--format=%an <%ae>"]);
        let cgit_author = git(cgit_dir, &["log", "-1", "--format=%an <%ae>"]);
        assert_eq!(
            gix_author, cgit_author,
            "author should be preserved identically by both gix and C Git"
        );

        assert_eq!(gix_author, orig_author, "gix should preserve the original author");
        assert_eq!(
            cgit_author, orig_author,
            "C Git should also preserve the original author"
        );

        let gix_committer = git(gix_dir, &["log", "-1", "--format=%cn <%ce>"]);
        let cgit_committer = git(cgit_dir, &["log", "-1", "--format=%cn <%ce>"]);
        assert!(!gix_committer.is_empty(), "gix committer should be set");
        assert!(!cgit_committer.is_empty(), "C Git committer should be set");

        Ok(())
    }
}
