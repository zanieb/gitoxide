/// Integration tests for git notes (read-only API).
///
/// Tests cover: notes_list, note_find, note_read across default and custom refs.
mod note {
    fn repo_with_notes() -> crate::Result<gix::Repository> {
        crate::named_repo("make_note_repo.sh")
    }

    /// Get commit IDs in order: HEAD~2 (A), HEAD~1 (B), HEAD (C).
    fn commit_ids(repo: &gix::Repository) -> crate::Result<[gix_hash::ObjectId; 3]> {
        let c = repo.head_id()?.detach();
        let c_commit = repo.find_object(c)?.into_commit();
        let b = c_commit.parent_ids().next().expect("commit C has parent").detach();
        let b_commit = repo.find_object(b)?.into_commit();
        let a = b_commit.parent_ids().next().expect("commit B has parent").detach();
        Ok([a, b, c])
    }

    #[test]
    fn notes_list_returns_default_notes() -> crate::Result {
        let repo = repo_with_notes()?;
        let [commit_a, commit_b, _commit_c] = commit_ids(&repo)?;

        let entries = repo.notes_list(None)?;
        assert_eq!(entries.len(), 2, "should have 2 notes in default ref");

        let targets: Vec<_> = entries.iter().map(|e| e.target).collect();
        assert!(targets.contains(&commit_a), "should have note for commit A");
        assert!(targets.contains(&commit_b), "should have note for commit B");

        Ok(())
    }

    #[test]
    fn notes_list_custom_ref() -> crate::Result {
        let repo = repo_with_notes()?;
        let [_commit_a, _commit_b, commit_c] = commit_ids(&repo)?;

        let entries = repo.notes_list(Some("refs/notes/custom"))?;
        assert_eq!(entries.len(), 1, "should have 1 note in custom ref");
        assert_eq!(entries[0].target, commit_c);

        Ok(())
    }

    #[test]
    fn notes_list_nonexistent_ref_is_error() -> crate::Result {
        let repo = repo_with_notes()?;
        let result = repo.notes_list(Some("refs/notes/nonexistent"));
        assert!(result.is_err(), "nonexistent notes ref should error");
        Ok(())
    }

    #[test]
    fn note_find_existing() -> crate::Result {
        let repo = repo_with_notes()?;
        let [commit_a, _commit_b, _commit_c] = commit_ids(&repo)?;

        let note_id = repo.note_find(&commit_a, None)?;
        assert!(note_id.is_some(), "should find note for commit A");

        Ok(())
    }

    #[test]
    fn note_find_missing() -> crate::Result {
        let repo = repo_with_notes()?;
        let [_commit_a, _commit_b, commit_c] = commit_ids(&repo)?;

        // Commit C has no note in the default ref.
        let note_id = repo.note_find(&commit_c, None)?;
        assert!(note_id.is_none(), "commit C should have no note in default ref");

        Ok(())
    }

    #[test]
    fn note_find_in_custom_ref() -> crate::Result {
        let repo = repo_with_notes()?;
        let [_commit_a, _commit_b, commit_c] = commit_ids(&repo)?;

        let note_id = repo.note_find(&commit_c, Some("refs/notes/custom"))?;
        assert!(note_id.is_some(), "commit C should have note in custom ref");

        Ok(())
    }

    #[test]
    fn note_read_existing() -> crate::Result {
        let repo = repo_with_notes()?;
        let [commit_a, _commit_b, _commit_c] = commit_ids(&repo)?;

        let note = repo.note_read(&commit_a, None)?;
        assert!(note.is_some(), "should read note for commit A");

        let note = note.unwrap();
        assert_eq!(note.target, commit_a);
        let msg = String::from_utf8_lossy(&note.message);
        assert!(
            msg.contains("Note for commit A"),
            "note message should contain expected text, got: {msg}"
        );

        Ok(())
    }

    #[test]
    fn note_read_commit_b() -> crate::Result {
        let repo = repo_with_notes()?;
        let [_commit_a, commit_b, _commit_c] = commit_ids(&repo)?;

        let note = repo.note_read(&commit_b, None)?.expect("note for B exists");
        assert_eq!(note.target, commit_b);
        let msg = String::from_utf8_lossy(&note.message);
        assert!(
            msg.contains("Note for commit B"),
            "note message should contain expected text, got: {msg}"
        );

        Ok(())
    }

    #[test]
    fn note_read_missing_returns_none() -> crate::Result {
        let repo = repo_with_notes()?;
        let [_commit_a, _commit_b, commit_c] = commit_ids(&repo)?;

        let note = repo.note_read(&commit_c, None)?;
        assert!(note.is_none(), "commit C has no note in default ref");

        Ok(())
    }

    #[test]
    fn note_read_custom_ref() -> crate::Result {
        let repo = repo_with_notes()?;
        let [_commit_a, _commit_b, commit_c] = commit_ids(&repo)?;

        let note = repo
            .note_read(&commit_c, Some("refs/notes/custom"))?
            .expect("custom note for C exists");
        assert_eq!(note.target, commit_c);
        let msg = String::from_utf8_lossy(&note.message);
        assert!(
            msg.contains("Custom note for C"),
            "custom note message should contain expected text, got: {msg}"
        );

        Ok(())
    }

    #[test]
    fn note_ids_are_consistent() -> crate::Result {
        let repo = repo_with_notes()?;
        let [commit_a, _commit_b, _commit_c] = commit_ids(&repo)?;

        // The note_id from find and read should match.
        let find_id = repo.note_find(&commit_a, None)?.expect("note exists");
        let read_note = repo.note_read(&commit_a, None)?.expect("note exists");
        assert_eq!(find_id, read_note.note_id, "find and read should return same note_id");

        Ok(())
    }

    #[test]
    fn notes_list_entries_have_valid_note_ids() -> crate::Result {
        let repo = repo_with_notes()?;
        let entries = repo.notes_list(None)?;

        for entry in &entries {
            // Each note_id should be a readable blob.
            let obj = repo.find_object(entry.note_id)?;
            assert_eq!(obj.kind, gix_object::Kind::Blob, "note object should be a blob");
            assert!(!obj.data.is_empty(), "note blob should not be empty");
        }

        Ok(())
    }

    // ======================================================================
    // Write operation tests (ported from git/t/t3301-notes.sh)
    // These use writable repos since they modify state.
    // ======================================================================

    fn repo_rw_notes() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
        crate::util::repo_rw("make_note_repo.sh")
    }

    // t3301: "create notes" - git notes add -m "b4"
    #[test]
    fn note_add_creates_note() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [_a, _b, commit_c] = commit_ids(&repo)?;

        // Commit C has no note in default ref
        assert!(repo.note_read(&commit_c, None)?.is_none());

        // Add a note
        repo.note_add(commit_c, b"new note on C\n", None, false)?;

        // Read it back
        let note = repo.note_read(&commit_c, None)?.expect("note should exist");
        assert_eq!(note.message, b"new note on C\n");
        assert_eq!(note.target, commit_c);

        Ok(())
    }

    // t3301: notes tree has one more entry after add
    #[test]
    fn note_add_increases_list_count() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [_a, _b, commit_c] = commit_ids(&repo)?;

        let before = repo.notes_list(None)?.len();
        repo.note_add(commit_c, b"note\n", None, false)?;
        let after = repo.notes_list(None)?.len();
        assert_eq!(after, before + 1, "list should have one more entry");

        Ok(())
    }

    // t3301: "cannot git notes add -m where notes already exists"
    #[test]
    fn note_add_without_force_fails_if_exists() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [commit_a, _b, _c] = commit_ids(&repo)?;

        // commit_a already has a note from the fixture
        let result = repo.note_add(commit_a, b"conflict\n", None, false);
        assert!(result.is_err(), "add without force on existing note should fail");

        // Original note unchanged
        let note = repo.note_read(&commit_a, None)?.expect("note exists");
        assert!(
            String::from_utf8_lossy(&note.message).contains("Note for commit A"),
            "original note should be unchanged"
        );

        Ok(())
    }

    // t3301: "can overwrite existing note with git notes add -f -m"
    #[test]
    fn note_add_with_force_overwrites() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [commit_a, _b, _c] = commit_ids(&repo)?;

        // Overwrite with force
        repo.note_add(commit_a, b"overwritten\n", None, true)?;

        let note = repo.note_read(&commit_a, None)?.expect("note exists");
        assert_eq!(note.message, b"overwritten\n");

        Ok(())
    }

    // t3301: notes on different commits
    #[test]
    fn notes_on_different_commits_are_independent() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [commit_a, commit_b, commit_c] = commit_ids(&repo)?;

        // Add note to C (A and B already have notes)
        repo.note_add(commit_c, b"note on C\n", None, false)?;

        // All three should have independent notes
        let a_note = repo.note_read(&commit_a, None)?.expect("A note");
        let b_note = repo.note_read(&commit_b, None)?.expect("B note");
        let c_note = repo.note_read(&commit_c, None)?.expect("C note");

        assert!(String::from_utf8_lossy(&a_note.message).contains("Note for commit A"));
        assert!(String::from_utf8_lossy(&b_note.message).contains("Note for commit B"));
        assert_eq!(c_note.message, b"note on C\n");

        Ok(())
    }

    // t3301: "remove note with git notes remove"
    #[test]
    fn note_remove_deletes_note() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [commit_a, _b, _c] = commit_ids(&repo)?;

        // Commit A has a note
        assert!(repo.note_read(&commit_a, None)?.is_some());

        // Remove it
        repo.note_remove(&commit_a, None)?;

        // Should be gone
        assert!(repo.note_read(&commit_a, None)?.is_none());

        Ok(())
    }

    // t3301: "removing non-existing note should not create new commit"
    #[test]
    fn note_remove_nonexistent_returns_error() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [_a, _b, commit_c] = commit_ids(&repo)?;

        // Commit C has no note in default ref
        let result = repo.note_remove(&commit_c, None);
        assert!(result.is_err(), "removing non-existent note should fail");

        Ok(())
    }

    // t3301: "setup alternate notes ref"
    #[test]
    fn note_add_with_alternate_ref() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [commit_a, _b, _c] = commit_ids(&repo)?;

        // Add note to alternate ref
        repo.note_add(commit_a, b"alt note\n", Some("refs/notes/alternate"), false)?;

        // Default ref note should be unchanged
        let default_note = repo.note_read(&commit_a, None)?.expect("default note");
        assert!(String::from_utf8_lossy(&default_note.message).contains("Note for commit A"));

        // Alternate ref should have the new note
        let alt_note = repo
            .note_read(&commit_a, Some("refs/notes/alternate"))?
            .expect("alt note");
        assert_eq!(alt_note.message, b"alt note\n");

        Ok(())
    }

    // t3301: "remove note with add -f -F /dev/null" (empty note = removal)
    #[test]
    fn note_add_empty_message_removes_note() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [commit_a, _b, _c] = commit_ids(&repo)?;

        // Commit A has a note
        assert!(repo.note_read(&commit_a, None)?.is_some());

        // Add with empty message and force = removal
        repo.note_add(commit_a, b"", None, true)?;

        // Should be gone
        assert!(repo.note_read(&commit_a, None)?.is_none());

        Ok(())
    }

    // t3301: notes tree is stored under refs/notes/commits
    #[test]
    fn note_add_creates_proper_ref_structure() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [_a, _b, commit_c] = commit_ids(&repo)?;

        repo.note_add(commit_c, b"ref test\n", None, false)?;

        // The notes ref should point to a commit
        let notes_ref = repo.find_reference("refs/notes/commits")?;
        let commit_obj = notes_ref.id().object()?;
        assert_eq!(commit_obj.kind, gix_object::Kind::Commit);

        Ok(())
    }

    // t3301: removing both notes, then list should be empty
    #[test]
    fn removing_all_notes_leaves_empty_list() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [commit_a, commit_b, _c] = commit_ids(&repo)?;

        repo.note_remove(&commit_a, None)?;
        repo.note_remove(&commit_b, None)?;

        let entries = repo.notes_list(None)?;
        assert!(entries.is_empty(), "all notes removed");

        Ok(())
    }

    // t3301: multi-line notes
    #[test]
    fn note_preserves_multiline_content() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [_a, _b, commit_c] = commit_ids(&repo)?;

        let multiline = b"line1\nline2\nline3\n";
        repo.note_add(commit_c, multiline, None, false)?;

        let note = repo.note_read(&commit_c, None)?.expect("note exists");
        assert_eq!(note.message, multiline);

        Ok(())
    }

    // t3301: remove from alternate ref
    #[test]
    fn note_remove_from_alternate_ref() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [_a, _b, commit_c] = commit_ids(&repo)?;

        // Commit C has a note in custom ref
        assert!(repo.note_read(&commit_c, Some("refs/notes/custom"))?.is_some());

        // Remove from custom ref
        repo.note_remove(&commit_c, Some("refs/notes/custom"))?;

        // Custom should be gone
        assert!(repo.note_read(&commit_c, Some("refs/notes/custom"))?.is_none());

        // Default ref notes should be unaffected
        let [commit_a, _b, _c] = commit_ids(&repo)?;
        assert!(repo.note_read(&commit_a, None)?.is_some());

        Ok(())
    }

    // t3301: adding a note to a brand-new (nonexistent) ref creates it
    #[test]
    fn note_add_to_new_ref_creates_it() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [commit_a, _b, _c] = commit_ids(&repo)?;

        let new_ref = "refs/notes/brand-new";

        // The ref should not exist yet.
        assert!(
            repo.notes_list(Some(new_ref)).is_err(),
            "brand-new ref should not exist"
        );

        // Add a note to the new ref.
        repo.note_add(commit_a, b"new ref note\n", Some(new_ref), false)?;

        // Now the ref should exist and contain the note.
        let entries = repo.notes_list(Some(new_ref))?;
        assert_eq!(entries.len(), 1, "new ref should have one note");
        assert_eq!(entries[0].target, commit_a);

        let note = repo.note_read(&commit_a, Some(new_ref))?.expect("note should exist");
        assert_eq!(note.message, b"new ref note\n");

        Ok(())
    }

    // t3301: adding and then force-overwriting a note preserves only the new content
    #[test]
    fn note_force_overwrite_replaces_content() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;
        let [commit_a, _b, _c] = commit_ids(&repo)?;

        // Original note content.
        let orig = repo.note_read(&commit_a, None)?.expect("note exists");
        assert!(String::from_utf8_lossy(&orig.message).contains("Note for commit A"));

        // Overwrite with force, then overwrite again.
        repo.note_add(commit_a, b"first overwrite\n", None, true)?;
        repo.note_add(commit_a, b"second overwrite\n", None, true)?;

        let note = repo.note_read(&commit_a, None)?.expect("note exists");
        assert_eq!(
            note.message, b"second overwrite\n",
            "only the last overwrite should be present"
        );

        // The list should still have the same number of notes (2: A and B).
        let entries = repo.notes_list(None)?;
        assert_eq!(entries.len(), 2, "overwrite should not duplicate entries");

        Ok(())
    }

    // Adding a note for a nonexistent target object should fail
    #[test]
    fn note_add_to_nonexistent_target_fails() -> crate::Result {
        let (repo, _tmp) = repo_rw_notes()?;

        let fake_id = gix_hash::ObjectId::from_hex(b"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef")?;
        let result = repo.note_add(fake_id, b"nope\n", None, false);
        assert!(result.is_err(), "adding note to nonexistent object should fail");

        Ok(())
    }

    // core.notesRef config support: when set, notes operations use the configured ref
    // instead of the default refs/notes/commits
    #[test]
    fn core_notes_ref_config_is_used_as_default() -> crate::Result {
        let (mut repo, _tmp) = repo_rw_notes()?;
        let [_a, _b, commit_c] = commit_ids(&repo)?;

        // Set core.notesRef to point to the custom ref
        {
            let mut config = repo.config_snapshot_mut();
            config.set_raw_value(&"core.notesRef", "refs/notes/custom")?;
        }

        // Now, calling notes_list with None should use refs/notes/custom
        let entries = repo.notes_list(None)?;
        assert_eq!(entries.len(), 1, "custom ref has 1 note");
        assert_eq!(entries[0].target, commit_c, "the note is for commit C");

        // note_read with None should also use the custom ref
        let note = repo
            .note_read(&commit_c, None)?
            .expect("note should exist via config ref");
        let msg = String::from_utf8_lossy(&note.message);
        assert!(
            msg.contains("Custom note for C"),
            "should read from custom ref, got: {msg}"
        );

        // Explicit ref should still override config
        let [commit_a, _b, _c] = commit_ids(&repo)?;
        let default_note = repo.note_read(&commit_a, Some("refs/notes/commits"))?;
        assert!(
            default_note.is_some(),
            "explicit ref should bypass core.notesRef config"
        );

        Ok(())
    }
}
