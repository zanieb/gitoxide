#[cfg(feature = "worktree-mutation")]
mod stash {
    use crate::util::repo_rw;

    fn repo_rw_stash() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
        repo_rw("make_stash_repo.sh")
    }

    #[test]
    fn stash_list_empty_when_no_stashes() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;
        let entries = repo.stash_list()?;
        assert!(entries.is_empty(), "no stash entries should exist initially");
        Ok(())
    }

    #[test]
    fn stash_save_creates_stash_entry() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        // The fixture has a staged modification, so stash_save should work.
        let stash_id = repo.stash_save(None)?;

        // Verify a stash entry was created.
        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 1, "one stash entry should exist");
        assert_eq!(
            entries[0].commit_id.detach(),
            stash_id.detach(),
            "stash list should contain the created stash commit"
        );

        // The message should contain "WIP on main"
        let msg = entries[0].message.to_string();
        assert!(msg.contains("WIP on main"), "message should contain branch name: {msg}");

        Ok(())
    }

    #[test]
    fn stash_save_with_custom_message() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        let _stash_id = repo.stash_save(Some("my custom message"))?;

        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 1);

        let msg = entries[0].message.to_string();
        assert!(
            msg.contains("my custom message"),
            "message should contain custom text: {msg}"
        );

        Ok(())
    }

    #[test]
    fn stash_save_nothing_to_stash_when_clean() -> crate::Result {
        let (repo, _tmp) = repo_rw("make_reset_repo.sh")?;

        // The reset repo fixture has HEAD matching the index, so nothing to stash.
        let result = repo.stash_save(None);
        assert!(result.is_err(), "stash_save should fail when nothing to stash");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Nothing to stash"),
            "error should indicate nothing to stash: {err}"
        );

        Ok(())
    }

    #[test]
    fn stash_save_resets_index_to_head() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;
        let head_id = repo.head_id()?.detach();

        // Before save, the index differs from HEAD (file.txt = "modified").
        let index_before = repo.open_index()?;
        let head_commit = repo.find_object(head_id)?.try_into_commit().expect("commit");
        let head_tree_id = head_commit.tree_id().expect("has tree");
        let head_tree = repo.find_object(head_tree_id)?.try_into_tree().expect("tree");
        let head_tree_decoded = head_tree.decode().expect("decoded");
        let head_file_oid = head_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "file.txt".as_bytes())
            .expect("file.txt in tree")
            .oid;
        let index_entry_before = index_before
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_before);
                p == b"file.txt"
            })
            .expect("file.txt in index");
        assert_ne!(
            index_entry_before.id, head_file_oid,
            "before save, index should differ from HEAD"
        );

        // Save stash - this should also reset.
        repo.stash_save(None)?;

        // After save, the index should match HEAD.
        let index_after = repo.open_index()?;
        let entry_after = index_after
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_after);
                p == b"file.txt"
            })
            .expect("file.txt in index after save");
        assert_eq!(
            entry_after.id, head_file_oid,
            "after save, index should match HEAD (reset happened)"
        );

        Ok(())
    }

    #[test]
    fn stash_drop_last_entry_removes_ref() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        repo.stash_save(None)?;
        assert_eq!(repo.stash_list()?.len(), 1);

        repo.stash_drop(0)?;

        let entries = repo.stash_list()?;
        assert!(
            entries.is_empty(),
            "stash should be empty after dropping the only entry"
        );

        // refs/stash should no longer exist.
        let stash_ref = repo.try_find_reference("refs/stash")?;
        assert!(
            stash_ref.is_none(),
            "refs/stash should be gone after dropping last stash"
        );

        Ok(())
    }

    #[test]
    fn stash_drop_out_of_range_fails() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        repo.stash_save(None)?;

        let result = repo.stash_drop(5);
        assert!(result.is_err(), "dropping out-of-range index should fail");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("out of range"),
            "error should mention out of range: {err}"
        );

        Ok(())
    }

    #[test]
    fn stash_drop_no_stash_fails() -> crate::Result {
        let (repo, _tmp) = repo_rw("make_reset_repo.sh")?;

        let result = repo.stash_drop(0);
        assert!(result.is_err(), "dropping from empty stash should fail");

        Ok(())
    }

    #[test]
    fn stash_save_creates_valid_commit_structure() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        let head_id = repo.head_id()?.detach();
        let stash_id = repo.stash_save(None)?;

        // The stash commit should have 2 parents: HEAD and the index commit.
        let stash_commit = repo.find_object(stash_id)?.into_commit();
        let parent_ids: Vec<_> = stash_commit.parent_ids().map(|id| id.detach()).collect();
        assert_eq!(parent_ids.len(), 2, "stash commit should have 2 parents");
        assert_eq!(parent_ids[0], head_id, "first parent should be HEAD");

        // The index commit (second parent) should have HEAD as its parent.
        let index_commit = repo.find_object(parent_ids[1])?.into_commit();
        let index_parent_ids: Vec<_> = index_commit.parent_ids().map(|id| id.detach()).collect();
        assert_eq!(index_parent_ids.len(), 1, "index commit should have 1 parent");
        assert_eq!(index_parent_ids[0], head_id, "index commit parent should be HEAD");

        Ok(())
    }

    #[test]
    fn stash_apply_restores_index_state() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;
        let head_id = repo.head_id()?.detach();

        // Save stash (captures staged "modified" in file.txt, then resets to HEAD).
        repo.stash_save(None)?;

        // After save, index should already match HEAD (stash_save resets).
        let head_commit = repo.find_object(head_id)?.try_into_commit().expect("commit");
        let head_tree_id = head_commit.tree_id().expect("has tree");
        let head_tree = repo.find_object(head_tree_id)?.try_into_tree().expect("tree");
        let head_tree_decoded = head_tree.decode().expect("decoded");
        let head_file_oid = head_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "file.txt".as_bytes())
            .expect("file.txt in tree")
            .oid;

        // Apply the stash.
        repo.stash_apply(0)?;

        // After apply, the index should have the stashed version (different from HEAD).
        let index_after = repo.open_index()?;
        let entry_after = index_after
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_after);
                p == b"file.txt"
            })
            .expect("file.txt in index after apply");
        assert_ne!(
            entry_after.id, head_file_oid,
            "after apply, index should differ from HEAD"
        );

        // The stash should still exist (apply does not remove it).
        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 1, "stash entry should still exist after apply");

        Ok(())
    }

    #[test]
    fn stash_apply_no_stash_fails() -> crate::Result {
        let (repo, _tmp) = repo_rw("make_reset_repo.sh")?;

        let result = repo.stash_apply(0);
        assert!(result.is_err(), "apply with no stash should fail");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("No stash"),
            "error should mention no stash: {err}"
        );

        Ok(())
    }

    #[test]
    fn stash_apply_out_of_range_fails() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        repo.stash_save(None)?;

        let result = repo.stash_apply(5);
        assert!(result.is_err(), "apply out-of-range should fail");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("out of range"),
            "error should mention out of range: {err}"
        );

        Ok(())
    }

    #[test]
    fn stash_pop_applies_and_drops() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        // Save a stash (also resets to HEAD).
        repo.stash_save(None)?;

        // Pop should apply and then drop.
        repo.stash_pop(0)?;

        // After pop, the stash list should be empty.
        let entries = repo.stash_list()?;
        assert!(entries.is_empty(), "stash should be empty after pop");

        // And refs/stash should be gone.
        let stash_ref = repo.try_find_reference("refs/stash")?;
        assert!(stash_ref.is_none(), "refs/stash should be gone after pop");

        Ok(())
    }

    #[test]
    fn stash_pop_no_stash_fails() -> crate::Result {
        let (repo, _tmp) = repo_rw("make_reset_repo.sh")?;

        let result = repo.stash_pop(0);
        assert!(result.is_err(), "pop with no stash should fail");

        Ok(())
    }

    /// Verify the stash commit tree captures worktree state (not just index state).
    ///
    /// C Git stash format:
    /// - Index commit: tree = index state
    /// - Stash commit: tree = worktree state (includes unstaged modifications)
    ///
    /// When a file is staged with one version but the worktree has a further modification,
    /// the stash commit's tree should contain the worktree version, while the index commit's
    /// tree should contain the staged version.
    #[test]
    fn stash_commit_tree_captures_worktree_state() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // The fixture has file.txt staged as "modified" (different from HEAD "hello").
        // Now write a further unstaged change to the worktree.
        std::fs::write(workdir.join("file.txt"), "worktree version\n")?;

        let stash_id = repo.stash_save(None)?;

        // Get the stash commit tree.
        let stash_commit = repo.find_object(stash_id)?.try_into_commit().expect("commit");
        let stash_tree_id = stash_commit.tree_id().expect("has tree").detach();
        let parent_ids: Vec<_> = stash_commit.parent_ids().collect();
        assert_eq!(parent_ids.len(), 2, "stash commit should have 2 parents");

        // The index commit (second parent) has the staged tree.
        let index_commit = repo.find_object(parent_ids[1])?.try_into_commit().expect("commit");
        let index_tree_id = index_commit.tree_id().expect("has tree").detach();

        // The stash commit tree should differ from the index commit tree,
        // because the worktree had additional unstaged changes.
        assert_ne!(
            stash_tree_id, index_tree_id,
            "stash commit tree should differ from index commit tree when worktree has unstaged changes"
        );

        // Verify the stash tree contains the worktree version of the file.
        let stash_tree = repo.find_object(stash_tree_id)?.try_into_tree().expect("tree");
        let stash_tree_decoded = stash_tree.decode().expect("decoded");
        let stash_file_entry = stash_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "file.txt".as_bytes())
            .expect("file.txt should be in stash tree");
        let stash_blob = repo.find_object(stash_file_entry.oid)?;
        assert_eq!(
            stash_blob.data, b"worktree version\n",
            "stash tree should contain the worktree version of the file"
        );

        // Verify the index tree contains the staged version.
        let index_tree = repo.find_object(index_tree_id)?.try_into_tree().expect("tree");
        let index_tree_decoded = index_tree.decode().expect("decoded");
        let index_file_entry = index_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "file.txt".as_bytes())
            .expect("file.txt should be in index tree");
        let index_blob = repo.find_object(index_file_entry.oid)?;
        assert_eq!(
            index_blob.data, b"modified\n",
            "index tree should contain the staged version of the file"
        );

        Ok(())
    }

    /// When the index matches HEAD but the worktree has modifications,
    /// stash should still capture the worktree changes.
    #[test]
    fn stash_save_captures_unstaged_only_changes() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_repo.sh")?;
        let workdir = tmp.path().to_owned();

        // make_reset_repo.sh has HEAD == index (clean state).
        // Write an unstaged modification to 'first'.
        std::fs::write(workdir.join("first"), "unstaged worktree change\n")?;

        // Stash should succeed because the worktree differs from HEAD,
        // even though the index matches HEAD.
        let stash_id = repo.stash_save(None)?;

        // The stash commit tree should contain the worktree version.
        let stash_commit = repo.find_object(stash_id)?.try_into_commit().expect("commit");
        let stash_tree_id = stash_commit.tree_id().expect("has tree");
        let stash_tree = repo.find_object(stash_tree_id)?.try_into_tree().expect("tree");
        let stash_tree_decoded = stash_tree.decode().expect("decoded");
        let first_entry = stash_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "first".as_bytes())
            .expect("first should be in stash tree");
        let blob = repo.find_object(first_entry.oid)?;
        assert_eq!(
            blob.data, b"unstaged worktree change\n",
            "stash tree should contain the unstaged worktree change"
        );

        Ok(())
    }

    #[test]
    fn stash_save_twice_creates_two_entries() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;

        // First stash save (saves "modified", resets to HEAD "hello").
        repo.stash_save(None)?;
        assert_eq!(repo.stash_list()?.len(), 1);

        // Now the index matches HEAD. Create a new modification to stash.
        let workdir = tmp.path().to_owned();
        std::fs::write(workdir.join("file.txt"), "second modification\n")?;

        // Stage the new change via git add (update the index).
        // We need to write a new blob and update the index entry.
        let new_blob_id = repo.write_blob("second modification\n")?;
        let mut index = repo.open_index()?;
        for (entry, entry_path) in index.entries_mut_with_paths() {
            let p: &[u8] = &**entry_path;
            if p == b"file.txt" {
                entry.id = new_blob_id.detach();
                break;
            }
        }
        index.write(Default::default())?;

        // Second stash save.
        repo.stash_save(Some("second stash"))?;

        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 2, "should have two stash entries");

        // stash@{0} is the most recent.
        let msg0 = entries[0].message.to_string();
        assert!(
            msg0.contains("second stash"),
            "stash@{{0}} should be the second stash: {msg0}"
        );

        // stash@{1} is the first one.
        let msg1 = entries[1].message.to_string();
        assert!(
            msg1.contains("WIP on main"),
            "stash@{{1}} should be the first stash: {msg1}"
        );

        Ok(())
    }

    #[test]
    fn stash_save_keep_index_preserves_staged_changes() -> crate::Result {
        use gix::repository::stash::StashSaveOptions;

        let (repo, _tmp) = repo_rw_stash()?;
        let head_id = repo.head_id()?.detach();

        // Get the HEAD tree's version of file.txt for comparison.
        let head_commit = repo.find_object(head_id)?.try_into_commit().expect("commit");
        let head_tree_id = head_commit.tree_id().expect("has tree");
        let head_tree = repo.find_object(head_tree_id)?.try_into_tree().expect("tree");
        let head_tree_decoded = head_tree.decode().expect("decoded");
        let head_file_oid = head_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "file.txt".as_bytes())
            .expect("file.txt in tree")
            .oid;

        // Before stash, the index has "modified" staged (different from HEAD "hello").
        let index_before = repo.open_index()?;
        let staged_oid = index_before
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_before);
                p == b"file.txt"
            })
            .expect("file.txt in index")
            .id;
        assert_ne!(staged_oid, head_file_oid, "staged should differ from HEAD");

        // Stash with keep_index.
        repo.stash_save_opts(StashSaveOptions {
            message: Some("keep-index test"),
            keep_index: true,
            include_untracked: false,
        })?;

        // After stash with keep_index, the index should STILL have the staged version.
        let index_after = repo.open_index()?;
        let entry_after = index_after
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_after);
                p == b"file.txt"
            })
            .expect("file.txt in index after keep-index save");
        assert_ne!(
            entry_after.id, head_file_oid,
            "after keep-index save, index should NOT be reset to HEAD"
        );
        assert_eq!(
            entry_after.id, staged_oid,
            "after keep-index save, index should retain the staged version"
        );

        Ok(())
    }

    #[test]
    fn stash_save_include_untracked_captures_untracked_files() -> crate::Result {
        use gix::repository::stash::StashSaveOptions;

        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Create an untracked file.
        std::fs::write(workdir.join("untracked.txt"), "untracked content\n")?;

        // Stash with include_untracked.
        let stash_id = repo.stash_save_opts(StashSaveOptions {
            message: Some("include-untracked test"),
            keep_index: false,
            include_untracked: true,
        })?;

        // The stash commit should have 3 parents (HEAD, index, untracked).
        let stash_commit = repo.find_object(stash_id)?.into_commit();
        let parent_ids: Vec<_> = stash_commit.parent_ids().map(|id| id.detach()).collect();
        assert_eq!(
            parent_ids.len(),
            3,
            "stash with include_untracked should have 3 parents"
        );

        // The third parent is the untracked files commit.
        let untracked_commit = repo.find_object(parent_ids[2])?.into_commit();
        let untracked_tree_id = untracked_commit.tree_id().expect("has tree");

        // The untracked commit should have no parents (it's a standalone commit).
        let untracked_parent_ids: Vec<_> = untracked_commit.parent_ids().map(|id| id.detach()).collect();
        assert!(
            untracked_parent_ids.is_empty(),
            "untracked commit should have no parents"
        );

        // The untracked tree should contain untracked.txt.
        let untracked_tree = repo.find_object(untracked_tree_id)?.try_into_tree().expect("tree");
        let untracked_tree_decoded = untracked_tree.decode().expect("decoded");
        let untracked_entry = untracked_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "untracked.txt".as_bytes())
            .expect("untracked.txt should be in the untracked tree");
        let blob = repo.find_object(untracked_entry.oid)?;
        assert_eq!(
            blob.data, b"untracked content\n",
            "untracked blob should contain the file content"
        );

        // The untracked file should be removed from the worktree.
        assert!(
            !workdir.join("untracked.txt").exists(),
            "untracked file should be removed from worktree after stash"
        );

        Ok(())
    }

    /// When include_untracked is set but there are no untracked files,
    /// the stash should still have 3 parents (with an empty tree for the 3rd parent).
    /// This matches C Git and libgit2 behavior -- consumers check parentcount == 3
    /// to detect the untracked format.
    #[test]
    fn stash_save_include_untracked_without_untracked_still_creates_3_parents() -> crate::Result {
        use gix::repository::stash::StashSaveOptions;

        let (repo, _tmp) = repo_rw_stash()?;

        // Stash with include_untracked but no untracked files exist.
        // Should still work (staged changes provide something to stash).
        let stash_id = repo.stash_save_opts(StashSaveOptions {
            message: None,
            keep_index: false,
            include_untracked: true,
        })?;

        // Even with no untracked files, should have 3 parents (empty tree for 3rd parent).
        let stash_commit = repo.find_object(stash_id)?.into_commit();
        let parent_ids: Vec<_> = stash_commit.parent_ids().map(|id| id.detach()).collect();
        assert_eq!(
            parent_ids.len(),
            3,
            "stash with include_untracked should always have 3 parents, even with no untracked files"
        );

        // The 3rd parent's tree should be the empty tree.
        let untracked_commit = repo.find_object(parent_ids[2])?.into_commit();
        let untracked_tree_id = untracked_commit.tree_id().expect("has tree").detach();
        let empty_tree_id = gix_hash::ObjectId::empty_tree(repo.object_hash());
        assert_eq!(
            untracked_tree_id, empty_tree_id,
            "3rd parent should have the empty tree when no untracked files exist"
        );

        Ok(())
    }

    /// Create 3 stashes, drop the middle one (stash@{1}), and verify stash@{0}
    /// and the former stash@{2} (now stash@{1}) are correct.
    #[test]
    fn stash_drop_mid_stack() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // First stash (will become stash@{2}).
        repo.stash_save(Some("first"))?;
        assert_eq!(repo.stash_list()?.len(), 1);

        // Create a new modification for second stash.
        std::fs::write(workdir.join("file.txt"), "second change\n")?;
        let blob_id = repo.write_blob("second change\n")?;
        let mut index = repo.open_index()?;
        for (entry, entry_path) in index.entries_mut_with_paths() {
            let p: &[u8] = &**entry_path;
            if p == b"file.txt" {
                entry.id = blob_id.detach();
                break;
            }
        }
        index.write(Default::default())?;

        repo.stash_save(Some("second"))?;
        assert_eq!(repo.stash_list()?.len(), 2);

        // Create a new modification for third stash.
        std::fs::write(workdir.join("file.txt"), "third change\n")?;
        let blob_id = repo.write_blob("third change\n")?;
        let mut index = repo.open_index()?;
        for (entry, entry_path) in index.entries_mut_with_paths() {
            let p: &[u8] = &**entry_path;
            if p == b"file.txt" {
                entry.id = blob_id.detach();
                break;
            }
        }
        index.write(Default::default())?;

        repo.stash_save(Some("third"))?;
        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 3);

        // Verify ordering: stash@{0}=third, stash@{1}=second, stash@{2}=first.
        assert!(entries[0].message.to_string().contains("third"));
        assert!(entries[1].message.to_string().contains("second"));
        assert!(entries[2].message.to_string().contains("first"));

        // Record the commit IDs for stash@{0} (third) and stash@{2} (first).
        let third_id = entries[0].commit_id.detach();
        let first_id = entries[2].commit_id.detach();

        // Drop the middle one (stash@{1} = "second").
        repo.stash_drop(1)?;

        let entries_after = repo.stash_list()?;
        assert_eq!(
            entries_after.len(),
            2,
            "should have 2 stash entries after mid-stack drop"
        );

        // stash@{0} should still be "third" with the same commit ID.
        assert_eq!(entries_after[0].commit_id.detach(), third_id);
        assert!(entries_after[0].message.to_string().contains("third"));

        // stash@{1} should now be "first" (formerly stash@{2}).
        assert_eq!(entries_after[1].commit_id.detach(), first_id);
        assert!(entries_after[1].message.to_string().contains("first"));

        Ok(())
    }

    /// Stash, make a new commit on the same branch, then apply the stash.
    /// This verifies apply works when HEAD has moved since the stash was created.
    #[test]
    fn stash_apply_after_head_moved() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Save stash (captures the staged "modified" in file.txt, resets to HEAD).
        repo.stash_save(None)?;

        // Now make a new commit that changes something else (add a new file).
        std::fs::write(workdir.join("other.txt"), "other content\n")?;
        let blob_id = repo.write_blob("other content\n")?;
        let mut index = repo.open_index()?;
        use gix::bstr::ByteSlice;
        index.add_entry(
            gix_index::entry::Stat::default(),
            blob_id.detach(),
            gix_index::entry::Flags::empty(),
            gix_index::entry::Mode::FILE,
            b"other.txt".as_bstr(),
        );
        index.sort_entries();
        index.write(Default::default())?;

        // Create the commit.
        let new_tree_outcome = index.write_tree_to(|tree| repo.write_object(tree).map(|id| id.detach()))?;
        let committer_ref = repo.committer().expect("has committer").expect("valid");
        let committer: gix_actor::Signature = committer_ref.into();
        let head_id = repo.head_id()?.detach();
        let commit = gix_object::Commit {
            tree: new_tree_outcome.tree_id,
            parents: smallvec::smallvec![head_id],
            author: committer.clone(),
            committer,
            encoding: None,
            message: "add other.txt".into(),
            extra_headers: Default::default(),
        };
        let commit_id = repo.write_object(&commit)?.detach();

        // Update HEAD to point at the new commit.
        use gix_ref::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
        let head_ref = repo.head_ref()?.expect("have HEAD ref");
        repo.edit_reference(RefEdit {
            change: Change::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    force_create_reflog: false,
                    message: "add other.txt".into(),
                },
                expected: PreviousValue::Any,
                new: gix_ref::Target::Object(commit_id),
            },
            name: head_ref.inner.name,
            deref: false,
        })?;

        let new_head_id = repo.head_id()?.detach();
        assert_ne!(new_head_id, head_id, "HEAD should have moved");

        // Now apply the stash. The stash was made against the OLD HEAD.
        repo.stash_apply(0)?;

        // After apply, file.txt should have the stashed content in the index.
        let index_after = repo.open_index()?;
        let entry = index_after
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_after);
                p == b"file.txt"
            })
            .expect("file.txt in index after apply");

        // The stash had "modified" content; HEAD had "hello".
        // The stash tree contains "modified", the parent tree contains "hello",
        // so apply should set file.txt to "modified".
        let blob = repo.find_object(entry.id)?;
        assert_eq!(
            blob.data, b"modified\n",
            "after apply on moved HEAD, stashed changes should be restored"
        );

        // other.txt (from the new commit) should still be in the index.
        let other_entry = index_after.entries().iter().find(|e| {
            let p: &[u8] = &**e.path(&index_after);
            p == b"other.txt"
        });
        assert!(
            other_entry.is_some(),
            "other.txt from new commit should still be present"
        );

        Ok(())
    }

    /// Stash a newly added file (tracked via `git add` but not yet committed).
    #[test]
    fn stash_save_newly_added_file() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Create a brand new file and add it to the index.
        std::fs::write(workdir.join("newfile.txt"), "new content\n")?;
        let blob_id = repo.write_blob("new content\n")?;
        let mut index = repo.open_index()?;
        use gix::bstr::ByteSlice;
        index.add_entry(
            gix_index::entry::Stat::default(),
            blob_id.detach(),
            gix_index::entry::Flags::empty(),
            gix_index::entry::Mode::FILE,
            b"newfile.txt".as_bstr(),
        );
        index.sort_entries();
        index.write(Default::default())?;

        let head_id = repo.head_id()?.detach();

        // Save stash - should capture the new file.
        let stash_id = repo.stash_save(None)?;

        // The stash commit tree should contain newfile.txt.
        let stash_commit = repo.find_object(stash_id)?.try_into_commit().expect("commit");
        let stash_tree_id = stash_commit.tree_id().expect("has tree");
        let stash_tree = repo.find_object(stash_tree_id)?.try_into_tree().expect("tree");
        let stash_tree_decoded = stash_tree.decode().expect("decoded");
        let new_entry = stash_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "newfile.txt".as_bytes());
        assert!(new_entry.is_some(), "stash tree should contain the newly added file");
        let blob = repo.find_object(new_entry.unwrap().oid)?;
        assert_eq!(blob.data, b"new content\n");

        // After stash, the index should no longer contain newfile.txt (reset to HEAD).
        let index_after = repo.open_index()?;
        let has_newfile = index_after.entries().iter().any(|e| {
            let p: &[u8] = &**e.path(&index_after);
            p == b"newfile.txt"
        });
        assert!(
            !has_newfile,
            "after stash, newly added file should be removed from index (reset to HEAD)"
        );

        // HEAD tree should NOT contain newfile.txt.
        let head_commit = repo.find_object(head_id)?.try_into_commit().expect("commit");
        let head_tree_id = head_commit.tree_id().expect("has tree");
        let head_tree = repo.find_object(head_tree_id)?.try_into_tree().expect("tree");
        let head_tree_decoded = head_tree.decode().expect("decoded");
        let head_has_newfile = head_tree_decoded
            .entries
            .iter()
            .any(|e| e.filename == "newfile.txt".as_bytes());
        assert!(!head_has_newfile, "HEAD tree should not contain newfile.txt");

        // Apply the stash to restore.
        repo.stash_apply(0)?;

        // After apply, newfile.txt should be back in the index.
        let index_restored = repo.open_index()?;
        let restored_entry = index_restored.entries().iter().find(|e| {
            let p: &[u8] = &**e.path(&index_restored);
            p == b"newfile.txt"
        });
        assert!(
            restored_entry.is_some(),
            "after apply, newly added file should be restored in the index"
        );

        Ok(())
    }

    /// Verify that .gitignore patterns are respected when collecting untracked files.
    ///
    /// Files matching .gitignore patterns should NOT be captured as untracked by stash.
    #[test]
    fn stash_include_untracked_respects_gitignore() -> crate::Result {
        use gix::repository::stash::StashSaveOptions;

        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Create a .gitignore that ignores *.log files.
        std::fs::write(workdir.join(".gitignore"), "*.log\n")?;

        // Stage the .gitignore so it's tracked.
        let gitignore_blob = repo.write_blob("*.log\n")?;
        let mut index = repo.open_index()?;
        use gix::bstr::ByteSlice;
        index.add_entry(
            gix_index::entry::Stat::default(),
            gitignore_blob.detach(),
            gix_index::entry::Flags::empty(),
            gix_index::entry::Mode::FILE,
            b".gitignore".as_bstr(),
        );
        index.sort_entries();
        index.write(Default::default())?;

        // Create an ignored file and a non-ignored untracked file.
        std::fs::write(workdir.join("debug.log"), "log content\n")?;
        std::fs::write(workdir.join("untracked.txt"), "real untracked\n")?;

        let stash_id = repo.stash_save_opts(StashSaveOptions {
            message: Some("gitignore test"),
            keep_index: false,
            include_untracked: true,
        })?;

        // Get the untracked commit (3rd parent).
        let stash_commit = repo.find_object(stash_id)?.into_commit();
        let parent_ids: Vec<_> = stash_commit.parent_ids().map(|id| id.detach()).collect();
        assert_eq!(parent_ids.len(), 3, "stash with untracked files should have 3 parents");

        let untracked_commit = repo.find_object(parent_ids[2])?.into_commit();
        let untracked_tree_id = untracked_commit.tree_id().expect("has tree");
        let untracked_tree = repo.find_object(untracked_tree_id)?.try_into_tree().expect("tree");
        let untracked_tree_decoded = untracked_tree.decode().expect("decoded");

        let filenames: Vec<String> = untracked_tree_decoded
            .entries
            .iter()
            .map(|e| String::from_utf8_lossy(e.filename.as_ref()).to_string())
            .collect();

        // Ignored files matching .gitignore patterns should NOT be captured.
        assert!(
            !filenames.contains(&"debug.log".to_string()),
            "ignored file debug.log should NOT be captured as untracked. \
             Files in untracked tree: {filenames:?}"
        );

        // The truly untracked file should be captured.
        assert!(
            filenames.contains(&"untracked.txt".to_string()),
            "untracked.txt should be in the untracked tree. Files: {filenames:?}"
        );

        Ok(())
    }

    /// Verify that stash apply restores untracked files from the 3rd parent.
    #[test]
    fn stash_apply_restores_untracked_files() -> crate::Result {
        use gix::repository::stash::StashSaveOptions;

        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Create an untracked file.
        std::fs::write(workdir.join("loose.txt"), "loose content\n")?;

        // Stash with include_untracked.
        repo.stash_save_opts(StashSaveOptions {
            message: Some("untracked test"),
            keep_index: false,
            include_untracked: true,
        })?;

        // The untracked file should have been removed from worktree by save.
        assert!(
            !workdir.join("loose.txt").exists(),
            "untracked file should be removed after stash save"
        );

        // Apply the stash.
        repo.stash_apply(0)?;

        // Untracked files should be restored from the 3rd parent commit.
        assert!(
            workdir.join("loose.txt").exists(),
            "untracked file should be restored by stash_apply from the 3rd parent"
        );
        assert_eq!(
            std::fs::read_to_string(workdir.join("loose.txt"))?,
            "loose content\n",
            "restored untracked file should have original content"
        );

        Ok(())
    }

    /// Verify that apply detects conflicts when the worktree has divergent changes.
    #[test]
    fn stash_apply_conflict_detection() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Save stash (captures staged "modified" in file.txt, resets to HEAD "hello").
        repo.stash_save(None)?;

        // Now make a DIFFERENT modification to file.txt in the index.
        std::fs::write(workdir.join("file.txt"), "conflicting change\n")?;
        let blob_id = repo.write_blob("conflicting change\n")?;
        let mut index = repo.open_index()?;
        for (entry, entry_path) in index.entries_mut_with_paths() {
            let p: &[u8] = &**entry_path;
            if p == b"file.txt" {
                entry.id = blob_id.detach();
                break;
            }
        }
        index.write(Default::default())?;

        // Apply the stash - should fail because file.txt has been modified differently.
        let result = repo.stash_apply(0);
        assert!(
            result.is_err(),
            "apply should fail when the current index has conflicting changes"
        );
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("file.txt"),
            "error should mention the conflicting file: {err_msg}"
        );

        Ok(())
    }

    #[test]
    fn stash_save_keep_index_and_include_untracked_combined() -> crate::Result {
        use gix::repository::stash::StashSaveOptions;

        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();
        let head_id = repo.head_id()?.detach();

        // Get HEAD's file.txt oid for comparison.
        let head_commit = repo.find_object(head_id)?.try_into_commit().expect("commit");
        let head_tree_id = head_commit.tree_id().expect("has tree");
        let head_tree = repo.find_object(head_tree_id)?.try_into_tree().expect("tree");
        let head_tree_decoded = head_tree.decode().expect("decoded");
        let head_file_oid = head_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "file.txt".as_bytes())
            .expect("file.txt in tree")
            .oid;

        // Record the staged oid before stashing.
        let index_before = repo.open_index()?;
        let staged_oid = index_before
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_before);
                p == b"file.txt"
            })
            .expect("file.txt in index")
            .id;

        // Create an untracked file.
        std::fs::write(workdir.join("extra.txt"), "extra content\n")?;

        // Stash with both options.
        let stash_id = repo.stash_save_opts(StashSaveOptions {
            message: Some("combined test"),
            keep_index: true,
            include_untracked: true,
        })?;

        // Stash commit should have 3 parents.
        let stash_commit = repo.find_object(stash_id)?.into_commit();
        let parent_ids: Vec<_> = stash_commit.parent_ids().map(|id| id.detach()).collect();
        assert_eq!(parent_ids.len(), 3, "combined should have 3 parents");

        // Index should retain staged changes (keep_index).
        let index_after = repo.open_index()?;
        let entry_after = index_after
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_after);
                p == b"file.txt"
            })
            .expect("file.txt in index");
        assert_ne!(
            entry_after.id, head_file_oid,
            "keep_index should preserve staged changes"
        );
        assert_eq!(entry_after.id, staged_oid, "index should still have staged oid");

        // Untracked file should be removed.
        assert!(!workdir.join("extra.txt").exists(), "untracked file should be removed");

        Ok(())
    }

    // ========================================================================
    // C Git interop tests
    // ========================================================================

    /// Create a stash with gix and verify it's visible to C Git's `git stash list`.
    #[test]
    fn interop_gix_stash_visible_to_c_git() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        repo.stash_save(Some("gix-created stash"))?;

        let output = std::process::Command::new("git")
            .args(["stash", "list"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH for interop tests");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("gix-created stash"),
            "C Git should see the stash created by gix: {stdout}"
        );

        Ok(())
    }

    /// Create a stash with C Git and verify gix can list and apply it.
    #[test]
    fn interop_c_git_stash_applied_by_gix() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_repo.sh")?;
        let workdir = tmp.path().to_owned();

        // Modify a tracked file and create a stash with C Git.
        std::fs::write(workdir.join("first"), "c-git-stash-content\n")?;
        let status = std::process::Command::new("git")
            .args(["stash", "push", "-m", "c-git stash"])
            .current_dir(&workdir)
            .status()
            .expect("git must be on PATH for interop tests");
        assert!(status.success(), "C git stash push should succeed");

        // Verify gix can see the stash.
        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 1, "gix should see the C Git stash");
        let msg = entries[0].message.to_string();
        assert!(
            msg.contains("c-git stash"),
            "gix should read the C Git stash message: {msg}"
        );

        // Apply via gix.
        repo.stash_apply(0)?;

        // Verify the stashed content was restored.
        let index_after = repo.open_index()?;
        let entry = index_after
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_after);
                p == b"first"
            })
            .expect("first in index after apply");
        let blob = repo.find_object(entry.id)?;
        assert_eq!(
            blob.data, b"c-git-stash-content\n",
            "gix apply should restore C Git stash content"
        );

        Ok(())
    }

    /// Create multiple stashes with gix, verify C Git sees the correct count and order.
    #[test]
    fn interop_multiple_gix_stashes_visible_to_c_git() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // First stash.
        repo.stash_save(Some("first-gix"))?;

        // Create new change for second stash.
        std::fs::write(workdir.join("file.txt"), "second change\n")?;
        let blob_id = repo.write_blob("second change\n")?;
        let mut index = repo.open_index()?;
        for (entry, entry_path) in index.entries_mut_with_paths() {
            let p: &[u8] = &**entry_path;
            if p == b"file.txt" {
                entry.id = blob_id.detach();
                break;
            }
        }
        index.write(Default::default())?;

        repo.stash_save(Some("second-gix"))?;

        let output = std::process::Command::new("git")
            .args(["stash", "list"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 2, "C Git should see 2 stashes: {stdout}");
        assert!(
            lines[0].contains("second-gix"),
            "stash@{{0}} should be second-gix: {}",
            lines[0]
        );
        assert!(
            lines[1].contains("first-gix"),
            "stash@{{1}} should be first-gix: {}",
            lines[1]
        );

        Ok(())
    }

    /// Interop: stash with include_untracked created by gix has correct 3-parent structure
    /// visible to C Git's `git stash show --include-untracked`.
    #[test]
    fn interop_gix_untracked_stash_visible_to_c_git() -> crate::Result {
        use gix::repository::stash::StashSaveOptions;

        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        std::fs::write(workdir.join("untracked-interop.txt"), "untracked\n")?;

        repo.stash_save_opts(StashSaveOptions {
            message: Some("untracked-interop"),
            keep_index: false,
            include_untracked: true,
        })?;

        // C Git should see the stash and the third parent.
        let output = std::process::Command::new("git")
            .args(["rev-parse", "stash^3"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH");
        assert!(
            output.status.success(),
            "stash^3 should exist (untracked commit): {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Verify the untracked file is in the third parent tree.
        let output = std::process::Command::new("git")
            .args(["ls-tree", "--name-only", "stash^3:"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("untracked-interop.txt"),
            "C Git should see untracked file in stash^3: {stdout}"
        );

        Ok(())
    }

    // ========================================================================
    // Additional test coverage from C Git t3903 / t3905
    // ========================================================================

    /// Stash when a tracked file has been deleted from the worktree.
    /// Corresponds to C Git test: "stash save --include-untracked removed files".
    #[test]
    fn stash_save_with_deleted_worktree_file() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Remove the tracked file from the worktree (but it's still in the index).
        std::fs::remove_file(workdir.join("file.txt"))?;

        // Stash should still succeed because the index differs from HEAD.
        let stash_id = repo.stash_save(None)?;

        // After stash, the file should be restored (reset to HEAD).
        assert!(
            workdir.join("file.txt").exists(),
            "file.txt should be restored after stash save (hard reset to HEAD)"
        );
        let restored = std::fs::read_to_string(workdir.join("file.txt"))?;
        assert_eq!(restored.trim(), "hello", "restored file should match HEAD content");

        // The stash commit tree should capture the staged (modified) content,
        // since the index had "modified" even though the worktree file was deleted.
        let stash_commit = repo.find_object(stash_id)?.try_into_commit().expect("commit");
        let stash_tree_id = stash_commit.tree_id().expect("has tree");
        let stash_tree = repo.find_object(stash_tree_id)?.try_into_tree().expect("tree");
        let stash_tree_decoded = stash_tree.decode().expect("decoded");
        let entry = stash_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "file.txt".as_bytes())
            .expect("file.txt in stash tree");
        let blob = repo.find_object(entry.oid)?;
        // The worktree file was deleted, so build_worktree_tree would not find it on disk
        // and should fall through to the index version ("modified").
        assert_eq!(
            blob.data, b"modified\n",
            "stash tree should capture the index version when worktree file is deleted"
        );

        Ok(())
    }

    /// Stash binary file changes.
    /// Corresponds to C Git test: "stash --staged with binary file".
    #[test]
    fn stash_save_binary_file() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Write a binary file (contains null bytes).
        let binary_content: Vec<u8> = vec![0x00, 0x01, 0x02, 0xFF, 0xFE, 0x00, 0x42];
        std::fs::write(workdir.join("binary.bin"), &binary_content)?;

        // Add to index.
        let blob_id = repo.write_blob(&binary_content)?;
        let mut index = repo.open_index()?;
        use gix::bstr::ByteSlice;
        index.add_entry(
            gix_index::entry::Stat::default(),
            blob_id.detach(),
            gix_index::entry::Flags::empty(),
            gix_index::entry::Mode::FILE,
            b"binary.bin".as_bstr(),
        );
        index.sort_entries();
        index.write(Default::default())?;

        let stash_id = repo.stash_save(Some("binary stash"))?;

        // Verify the stash tree has the binary file.
        let stash_commit = repo.find_object(stash_id)?.try_into_commit().expect("commit");
        let stash_tree_id = stash_commit.tree_id().expect("has tree");
        let stash_tree = repo.find_object(stash_tree_id)?.try_into_tree().expect("tree");
        let stash_tree_decoded = stash_tree.decode().expect("decoded");
        let entry = stash_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "binary.bin".as_bytes())
            .expect("binary.bin in stash tree");
        let blob = repo.find_object(entry.oid)?;
        assert_eq!(
            blob.data, binary_content,
            "stash should preserve binary content exactly"
        );

        // After save, binary.bin should not be in index (reset to HEAD).
        let index_after = repo.open_index()?;
        let has_binary = index_after.entries().iter().any(|e| {
            let p: &[u8] = &**e.path(&index_after);
            p == b"binary.bin"
        });
        assert!(
            !has_binary,
            "binary.bin should be removed from index after stash (reset to HEAD)"
        );

        // Apply and verify binary content is restored.
        repo.stash_apply(0)?;
        let index_restored = repo.open_index()?;
        let restored_entry = index_restored
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_restored);
                p == b"binary.bin"
            })
            .expect("binary.bin should be restored after apply");
        let restored_blob = repo.find_object(restored_entry.id)?;
        assert_eq!(
            restored_blob.data, binary_content,
            "restored binary content should match original"
        );

        Ok(())
    }

    /// Apply does not need a perfectly clean working directory.
    /// Unrelated modified files should not block stash apply.
    /// Corresponds to C Git test: "apply does not need clean working directory".
    #[test]
    fn stash_apply_with_unrelated_worktree_changes() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Stash the staged "modified" in file.txt, reset to HEAD "hello".
        repo.stash_save(None)?;

        // Now make a change to a DIFFERENT (new) file in the worktree.
        // This simulates the C Git test where "other-file" has changes.
        std::fs::write(workdir.join("other-unrelated.txt"), "unrelated change\n")?;

        // Apply should succeed because file.txt is clean (matches HEAD).
        repo.stash_apply(0)?;

        // Verify the stash was applied.
        let index_after = repo.open_index()?;
        let entry = index_after
            .entries()
            .iter()
            .find(|e| {
                let p: &[u8] = &**e.path(&index_after);
                p == b"file.txt"
            })
            .expect("file.txt in index");
        let blob = repo.find_object(entry.id)?;
        assert_eq!(blob.data, b"modified\n", "stashed content should be applied");

        // The unrelated worktree file should still be there.
        assert!(
            workdir.join("other-unrelated.txt").exists(),
            "unrelated worktree file should not be affected"
        );

        Ok(())
    }

    /// Stash message format: verify "On <branch>: <message>" format.
    /// Corresponds to C Git test: "push -m shows right message".
    #[test]
    fn stash_message_format_matches_c_git() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        repo.stash_save(Some("test message"))?;

        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 1);

        let msg = entries[0].message.to_string();
        // C Git format: "On main: test message"
        assert!(
            msg.contains("On main: test message"),
            "stash message should match C Git format 'On <branch>: <msg>': {msg}"
        );

        Ok(())
    }

    /// Multiline custom messages should have newlines replaced with spaces in the reflog.
    /// C Git and libgit2 sanitize reflog entries since they are line-oriented.
    #[test]
    fn stash_multiline_message_sanitized_in_reflog() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        repo.stash_save(Some("line one\nline two\rline three"))?;

        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 1);

        let msg = entries[0].message.to_string();
        // Newlines and carriage returns should be replaced with spaces.
        assert!(
            !msg.contains('\n'),
            "reflog message should not contain newlines: {msg:?}"
        );
        assert!(
            !msg.contains('\r'),
            "reflog message should not contain carriage returns: {msg:?}"
        );
        assert!(
            msg.contains("line one line two line three"),
            "reflog message should have newlines replaced with spaces: {msg:?}"
        );

        Ok(())
    }

    /// Stash WIP message format when no custom message is provided.
    /// C Git format: "WIP on <branch>: <short-hash> <commit message>"
    #[test]
    fn stash_wip_message_format() -> crate::Result {
        let (repo, _tmp) = repo_rw_stash()?;

        let head_id_hex = repo.head_id()?.to_string();
        let head_short = &head_id_hex[..7];

        repo.stash_save(None)?;

        let entries = repo.stash_list()?;
        let msg = entries[0].message.to_string();
        assert!(
            msg.starts_with("WIP on main: "),
            "WIP message should start with 'WIP on main: ': {msg}"
        );
        assert!(
            msg.contains(head_short),
            "WIP message should contain short HEAD hash {head_short}: {msg}"
        );
        assert!(
            msg.contains("initial commit"),
            "WIP message should contain commit subject: {msg}"
        );

        Ok(())
    }

    /// Untracked files in subdirectories should be captured.
    /// Corresponds to C Git t3905: untracked files in nested directories.
    #[test]
    fn stash_include_untracked_nested_directories() -> crate::Result {
        use gix::repository::stash::StashSaveOptions;

        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Create nested untracked directory structure.
        let subdir = workdir.join("subdir");
        std::fs::create_dir_all(&subdir)?;
        std::fs::write(subdir.join("nested.txt"), "nested content\n")?;
        std::fs::write(workdir.join("toplevel.txt"), "top content\n")?;

        let stash_id = repo.stash_save_opts(StashSaveOptions {
            message: Some("nested untracked"),
            keep_index: false,
            include_untracked: true,
        })?;

        // Both files should be removed from worktree.
        assert!(
            !subdir.join("nested.txt").exists(),
            "nested untracked file should be removed"
        );
        assert!(
            !workdir.join("toplevel.txt").exists(),
            "toplevel untracked file should be removed"
        );

        // Verify the untracked commit tree contains both files.
        let stash_commit = repo.find_object(stash_id)?.into_commit();
        let parent_ids: Vec<_> = stash_commit.parent_ids().map(|id| id.detach()).collect();
        assert_eq!(parent_ids.len(), 3, "should have 3 parents with untracked");

        let untracked_commit = repo.find_object(parent_ids[2])?.into_commit();
        let untracked_tree_id = untracked_commit.tree_id().expect("has tree");

        // Build an index from the untracked tree to enumerate all entries
        // (since the tree may have nested subtrees).
        let untracked_index = repo.index_from_tree(&untracked_tree_id)?;
        let paths: Vec<String> = untracked_index
            .entries()
            .iter()
            .map(|e| {
                let p: &[u8] = &**e.path(&untracked_index);
                String::from_utf8_lossy(p).to_string()
            })
            .collect();

        assert!(
            paths.contains(&"subdir/nested.txt".to_string()),
            "untracked tree should contain subdir/nested.txt: {paths:?}"
        );
        assert!(
            paths.contains(&"toplevel.txt".to_string()),
            "untracked tree should contain toplevel.txt: {paths:?}"
        );

        // Apply and verify they come back.
        repo.stash_apply(0)?;
        assert!(
            workdir.join("toplevel.txt").exists(),
            "toplevel.txt should be restored after apply"
        );
        assert!(
            subdir.join("nested.txt").exists(),
            "subdir/nested.txt should be restored after apply"
        );
        assert_eq!(std::fs::read_to_string(subdir.join("nested.txt"))?, "nested content\n");

        Ok(())
    }

    /// Stash with multiple tracked files having different changes.
    /// Tests that all changed files are captured, not just the first one.
    #[test]
    fn stash_save_multiple_changed_files() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_repo.sh")?;
        let workdir = tmp.path().to_owned();

        // Modify both tracked files.
        std::fs::write(workdir.join("first"), "changed first\n")?;
        std::fs::write(workdir.join("second"), "changed second\n")?;

        // Stage both changes.
        let blob1 = repo.write_blob("changed first\n")?;
        let blob2 = repo.write_blob("changed second\n")?;
        let mut index = repo.open_index()?;
        for (entry, entry_path) in index.entries_mut_with_paths() {
            let p: &[u8] = &**entry_path;
            if p == b"first" {
                entry.id = blob1.detach();
            } else if p == b"second" {
                entry.id = blob2.detach();
            }
        }
        index.write(Default::default())?;

        let stash_id = repo.stash_save(Some("multi-file"))?;

        // Verify both files are in the stash tree with the modified content.
        let stash_commit = repo.find_object(stash_id)?.try_into_commit().expect("commit");
        let stash_tree_id = stash_commit.tree_id().expect("has tree");
        let stash_tree = repo.find_object(stash_tree_id)?.try_into_tree().expect("tree");
        let stash_tree_decoded = stash_tree.decode().expect("decoded");

        let first_entry = stash_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "first".as_bytes())
            .expect("first in stash tree");
        let first_blob = repo.find_object(first_entry.oid)?;
        assert_eq!(first_blob.data, b"changed first\n");

        let second_entry = stash_tree_decoded
            .entries
            .iter()
            .find(|e| e.filename == "second".as_bytes())
            .expect("second in stash tree");
        let second_blob = repo.find_object(second_entry.oid)?;
        assert_eq!(second_blob.data, b"changed second\n");

        // After stash, both should be reset to HEAD.
        let first_content = std::fs::read_to_string(workdir.join("first"))?;
        assert_eq!(first_content, "modified first\n", "first should be reset to HEAD");
        let second_content = std::fs::read_to_string(workdir.join("second"))?;
        assert_eq!(second_content, "second file\n", "second should be reset to HEAD");

        Ok(())
    }

    /// Pop restores worktree file content, not just the index.
    /// Verifies the end-to-end user experience: files on disk match the stashed state.
    #[test]
    fn stash_pop_restores_worktree_file_content() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Write a further worktree change on top of the staged change.
        std::fs::write(workdir.join("file.txt"), "worktree pop test\n")?;

        repo.stash_save(None)?;

        // After save, file.txt should be at HEAD content ("hello").
        let content_after_save = std::fs::read_to_string(workdir.join("file.txt"))?;
        assert_eq!(
            content_after_save.trim(),
            "hello",
            "after save, worktree should be reset to HEAD"
        );

        // Pop should restore the stashed state.
        repo.stash_pop(0)?;

        // The worktree file should now have the stashed worktree content.
        let content_after_pop = std::fs::read_to_string(workdir.join("file.txt"))?;
        assert_eq!(
            content_after_pop, "worktree pop test\n",
            "after pop, worktree file should have the stashed content"
        );

        Ok(())
    }

    /// Stash reflog should be properly updated for C Git compatibility.
    /// After drop, `git reflog show refs/stash` should show correct entries.
    #[test]
    fn interop_stash_drop_updates_reflog_for_c_git() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Create two stashes.
        repo.stash_save(Some("first-drop-test"))?;

        std::fs::write(workdir.join("file.txt"), "second drop change\n")?;
        let blob_id = repo.write_blob("second drop change\n")?;
        let mut index = repo.open_index()?;
        for (entry, entry_path) in index.entries_mut_with_paths() {
            let p: &[u8] = &**entry_path;
            if p == b"file.txt" {
                entry.id = blob_id.detach();
                break;
            }
        }
        index.write(Default::default())?;

        repo.stash_save(Some("second-drop-test"))?;

        // Verify C Git sees 2 stashes.
        let output = std::process::Command::new("git")
            .args(["stash", "list"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.lines().count(),
            2,
            "C Git should see 2 stashes before drop: {stdout}"
        );

        // Drop stash@{0} (the most recent).
        repo.stash_drop(0)?;

        // C Git should now see only 1 stash.
        let output = std::process::Command::new("git")
            .args(["stash", "list"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 1, "C Git should see 1 stash after drop: {stdout}");
        assert!(
            lines[0].contains("first-drop-test"),
            "remaining stash should be first-drop-test: {}",
            lines[0]
        );

        Ok(())
    }

    /// C Git creates a stash with untracked files (3-parent format),
    /// gix should be able to apply it and restore the untracked files.
    #[test]
    fn interop_c_git_untracked_stash_applied_by_gix() -> crate::Result {
        let (repo, tmp) = repo_rw("make_reset_repo.sh")?;
        let workdir = tmp.path().to_owned();

        // Create an untracked file and stash it with C Git.
        std::fs::write(workdir.join("c-untracked.txt"), "c-git untracked\n")?;

        let status = std::process::Command::new("git")
            .args(["stash", "push", "--include-untracked", "-m", "c-git-untracked"])
            .current_dir(&workdir)
            .status()
            .expect("git must be on PATH");
        assert!(status.success(), "C git stash push -u should succeed");

        // Untracked file should be removed by C Git.
        assert!(
            !workdir.join("c-untracked.txt").exists(),
            "C Git should have removed the untracked file"
        );

        // Verify gix can see the 3-parent stash.
        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 1);

        let stash_commit = repo.find_object(entries[0].commit_id.detach())?.into_commit();
        let parent_count = stash_commit.parent_ids().count();
        assert_eq!(parent_count, 3, "C Git untracked stash should have 3 parents");

        // Apply via gix -- should restore the untracked file.
        repo.stash_apply(0)?;

        assert!(
            workdir.join("c-untracked.txt").exists(),
            "gix apply should restore untracked file from C Git stash"
        );
        let content = std::fs::read_to_string(workdir.join("c-untracked.txt"))?;
        assert_eq!(
            content, "c-git untracked\n",
            "restored untracked file should have original content"
        );

        Ok(())
    }

    /// Create stash with gix, pop with C Git -- verifies round-trip interop.
    #[test]
    fn interop_gix_stash_popped_by_c_git() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Stash the staged modification via gix.
        repo.stash_save(Some("pop-by-c-git"))?;

        // Verify clean state after save.
        let content = std::fs::read_to_string(workdir.join("file.txt"))?;
        assert_eq!(content.trim(), "hello", "should be reset to HEAD");

        // Pop using C Git.
        let output = std::process::Command::new("git")
            .args(["stash", "pop"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH");
        assert!(
            output.status.success(),
            "C git stash pop should succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Verify the stashed content was restored.
        let content_after_pop = std::fs::read_to_string(workdir.join("file.txt"))?;
        assert_eq!(
            content_after_pop.trim(),
            "modified",
            "C git pop should restore gix-stashed content"
        );

        // Stash list should be empty.
        let output = std::process::Command::new("git")
            .args(["stash", "list"])
            .current_dir(&workdir)
            .output()
            .expect("git must be on PATH");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.is_empty(),
            "C Git stash list should be empty after pop: {stdout}"
        );

        Ok(())
    }

    /// Stash clear functionality: drop all stashes one by one.
    /// Verifies that dropping the last entry fully cleans up refs/stash.
    #[test]
    fn stash_drop_all_cleans_up() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Create 3 stashes.
        repo.stash_save(Some("first"))?;

        for label in &["second", "third"] {
            std::fs::write(workdir.join("file.txt"), format!("{label} change\n"))?;
            let blob_id = repo.write_blob(format!("{label} change\n").as_bytes())?;
            let mut index = repo.open_index()?;
            for (entry, entry_path) in index.entries_mut_with_paths() {
                let p: &[u8] = &**entry_path;
                if p == b"file.txt" {
                    entry.id = blob_id.detach();
                    break;
                }
            }
            index.write(Default::default())?;
            repo.stash_save(Some(label))?;
        }

        assert_eq!(repo.stash_list()?.len(), 3);

        // Drop all from the top.
        repo.stash_drop(0)?;
        assert_eq!(repo.stash_list()?.len(), 2);
        repo.stash_drop(0)?;
        assert_eq!(repo.stash_list()?.len(), 1);
        repo.stash_drop(0)?;
        assert!(repo.stash_list()?.is_empty());

        // refs/stash should be gone.
        let stash_ref = repo.try_find_reference("refs/stash")?;
        assert!(stash_ref.is_none(), "refs/stash should be gone after dropping all");

        Ok(())
    }

    /// Pop should NOT drop the stash if the apply fails due to conflicts.
    /// Corresponds to C Git test: "branch: should not drop the stash if the apply fails".
    #[test]
    fn stash_pop_preserves_stash_on_conflict() -> crate::Result {
        let (repo, tmp) = repo_rw_stash()?;
        let workdir = tmp.path().to_owned();

        // Stash the staged "modified".
        repo.stash_save(None)?;

        // Create a conflicting change.
        std::fs::write(workdir.join("file.txt"), "conflict\n")?;
        let blob_id = repo.write_blob("conflict\n")?;
        let mut index = repo.open_index()?;
        for (entry, entry_path) in index.entries_mut_with_paths() {
            let p: &[u8] = &**entry_path;
            if p == b"file.txt" {
                entry.id = blob_id.detach();
                break;
            }
        }
        index.write(Default::default())?;

        // Pop should fail due to conflict.
        let result = repo.stash_pop(0);
        assert!(result.is_err(), "pop should fail when there's a conflict");

        // The stash should still be present (not dropped).
        let entries = repo.stash_list()?;
        assert_eq!(entries.len(), 1, "stash should still exist after failed pop");

        Ok(())
    }
}
