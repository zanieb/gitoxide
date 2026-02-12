use bstr::{BString, ByteSlice};
use gix_index::entry::Stage;

use crate::index::Fixture;

fn icase_fixture() -> gix_index::File {
    Fixture::Generated("v2_icase_name_clashes").open()
}

#[test]
fn entry_by_path() {
    let file = icase_fixture();
    for entry in file.entries() {
        let path = entry.path(&file);
        assert_eq!(file.entry_by_path(path), Some(entry));
        assert_eq!(file.entry_by_path_and_stage(path, Stage::Unconflicted), Some(entry));
    }
}

#[test]
fn dirwalk_api_and_icase_support() {
    let file = Fixture::Loose("ignore-case-realistic").open();
    let icase = file.prepare_icase_backing();
    for entry in file.entries() {
        let entry_path = entry.path(&file);
        let a = file.entry_by_path_icase(entry_path, false, &icase);
        let b = file.entry_by_path_icase(entry_path, true, &icase);
        let c = file.entry_by_path_icase(entry_path.to_ascii_uppercase().as_bstr(), true, &icase);
        assert_eq!(
            a,
            b,
            "{entry_path}: an index without clashes produces exactly the same result, found {:?} and icase {:?}",
            a.map(|e| e.path(&file)),
            b.map(|e| e.path(&file))
        );
        assert_eq!(
            a,
            c,
            "{entry_path}: lower-case lookups work as well, found {:?} and icase {:?}",
            a.map(|e| e.path(&file)),
            c.map(|e| e.path(&file))
        );

        let mut last_pos = 0;
        while let Some(slash_idx) = entry_path[last_pos..].find_byte(b'/') {
            last_pos += slash_idx;
            let dir = entry_path[..last_pos].as_bstr();
            last_pos += 1;

            let entry = file
                .entry_closest_to_directory_or_directory(dir)
                .unwrap_or_else(|| panic!("didn't find {dir}"));
            assert!(
                entry.path(&file).starts_with(dir),
                "entry must actually be inside of directory"
            );

            let dir_upper: BString = dir.to_ascii_uppercase().into();
            let other_entry = file
                .entry_closest_to_directory_or_directory_icase(dir_upper.as_bstr(), true, &icase)
                .unwrap_or_else(|| panic!("didn't find upper-cased {dir_upper}"));
            assert_eq!(other_entry, entry, "the first entry is always the same, no matter what kind of search is conducted (as there are no clashes/ambiguities here)");
        }
    }
}

#[test]
fn entry_closest_to_directory_or_directory_with_submodule() {
    let file = Fixture::Generated("v2_all_file_kinds").open();

    assert!(
        file.entry_closest_to_directory_or_directory("d".into()).is_some(),
        "this is a directory"
    );
    assert!(
        file.entry_closest_to_directory_or_directory("sub".into()).is_some(),
        "this is a checked in repository, a directory itself"
    );
    assert!(
        file.entry_closest_to_directory_or_directory("sub-worktree".into())
            .is_some(),
        "a submodule that is officially registered, absolutely the same as 'sub' in the index."
    );
    assert!(
        file.entry_closest_to_directory_or_directory("a".into()).is_none(),
        "'a' is a file, and we ask for a directory"
    );
}

#[test]
fn entry_closest_to_directory_or_directory_icase_with_submodule() {
    let file = Fixture::Generated("v2_all_file_kinds").open();
    let icase = file.prepare_icase_backing();

    assert!(
        file.entry_closest_to_directory_or_directory_icase("D".into(), true, &icase)
            .is_some(),
        "this is a directory"
    );
    assert!(file
        .entry_closest_to_directory_or_directory_icase("D".into(), false, &icase)
        .is_none());

    assert!(
        file.entry_closest_to_directory_or_directory_icase("SuB".into(), true, &icase)
            .is_some(),
        "this is a checked in repository, a directory itself"
    );
    assert!(file
        .entry_closest_to_directory_or_directory_icase("SuB".into(), false, &icase)
        .is_none());

    assert!(
        file.entry_closest_to_directory_or_directory_icase("SUB-worktree".into(), true, &icase)
            .is_some(),
        "a submodule that is officially registered, absolutely the same as 'sub' in the index."
    );
    assert!(file
        .entry_closest_to_directory_or_directory_icase("SUB-worktree".into(), false, &icase)
        .is_none());

    assert!(
        file.entry_closest_to_directory_or_directory_icase("A".into(), true, &icase)
            .is_none(),
        "'a' is a file, and we ask for a directory"
    );
}

#[test]
fn ignorecase_clashes_and_order() {
    let file = icase_fixture();
    let icase = file.prepare_icase_backing();
    for entry in file.entries() {
        let entry_path = entry.path(&file);
        let a = file.entry_by_path_icase(entry_path, false, &icase);
        assert_eq!(
            a,
            Some(entry),
            "{entry_path}: in a case-sensitive search, we get exact matches, found {:?} ",
            a.map(|e| e.path(&file)),
        );

        let mut last_pos = 0;
        while let Some(slash_idx) = entry_path[last_pos..].find_byte(b'/') {
            last_pos += slash_idx;
            let dir = entry_path[..last_pos].as_bstr();
            last_pos += 1;

            let entry = file
                .entry_closest_to_directory_or_directory(dir)
                .unwrap_or_else(|| panic!("didn't find {dir}"));
            assert!(
                entry.path(&file).starts_with(dir),
                "entry must actually be inside of directory"
            );
        }
    }
    assert_eq!(
        file.entry_by_path_icase("file_x".into(), true, &icase)
            .map(|e| e.path(&file))
            .expect("in index"),
        "FILE_X",
        "it finds the entry that was inserted first"
    );

    assert_eq!(
        file.entry_by_path_icase("x".into(), true, &icase)
            .map(|e| e.path(&file))
            .expect("in index"),
        "X",
        "the file 'X' was inserted first, no way to see the symlink under 'x'"
    );

    assert!(
        file.entry_closest_to_directory_or_directory("d".into()).is_none(),
        "this is a file, and this directory search isn't case-sensitive"
    );
    let entry = file.entry_closest_to_directory_or_directory("D".into());
    assert_eq!(
        entry.map(|e| e.path(&file)).expect("present"),
        "D/B",
        "this is a directory, indeed, we find the first file in it"
    );
    let entry_icase = file.entry_closest_to_directory_or_directory_icase("d".into(), true, &icase);
    assert_eq!(
        entry_icase, entry,
        "case-insensitive searches don't confuse directories and files, so `d` finds `D`, the directory."
    );
}

#[test]
fn prefixed_entries_icase_with_name_clashes() {
    let file = icase_fixture();
    assert_eq!(
        file.prefixed_entries_range("file".into()),
        Some(7..9),
        "case sensitive search yields only two: file_x and file_X"
    );
}

#[test]
fn entry_by_path_and_stage() {
    let file = Fixture::Generated("v4_more_files_IEOT").open();
    for entry in file.entries() {
        let path = entry.path(&file);
        assert_eq!(
            file.entry_index_by_path_and_stage(path, Stage::Unconflicted)
                .map(|idx| &file.entries()[idx]),
            Some(entry)
        );
        assert_eq!(file.entry_by_path_and_stage(path, Stage::Unconflicted), Some(entry));
    }
}

#[test]
fn entry_by_path_with_conflicting_file() {
    let file = Fixture::Loose("conflicting-file").open();
    for expected_stage in [Stage::Base, Stage::Ours, Stage::Theirs] {
        assert!(
            file.entry_by_path_and_stage("file".into(), expected_stage).is_some(),
            "we have no stage 0 during a conflict, but all other ones. Missed {expected_stage:?}"
        );
    }

    assert_eq!(
        file.entry_by_path("file".into()).expect("found").stage(),
        Stage::Ours,
        "we always find our stage while in a merge"
    );
}

#[test]
fn prefixed_entries_with_multi_stage_file() {
    let file = Fixture::Loose("conflicting-file").open();

    assert_eq!(
        file.prefixed_entries("fil".into()).expect("present"),
        file.entries(),
        "it's possible to get the entire range"
    );
    assert_eq!(
        file.prefixed_entries("file".into()).expect("present"),
        file.entries(),
        "it's possible to get the entire range even if the same path matches multiple times"
    );
    assert_eq!(
        file.prefixed_entries("".into()).expect("present"),
        file.entries(),
        "empty prefix matches all"
    );
    assert_eq!(file.prefixed_entries_range("".into()), Some(0..3));
    assert_eq!(file.prefixed_entries_range("foo".into()), None);
}

#[test]
fn entry_range() {
    let file = Fixture::Loose("conflicting-file").open();

    assert_eq!(
        file.entry_range("file".into()),
        Some(0..3),
        "three stages, all but stage zero"
    );
    assert_eq!(file.entry_range("foo".into()), None, "does not exist");
}

#[test]
fn remove_entries() {
    let mut file = Fixture::Loose("conflicting-file").open();

    file.remove_entries(|idx, _, _| idx == 0);
    assert_eq!(file.entries().len(), 2);
    file.remove_entries(|idx, _, _| idx == 0);
    assert_eq!(file.entries().len(), 1);
    file.remove_entries(|idx, _, _| idx == 0);
    assert_eq!(file.entries().len(), 0);
    file.remove_entries(|_, _, _| unreachable!("should not be called"));
}

#[test]
fn remove_entry_at_index() {
    let mut file = Fixture::Loose("conflicting-file").open();

    file.remove_entry_at_index(0);
    assert_eq!(file.entries().len(), 2);
    file.remove_entry_at_index(0);
    assert_eq!(file.entries().len(), 1);
    file.remove_entry_at_index(0);
    assert_eq!(file.entries().len(), 0);
}

#[test]
fn sort_entries() {
    let mut file = Fixture::Generated("v4_more_files_IEOT").open();
    assert!(file.verify_entries().is_ok());
    let valid_entries = file.entries().len();

    let entry = file.entry(0).clone();
    let new_entry_path = "an initially incorrectly ordered entry".into();
    file.dangerously_push_entry(entry.stat, entry.id, entry.flags, entry.mode, new_entry_path);
    assert!(file.verify_entries().is_err(), "sort order doesn't fit anymore");

    for (idx, entry) in file.entries()[..valid_entries].iter().enumerate() {
        assert_eq!(
            file.entry_index_by_path_and_stage_bounded(entry.path(&file), Stage::Unconflicted, valid_entries),
            Some(idx),
            "we can still find entries in the correctly sorted region"
        );
    }
    assert_eq!(
        file.entry_by_path_and_stage(new_entry_path, Stage::Unconflicted),
        None,
        "new entry can't be found due to incorrect order"
    );

    file.sort_entries();
    assert!(file.verify_entries().is_ok(), "sorting of entries restores invariants");

    assert_eq!(
        file.entry_by_path_and_stage(new_entry_path, Stage::Unconflicted)
            .expect("can be found")
            .path(&file),
        new_entry_path,
        "we can find the correct entry now"
    );

    check_prefix(
        &file,
        "d",
        &["d/a", "d/b", "d/c", "d/last/123", "d/last/34", "d/last/6"],
    );
    check_prefix(
        &file,
        "d/",
        &["d/a", "d/b", "d/c", "d/last/123", "d/last/34", "d/last/6"],
    );
    check_prefix(&file, "d/last", &["d/last/123", "d/last/34", "d/last/6"]);
    check_prefix(&file, "d/last/", &["d/last/123", "d/last/34", "d/last/6"]);
    check_prefix(&file, "d/las", &["d/last/123", "d/last/34", "d/last/6"]);
    check_prefix(&file, "d/last/123", &["d/last/123"]);
    check_prefix(&file, "d/last/34", &["d/last/34"]);
    check_prefix(&file, "d/last/6", &["d/last/6"]);
    check_prefix(&file, "x", &["x"]);
    check_prefix(&file, "a", &["a", "an initially incorrectly ordered entry"]);
}

#[test]
fn prefixed_entries() {
    let mut file = Fixture::Generated("v4_more_files_IEOT").open();
    let entry = file.entry(0).clone();
    let new_entry_path = "an initially incorrectly ordered entry".into();
    file.dangerously_push_entry(entry.stat, entry.id, entry.flags, entry.mode, new_entry_path);
    file.sort_entries();

    check_prefix(&file, "a", &["a", "an initially incorrectly ordered entry"]);
    check_prefix(&file, "an", &["an initially incorrectly ordered entry"]);
    check_prefix(
        &file,
        "an initially incorrectly ordered entry",
        &["an initially incorrectly ordered entry"],
    );
    check_prefix(&file, "x", &["x"]);
    check_prefix(&file, "b", &["b"]);
    check_prefix(&file, "c", &["c"]);

    assert_eq!(
        file.prefixed_entries_range("".into()),
        Some(0..11),
        "empty prefixes match everything"
    );
    assert!(
        file.prefixed_entries_range("foo".into()).is_none(),
        "there is no match for this prefix"
    );
}

fn check_prefix(index: &gix_index::State, prefix: &str, expected: &[&str]) {
    assert_eq!(
        index
            .prefixed_entries(prefix.into())
            .unwrap_or_else(|| panic!("{prefix:?} must match at least one entry"))
            .iter()
            .map(|e| e.path(index))
            .collect::<Vec<_>>(),
        expected,
        "{prefix:?}"
    );
}

#[test]
fn path_is_directory_with_submodule() {
    let file = Fixture::Generated("v2_all_file_kinds").open();

    assert!(file.path_is_directory("sub-worktree".into()), "a submodule worktree");
    assert!(file.path_is_directory("d".into()), "a single-letter directory");
    assert!(
        file.path_is_directory("sub".into()),
        "this is the parent repository, and it was added as well"
    );
    assert!(
        !file.path_is_directory("su".into()),
        "just a sub-string of the directory which doesn't match"
    );
    assert!(
        !file.path_is_directory("a".into()),
        "a one-letter file isn't a directory"
    );
}

#[test]
fn path_is_directory() {
    let file = Fixture::Loose("ignore-case-realistic").open();

    // Test that directories containing entries are detected
    assert!(
        file.path_is_directory("tests".into()),
        "tests is a directory containing entries"
    );
    assert!(
        file.path_is_directory("tests/snapshots".into()),
        "tests/snapshots is a directory containing entries"
    );
    assert!(
        file.path_is_directory("tests/snapshots/porcelain".into()),
        "tests/snapshots/porcelain is a directory"
    );
    assert!(
        file.path_is_directory("tests/tools".into()),
        "tests/tools is a directory"
    );

    // Test that non-existent directories return false
    assert!(
        !file.path_is_directory("nonexistent".into()),
        "nonexistent is not a directory"
    );
    assert!(!file.path_is_directory("z".into()), "z is not a directory");
    assert!(
        !file.path_is_directory("test".into()),
        "test is not a directory (tests is)"
    );

    // Test that files are not directories
    assert!(
        !file.path_is_directory("tests/utilities.sh".into()),
        "tests/utilities.sh is a file, not a directory"
    );

    // Test that partial directory names don't match
    assert!(!file.path_is_directory("".into()), "empty path is not a directory");
}

#[test]
fn path_is_directory_icase() {
    let file = Fixture::Loose("ignore-case-realistic").open();
    let icase = file.prepare_icase_backing();

    // Test case-sensitive matching
    assert!(
        file.path_is_directory_icase("tests".into(), false, &icase),
        "tests is a directory (case-sensitive)"
    );
    assert!(
        file.path_is_directory_icase("tests/tools".into(), false, &icase),
        "tests/tools is a directory (case-sensitive)"
    );

    // Test case-insensitive matching
    assert!(
        file.path_is_directory_icase("TESTS".into(), true, &icase),
        "TESTS is a directory (case-insensitive, matches 'tests')"
    );
    assert!(
        file.path_is_directory_icase("tests/TOOLS".into(), true, &icase),
        "tests/TOOLS is a directory (case-insensitive, matches 'tests/tools')"
    );
    assert!(
        file.path_is_directory_icase("TESTS/SNAPSHOTS".into(), true, &icase),
        "TESTS/SNAPSHOTS is a directory (case-insensitive)"
    );

    // Test that non-existent paths return false even with icase
    assert!(
        !file.path_is_directory_icase("nonexistent".into(), true, &icase),
        "nonexistent is not a directory even with icase"
    );
    assert!(
        !file.path_is_directory_icase("Z".into(), true, &icase),
        "Z is not a directory even with icase"
    );
}

#[test]
fn path_is_directory_icase_with_submodule() {
    let file = Fixture::Generated("v2_all_file_kinds").open();
    let icase = file.prepare_icase_backing();

    assert!(
        file.path_is_directory_icase("SUB-worktree".into(), true, &icase),
        "a submodule worktree"
    );
    assert!(!file.path_is_directory_icase("SUB-worktree".into(), false, &icase));

    assert!(
        file.path_is_directory_icase("D".into(), true, &icase),
        "a single-letter directory"
    );
    assert!(!file.path_is_directory_icase("D".into(), false, &icase));

    assert!(
        file.path_is_directory_icase("SuB".into(), true, &icase),
        "this is the parent repository, and it was added as well"
    );
    assert!(!file.path_is_directory_icase("SuB".into(), false, &icase));

    assert!(
        !file.path_is_directory_icase("Su".into(), true, &icase),
        "just a sub-string of the directory which doesn't match"
    );
    assert!(
        !file.path_is_directory_icase("A".into(), true, &icase),
        "a one-letter file isn't a directory"
    );
    assert!(
        !file.path_is_directory_icase("a".into(), true, &icase),
        "a one-letter file isn't a directory, even with correct case"
    );
}

#[test]
fn path_is_directory_icase_with_clashes() {
    let file = icase_fixture();
    let icase = file.prepare_icase_backing();

    // Test directory detection with case clashes
    assert!(
        file.path_is_directory_icase("D".into(), false, &icase),
        "D is a directory (case-sensitive)"
    );
    assert!(
        file.path_is_directory_icase("d".into(), true, &icase),
        "d matches D directory (case-insensitive)"
    );

    // Test that files aren't detected as directories
    assert!(
        !file.path_is_directory_icase("X".into(), false, &icase),
        "X is a file, not a directory"
    );
    assert!(
        !file.path_is_directory_icase("x".into(), false, &icase),
        "x is a symlink, not a directory"
    );
}

#[test]
fn add_entry_inserts_at_correct_position() {
    let mut state = gix_index::State::new(gix_hash::Kind::Sha1);
    let stat = gix_index::entry::Stat::default();
    let id = gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let flags = gix_index::entry::Flags::empty();
    let mode = gix_index::entry::Mode::FILE;

    // Insert entries out of order -- add_entry should place them correctly.
    assert!(state.add_entry(stat, id, flags, mode, "c.txt".into()).is_none());
    assert!(state.add_entry(stat, id, flags, mode, "a.txt".into()).is_none());
    assert!(state.add_entry(stat, id, flags, mode, "b.txt".into()).is_none());

    assert_eq!(state.entries().len(), 3);
    let paths: Vec<&bstr::BStr> = state.entries().iter().map(|e| e.path(&state)).collect();
    assert_eq!(paths, &["a.txt", "b.txt", "c.txt"], "entries must be sorted");
    assert!(state.verify_entries().is_ok(), "sort invariant must hold");
}

#[test]
fn add_entry_replaces_existing() {
    let mut state = gix_index::State::new(gix_hash::Kind::Sha1);
    let stat = gix_index::entry::Stat::default();
    let id_a = gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let id_b = gix_hash::ObjectId::from_hex(b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
    let flags = gix_index::entry::Flags::empty();
    let mode = gix_index::entry::Mode::FILE;

    assert!(
        state.add_entry(stat, id_a, flags, mode, "file.txt".into()).is_none(),
        "first insert returns None"
    );
    let old = state
        .add_entry(stat, id_b, flags, mode, "file.txt".into())
        .expect("replacing returns old entry");
    assert_eq!(old.id, id_a);
    assert_eq!(state.entries().len(), 1, "entry was replaced, not added");
    assert_eq!(state.entries()[0].id, id_b);
}

#[test]
fn add_entry_different_stages_coexist() {
    use gix_index::entry::{Flags, Stage};

    let mut state = gix_index::State::new(gix_hash::Kind::Sha1);
    let stat = gix_index::entry::Stat::default();
    let id = gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let mode = gix_index::entry::Mode::FILE;

    // Add entries at different stages for the same path.
    state.add_entry(
        stat,
        id,
        Flags::from_stage(Stage::Unconflicted),
        mode,
        "conflict".into(),
    );
    state.add_entry(stat, id, Flags::from_stage(Stage::Ours), mode, "conflict".into());
    state.add_entry(stat, id, Flags::from_stage(Stage::Theirs), mode, "conflict".into());

    assert_eq!(state.entries().len(), 3, "three stages for the same path");
    assert!(state.verify_entries().is_ok(), "sort invariant must hold");
}

#[test]
fn add_entry_invalidates_tree_extension() {
    let worktree_dir =
        gix_testtools::scripted_fixture_read_only_standalone("make_index/v2_more_files.sh").expect("fixture works");
    let git_dir = worktree_dir.join(".git");
    let odb = gix_odb::at(git_dir.join("objects")).expect("odb");
    let head_tree_hex = std::fs::read_to_string(worktree_dir.join("head.tree")).expect("head.tree exists");
    let tree_id: gix_hash::ObjectId = head_tree_hex.trim().parse().expect("valid hash");

    let mut state = gix_index::State::from_tree(&tree_id, &odb, Default::default()).expect("from_tree");

    // Manually set up a tree extension by writing it once.
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
    state.write_tree_to(hash_tree).expect("write_tree_to");
    assert!(
        state.tree().expect("tree extension set").num_entries.is_some(),
        "tree extension is valid before mutation"
    );

    // Now add an entry -- the root tree should be invalidated.
    let stat = gix_index::entry::Stat::default();
    let id = gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    state.add_entry(
        stat,
        id,
        gix_index::entry::Flags::empty(),
        gix_index::entry::Mode::FILE,
        "new_file.txt".into(),
    );

    assert_eq!(
        state.tree().expect("tree extension still present").num_entries,
        None,
        "root tree node should be invalidated after adding an entry"
    );
}

#[test]
fn add_entry_invalidates_tree_extension_for_nested_path() {
    let mut state = gix_index::State::new(gix_hash::Kind::Sha1);
    let stat = gix_index::entry::Stat::default();
    let id = gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let flags = gix_index::entry::Flags::empty();
    let mode = gix_index::entry::Mode::FILE;

    // Populate some entries.
    state.add_entry(stat, id, flags, mode, "a/b/c.txt".into());
    state.add_entry(stat, id, flags, mode, "a/d.txt".into());
    state.add_entry(stat, id, flags, mode, "x.txt".into());

    // Build the TREE extension by writing.
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
    state.write_tree_to(hash_tree).expect("write_tree_to");
    let tree = state.tree().expect("tree ext");
    assert!(tree.num_entries.is_some(), "root valid");
    // Find child "a"
    let child_a = tree
        .children
        .iter()
        .find(|c| c.name.as_slice() == b"a")
        .expect("child a");
    assert!(child_a.num_entries.is_some(), "child a valid");
    let child_b = child_a
        .children
        .iter()
        .find(|c| c.name.as_slice() == b"b")
        .expect("child b");
    assert!(child_b.num_entries.is_some(), "child b valid");

    // Add an entry under a/b/ -- should invalidate root, a, and b but NOT other nodes.
    state.add_entry(stat, id, flags, mode, "a/b/new.txt".into());

    let tree = state.tree().expect("tree ext");
    assert_eq!(tree.num_entries, None, "root invalidated");
    let child_a = tree
        .children
        .iter()
        .find(|c| c.name.as_slice() == b"a")
        .expect("child a");
    assert_eq!(child_a.num_entries, None, "child a invalidated");
    let child_b = child_a
        .children
        .iter()
        .find(|c| c.name.as_slice() == b"b")
        .expect("child b");
    assert_eq!(child_b.num_entries, None, "child b invalidated");
}

#[test]
fn remove_entry_by_path_and_stage_basic() {
    let mut state = gix_index::State::new(gix_hash::Kind::Sha1);
    let stat = gix_index::entry::Stat::default();
    let id = gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let flags = gix_index::entry::Flags::empty();
    let mode = gix_index::entry::Mode::FILE;

    state.add_entry(stat, id, flags, mode, "a.txt".into());
    state.add_entry(stat, id, flags, mode, "b.txt".into());
    state.add_entry(stat, id, flags, mode, "c.txt".into());
    assert_eq!(state.entries().len(), 3);

    let removed = state
        .remove_entry_by_path_and_stage("b.txt".into(), gix_index::entry::Stage::Unconflicted)
        .expect("entry existed");
    assert_eq!(removed.path(&state), "b.txt");
    assert_eq!(state.entries().len(), 2);

    let paths: Vec<&bstr::BStr> = state.entries().iter().map(|e| e.path(&state)).collect();
    assert_eq!(paths, &["a.txt", "c.txt"]);
    assert!(state.verify_entries().is_ok());
}

#[test]
fn remove_entry_by_path_and_stage_returns_none_for_missing() {
    let mut state = gix_index::State::new(gix_hash::Kind::Sha1);
    let stat = gix_index::entry::Stat::default();
    let id = gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    state.add_entry(
        stat,
        id,
        gix_index::entry::Flags::empty(),
        gix_index::entry::Mode::FILE,
        "a.txt".into(),
    );

    assert!(
        state
            .remove_entry_by_path_and_stage("nonexistent.txt".into(), gix_index::entry::Stage::Unconflicted)
            .is_none(),
        "removing a non-existent entry returns None"
    );
    assert_eq!(state.entries().len(), 1, "nothing was removed");
}

#[test]
fn remove_entry_by_path_and_stage_invalidates_tree() {
    let mut state = gix_index::State::new(gix_hash::Kind::Sha1);
    let stat = gix_index::entry::Stat::default();
    let id = gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let flags = gix_index::entry::Flags::empty();
    let mode = gix_index::entry::Mode::FILE;

    state.add_entry(stat, id, flags, mode, "dir/file.txt".into());
    state.add_entry(stat, id, flags, mode, "other.txt".into());

    // Build tree extension
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
    state.write_tree_to(hash_tree).expect("write_tree_to");
    assert!(state.tree().unwrap().num_entries.is_some());

    state
        .remove_entry_by_path_and_stage("dir/file.txt".into(), gix_index::entry::Stage::Unconflicted)
        .expect("existed");

    let tree = state.tree().expect("tree ext");
    assert_eq!(tree.num_entries, None, "root is invalidated on remove");
    let dir_child = tree.children.iter().find(|c| c.name.as_slice() == b"dir");
    assert!(
        dir_child.map_or(true, |c| c.num_entries.is_none()),
        "dir subtree is invalidated"
    );
}
