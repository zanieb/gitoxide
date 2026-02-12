use std::path::Path;

use gix_index::State;
use gix_testtools::scripted_fixture_read_only_standalone;

fn hash_tree(tree: &gix_object::Tree) -> Result<gix_hash::ObjectId, std::io::Error> {
    use gix_object::WriteTo;
    let mut buf = Vec::new();
    tree.write_to(&mut buf)?;
    let header = format!("tree {}\0", buf.len());
    let mut hasher = gix_hash::hasher(gix_hash::Kind::Sha1);
    hasher.update(header.as_bytes());
    hasher.update(&buf);
    hasher
        .try_finalize()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

/// Test that writing a tree from an index produces the same tree as the original.
/// This is the round-trip test: tree -> index -> tree should produce the same tree id.
#[test]
fn roundtrip_tree_to_index_to_tree() -> crate::Result {
    let fixtures = [
        "make_index/v2.sh",
        "make_index/v2_more_files.sh",
        "make_index/v2_all_file_kinds.sh",
        "make_index/v4_more_files_IEOT.sh",
    ];

    for fixture in fixtures {
        let worktree_dir = scripted_fixture_read_only_standalone(fixture)?;
        let expected_tree_id = tree_id(&worktree_dir);

        let git_dir = worktree_dir.join(".git");
        let odb = gix_odb::at(git_dir.join("objects"))?;

        // Create index from the known tree
        let mut state = State::from_tree(&expected_tree_id, &odb, Default::default())?;

        // Write tree from index
        let outcome = state.write_tree_to(hash_tree)?;

        assert_eq!(
            outcome.tree_id, expected_tree_id,
            "round-trip tree->index->tree failed for {fixture}"
        );

        // Verify that the TREE extension was populated
        let tree_ext = state.tree().expect("tree extension should be set after write_tree_to");
        assert!(
            tree_ext.num_entries.is_some(),
            "tree extension should have entry count for {fixture}"
        );
        assert_eq!(
            tree_ext.id, expected_tree_id,
            "tree extension root id should match for {fixture}"
        );
    }
    Ok(())
}

/// Test writing a tree from an empty index produces the empty tree.
#[test]
fn empty_index_produces_empty_tree() -> crate::Result {
    let mut state = State::new(gix_hash::Kind::Sha1);

    let outcome = state.write_tree_to(hash_tree)?;

    let empty_tree_id = gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1);
    assert_eq!(
        outcome.tree_id, empty_tree_id,
        "empty index should produce the empty tree"
    );
    Ok(())
}

/// Test that entries with REMOVE flag are skipped when writing trees.
#[test]
fn entries_with_remove_flag_are_skipped() -> crate::Result {
    let fixture = "make_index/v2_more_files.sh";
    let worktree_dir = scripted_fixture_read_only_standalone(fixture)?;
    let git_dir = worktree_dir.join(".git");
    let odb = gix_odb::at(git_dir.join("objects"))?;
    let expected_tree_id = tree_id(&worktree_dir);

    let mut state = State::from_tree(&expected_tree_id, &odb, Default::default())?;

    // Mark an entry for removal
    let original_len = state.entries().len();
    assert!(original_len > 1, "need multiple entries for this test");
    state.entries_mut()[0].flags.insert(gix_index::entry::Flags::REMOVE);

    // Write tree -- should produce a different tree (missing one entry)
    let outcome = state.write_tree_to(hash_tree)?;

    assert_ne!(
        outcome.tree_id, expected_tree_id,
        "removing an entry should produce a different tree"
    );

    // The tree extension entry count should reflect one fewer entry
    let tree_ext = state.tree().expect("tree extension set");
    assert_eq!(
        tree_ext.num_entries,
        Some((original_len - 1) as u32),
        "tree extension should count only non-removed entries"
    );
    Ok(())
}

/// Test that sparse directory entries (Mode::DIR) use their OID directly as a tree OID
/// instead of being decomposed into sub-entries.
#[test]
fn sparse_directory_entries_use_precomputed_tree_oid() -> crate::Result {
    use bstr::ByteSlice;
    use gix_hash::ObjectId;
    use gix_index::entry;

    // Build a state with:
    // - a regular file "file.txt"
    // - a sparse directory entry "subdir" with Mode::DIR
    let mut state = State::new(gix_hash::Kind::Sha1);

    let blob_oid: ObjectId = "d670460b4b4aece5915caf5c68d12f560a9fe3e4".parse().unwrap();
    let tree_oid: ObjectId = "4b825dc642cb6eb9a060e54bf899d8856bce99be".parse().unwrap();

    state.dangerously_push_entry(
        Default::default(),
        blob_oid,
        entry::Flags::empty(),
        entry::Mode::FILE,
        b"file.txt".as_bstr(),
    );
    state.dangerously_push_entry(
        Default::default(),
        tree_oid,
        entry::Flags::empty(),
        entry::Mode::DIR,
        b"subdir".as_bstr(),
    );

    // Track the tree objects that get written.
    let mut written_trees: Vec<(ObjectId, gix_object::Tree)> = Vec::new();
    let outcome = state.write_tree_to(|tree| -> Result<ObjectId, std::io::Error> {
        let id = hash_tree(tree)?;
        written_trees.push((id, tree.clone()));
        Ok(id)
    })?;

    // The root tree should contain two entries: "file.txt" (blob) and "subdir" (tree).
    // The sparse directory "subdir" should use the pre-set tree_oid directly,
    // so the write callback should only be called once (for the root tree, not for "subdir").
    assert_eq!(
        written_trees.len(),
        1,
        "only the root tree should be written; sparse dir OID should be reused"
    );

    // The root tree should have the sparse dir entry with the correct tree_oid.
    let root_tree = &written_trees[0].1;
    assert_eq!(root_tree.entries.len(), 2, "root tree should have 2 entries");

    let subdir_entry = root_tree
        .entries
        .iter()
        .find(|e| e.filename == "subdir")
        .expect("subdir entry should exist");
    assert_eq!(
        subdir_entry.oid, tree_oid,
        "sparse directory OID should be used directly"
    );
    assert!(
        subdir_entry.mode.is_tree(),
        "sparse directory should be represented as a tree entry"
    );

    // Verify tree extension also records the sparse directory.
    let tree_ext = state.tree().expect("tree extension set");
    assert_eq!(tree_ext.id, outcome.tree_id);

    Ok(())
}

/// Test that intent-to-add entries are skipped when writing trees.
#[test]
fn intent_to_add_entries_are_skipped() -> crate::Result {
    use bstr::ByteSlice;
    use gix_hash::ObjectId;
    use gix_index::entry;

    let mut state = State::new(gix_hash::Kind::Sha1);
    let blob_oid: ObjectId = "d670460b4b4aece5915caf5c68d12f560a9fe3e4".parse().unwrap();

    state.dangerously_push_entry(
        Default::default(),
        blob_oid,
        entry::Flags::empty(),
        entry::Mode::FILE,
        b"real.txt".as_bstr(),
    );
    state.dangerously_push_entry(
        Default::default(),
        blob_oid,
        entry::Flags::INTENT_TO_ADD,
        entry::Mode::FILE,
        b"staged.txt".as_bstr(),
    );

    let outcome = state.write_tree_to(hash_tree)?;

    // The tree should only contain "real.txt", not "staged.txt".
    let tree_ext = state.tree().expect("tree extension set");
    assert_eq!(
        tree_ext.num_entries,
        Some(1),
        "only the non-intent-to-add entry should be counted"
    );

    // Build a reference state with just the one file and verify they match.
    let mut ref_state = State::new(gix_hash::Kind::Sha1);
    ref_state.dangerously_push_entry(
        Default::default(),
        blob_oid,
        entry::Flags::empty(),
        entry::Mode::FILE,
        b"real.txt".as_bstr(),
    );
    let ref_outcome = ref_state.write_tree_to(hash_tree)?;
    assert_eq!(
        outcome.tree_id, ref_outcome.tree_id,
        "intent-to-add entry should not affect tree output"
    );

    Ok(())
}

fn tree_id(root: &Path) -> gix_hash::ObjectId {
    let hex_hash =
        std::fs::read_to_string(root.join("head.tree")).expect("head.tree was created by git rev-parse @^{tree}");
    hex_hash.trim().parse().expect("valid hash")
}
