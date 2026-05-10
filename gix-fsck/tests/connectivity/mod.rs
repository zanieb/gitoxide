use gix_fsck::{write_lost_found, Connectivity, Error, Options};
use gix_hash::ObjectId;
use gix_hashtable::{HashMap, HashSet};
use gix_object::{tree::EntryKind, Data, Kind};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    LazyLock,
};

use crate::hex_to_id;

fn check_missing<'a>(repo_name: &str, commits: impl IntoIterator<Item = &'a ObjectId>) -> HashMap<ObjectId, Kind> {
    let db = {
        let fixture_path = gix_testtools::scripted_fixture_read_only("make_test_repos.sh")
            .expect("fixture path")
            .join(repo_name)
            .join(".git")
            .join("objects");
        let mut db = gix_odb::at(fixture_path).expect("valid odb");
        db.refresh_never();
        db
    };

    let mut missing: HashMap<ObjectId, Kind> = HashMap::default();
    let record_missing_and_assert_no_duplicate = |oid: &ObjectId, kind: Kind| {
        missing.try_insert(*oid, kind).expect("no duplicate oid");
    };

    let mut check = Connectivity::new(db, record_missing_and_assert_no_duplicate);
    for commit in commits.into_iter() {
        check.check_commit(commit).expect("commit is present");
    }
    missing
}

fn hex_to_ids<'a>(hex_ids: impl IntoIterator<Item = &'a str>) -> Vec<ObjectId> {
    hex_ids.into_iter().map(hex_to_id).collect()
}

fn hex_to_objects<'a>(hex_ids: impl IntoIterator<Item = &'a str>, kind: Kind) -> HashMap<ObjectId, Kind> {
    hex_to_ids(hex_ids).into_iter().map(|id| (id, kind)).collect()
}

struct MemoryDb {
    object_hash: gix_hash::Kind,
    objects: HashMap<ObjectId, (Kind, Vec<u8>)>,
}

impl Default for MemoryDb {
    fn default() -> Self {
        MemoryDb {
            object_hash: gix_hash::Kind::Sha1,
            objects: HashMap::default(),
        }
    }
}

impl MemoryDb {
    #[cfg(feature = "sha256")]
    fn new(object_hash: gix_hash::Kind) -> Self {
        MemoryDb {
            object_hash,
            objects: HashMap::default(),
        }
    }

    fn insert(&mut self, kind: Kind, data: impl Into<Vec<u8>>) -> ObjectId {
        let data = data.into();
        let id = gix_object::compute_hash(self.object_hash, kind, &data).expect("hashing works");
        self.objects.insert(id, (kind, data));
        id
    }

    fn tag_data(target: ObjectId, target_kind: Kind, name: &str) -> Vec<u8> {
        format!("object {target}\ntype {target_kind}\ntag {name}\n\n{name}\n").into_bytes()
    }

    fn commit_data(tree: ObjectId, message: &str) -> Vec<u8> {
        Self::commit_data_with_parents(tree, &[], message)
    }

    fn commit_data_with_parents(tree: ObjectId, parents: &[ObjectId], message: &str) -> Vec<u8> {
        let mut data = format!("tree {tree}\n").into_bytes();
        for parent in parents {
            data.extend_from_slice(format!("parent {parent}\n").as_bytes());
        }
        data.extend_from_slice(
            format!("author A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n{message}\n").as_bytes(),
        );
        data
    }

    fn tree_data(mode: &str, filename: &str, oid: ObjectId) -> Vec<u8> {
        Self::tree_entries(&[(mode, filename, oid)])
    }

    fn tree_entries(entries: &[(&str, &str, ObjectId)]) -> Vec<u8> {
        let mut data = Vec::new();
        for (mode, filename, oid) in entries {
            data.extend_from_slice(format!("{mode} {filename}").as_bytes());
            data.push(0);
            data.extend_from_slice(oid.as_bytes());
        }
        data
    }
}

impl gix_object::Find for MemoryDb {
    fn try_find<'a>(
        &self,
        id: &gix_hash::oid,
        buffer: &'a mut Vec<u8>,
    ) -> Result<Option<Data<'a>>, gix_object::find::Error> {
        Ok(self.objects.get(id).map(|(kind, data)| {
            buffer.clear();
            buffer.extend_from_slice(data);
            Data::new_with_hash(*kind, buffer, id.kind())
        }))
    }
}

impl gix_object::Exists for MemoryDb {
    fn exists(&self, id: &gix_hash::oid) -> bool {
        self.objects.contains_key(id)
    }
}

// Get a `&Vec<ObjectID` for each commit in the test fixture repository
fn all_commits() -> &'static [ObjectId] {
    static ALL_COMMITS: LazyLock<Vec<ObjectId>> = LazyLock::new(|| {
        hex_to_ids([
            "ebed23648b19484cb1f340c4ee04dda08479188a",
            "8ff6d0f8891c3cb22827be142cc64606121d47b3",
            "734c926856a328d1168ffd7088532e0d1ad19bbe",
        ])
    });
    &ALL_COMMITS
}

#[test]
fn no_missing() {
    // The "base" repo is the original, and has every object present
    assert_eq!(check_missing("base", all_commits()), HashMap::default());
}

#[test]
fn missing_blobs() {
    // The "blobless" repo is cloned with `--filter=blob:none`, and is missing one blob
    let expected = hex_to_objects(
        [
            "4cdeaab5b01f9a9fbbb2fb6c08404cf12b7bdab1",
            "c18147dc648481eeb65dc5e66628429a64843327",
        ],
        Kind::Blob,
    );
    assert_eq!(check_missing("blobless", all_commits()), expected);
}

#[test]
fn missing_trees() {
    // The "treeless" repo is cloned with `--filter=tree:0`, and is missing two trees
    // NOTE: This repo is also missing a blob, but we have no way of knowing that, as the tree referencing it is missing
    let expected = hex_to_objects(
        [
            "20317ffa7614f49b2702a057bf2833918ea9fd24",
            "fc264b3b6875a46e9031483aeb9994a1b897ffd3",
        ],
        Kind::Tree,
    );
    assert_eq!(check_missing("treeless", all_commits()), expected);
}

#[test]
fn tag_chain_to_existing_blob_has_no_missing_objects() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"hello".to_vec());
    let tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(blob_id, Kind::Blob, "blob-tag"));
    let outer_tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(tag_id, Kind::Tag, "outer-tag"));

    let mut missing = HashMap::default();
    let mut check = Connectivity::new(&db, |oid: &ObjectId, kind: Kind| {
        missing.insert(*oid, kind);
    });
    check.check_tag(&outer_tag_id).expect("tag chain is present");

    assert_eq!(missing, HashMap::default());
}

#[test]
fn tags_report_missing_targets() {
    let mut db = MemoryDb::default();
    let missing_blob = hex_to_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let missing_tag = hex_to_id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let blob_tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(missing_blob, Kind::Blob, "missing-blob"));
    let tag_tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(missing_tag, Kind::Tag, "missing-tag"));

    let mut missing = HashMap::default();
    let mut check = Connectivity::new(&db, |oid: &ObjectId, kind: Kind| {
        missing.insert(*oid, kind);
    });
    check.check_tag(&blob_tag_id).expect("containing tag is present");
    check.check_tag(&tag_tag_id).expect("containing tag is present");

    assert_eq!(
        missing,
        [(missing_blob, Kind::Blob), (missing_tag, Kind::Tag)]
            .into_iter()
            .collect()
    );
}

#[test]
fn object_roots_are_detected_by_kind_and_traversed() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"reachable".to_vec());
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "file", blob_id));
    let commit_id = db.insert(Kind::Commit, MemoryDb::commit_data(tree_id, "commit-root"));
    let tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(commit_id, Kind::Commit, "commit-tag"));

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    check.check_object(&tag_id).expect("tag root is present");

    assert!(check.unreachable(db.objects.keys()).is_empty());
}

#[cfg(feature = "sha256")]
#[test]
fn sha256_tree_entries_are_decoded_with_object_id_kind() {
    let mut db = MemoryDb::new(gix_hash::Kind::Sha256);
    let blob_id = db.insert(Kind::Blob, b"reachable".to_vec());
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "file", blob_id));
    let commit_id = db.insert(Kind::Commit, MemoryDb::commit_data(tree_id, "commit-root"));

    let mut check = Connectivity::new(&db, |oid: &ObjectId, kind: Kind| {
        panic!("all SHA-256 objects are present, but {kind} {oid} was reported missing");
    });
    check.check_commit(&commit_id).expect("commit is connected");

    assert!(check.unreachable(db.objects.keys()).is_empty());
}

#[test]
fn reachability_paths_describe_the_first_route_to_objects() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"reachable".to_vec());
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "file", blob_id));
    let commit_id = db.insert(Kind::Commit, MemoryDb::commit_data(tree_id, "commit-root"));
    let tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(commit_id, Kind::Commit, "commit-tag"));

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    check.check_tag(&tag_id).expect("tag root is present");

    let mut expected_blob_path = format!("tag {tag_id} -> commit {commit_id} -> tree {tree_id}").into_bytes();
    expected_blob_path.extend_from_slice(b" -> file");
    let blob_path: &[u8] = check.path_to(&blob_id).expect("blob path recorded").as_ref();
    assert_eq!(blob_path, expected_blob_path.as_slice());

    check.check_commit(&commit_id).expect("seen commit is skipped");
    let commit_path: &[u8] = check.path_to(&commit_id).expect("first commit path retained").as_ref();
    assert_eq!(commit_path, format!("tag {tag_id} -> commit {commit_id}").as_bytes());
}

#[test]
fn commit_roots_report_missing_parents() {
    let mut db = MemoryDb::default();
    let tree_id = db.insert(Kind::Tree, Vec::new());
    let missing_parent_id = hex_to_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let commit_id = db.insert(
        Kind::Commit,
        MemoryDb::commit_data_with_parents(tree_id, &[missing_parent_id], "child"),
    );

    let mut missing = HashMap::default();
    let mut check = Connectivity::new(&db, |oid: &ObjectId, kind: Kind| {
        missing.insert(*oid, kind);
    });
    check.check_commit(&commit_id).expect("commit itself is present");

    let parent_path = {
        let parent_path: &[u8] = check
            .path_to(&missing_parent_id)
            .expect("parent path recorded")
            .as_ref();
        parent_path.to_owned()
    };
    drop(check);

    assert_eq!(missing, [(missing_parent_id, Kind::Commit)].into_iter().collect());
    assert_eq!(
        parent_path.as_slice(),
        format!("commit {commit_id} -> parent {missing_parent_id}").as_bytes()
    );
}

#[test]
fn shallow_commit_roots_skip_parent_traversal_but_still_check_trees() {
    let mut db = MemoryDb::default();
    let missing_blob_id = hex_to_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let missing_parent_id = hex_to_id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "file", missing_blob_id));
    let commit_id = db.insert(
        Kind::Commit,
        MemoryDb::commit_data_with_parents(tree_id, &[missing_parent_id], "shallow"),
    );
    let is_shallow_commit = |id: &ObjectId| *id == commit_id;

    let mut missing = HashMap::default();
    let mut check = Connectivity::new(&db, |oid: &ObjectId, kind: Kind| {
        missing.insert(*oid, kind);
    });
    check
        .check_commit_with_options(
            &commit_id,
            Options {
                skip_commit_parents: Some(&is_shallow_commit),
                ..Options::default()
            },
        )
        .expect("shallow boundary commit itself is present");

    let parent_was_seen = check.path_to(&missing_parent_id).is_some();
    drop(check);

    assert_eq!(missing, [(missing_blob_id, Kind::Blob)].into_iter().collect());
    assert!(
        !parent_was_seen,
        "shallow-boundary parents are intentionally not traversed"
    );
}

#[test]
fn reflog_entries_are_connectivity_roots() {
    let mut db = MemoryDb::default();
    let first_blob_id = db.insert(Kind::Blob, b"first".to_vec());
    let first_tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "first", first_blob_id));
    let first_commit_id = db.insert(Kind::Commit, MemoryDb::commit_data(first_tree_id, "first"));
    let second_blob_id = db.insert(Kind::Blob, b"second".to_vec());
    let second_tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "second", second_blob_id));
    let second_commit_id = db.insert(Kind::Commit, MemoryDb::commit_data(second_tree_id, "second"));
    let null = ObjectId::null(gix_hash::Kind::Sha1);

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    check
        .check_reflog_entries([(null, first_commit_id), (second_commit_id, null)])
        .expect("reflog roots are present");

    assert!(check.unreachable(db.objects.keys()).is_empty());
    let first_path: &[u8] = check
        .path_to(&first_commit_id)
        .expect("new reflog id path recorded")
        .as_ref();
    assert!(first_path.starts_with(format!("reflog-new {first_commit_id}").as_bytes()));
    let second_path: &[u8] = check
        .path_to(&second_commit_id)
        .expect("old reflog id path recorded")
        .as_ref();
    assert!(second_path.starts_with(format!("reflog-old {second_commit_id}").as_bytes()));
}

#[test]
fn index_entries_are_connectivity_roots() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"index-blob".to_vec());
    let nested_blob_id = db.insert(Kind::Blob, b"nested".to_vec());
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "nested", nested_blob_id));
    let null = ObjectId::null(gix_hash::Kind::Sha1);
    let submodule_commit_id = hex_to_id("cccccccccccccccccccccccccccccccccccccccc");

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all in-repository objects are present"));
    check
        .check_index_entries([
            (blob_id, EntryKind::Blob),
            (tree_id, EntryKind::Tree),
            (submodule_commit_id, EntryKind::Commit),
            (null, EntryKind::Blob),
        ])
        .expect("index roots are present");

    assert!(check.unreachable(db.objects.keys()).is_empty());
    let blob_path: &[u8] = check.path_to(&blob_id).expect("index blob path recorded").as_ref();
    assert!(blob_path.starts_with(format!("index-entry {blob_id}").as_bytes()));
    let nested_blob_path: &[u8] = check
        .path_to(&nested_blob_id)
        .expect("nested index tree blob path recorded")
        .as_ref();
    assert!(nested_blob_path.ends_with(b" -> nested"));
}

#[test]
fn index_entries_report_missing_blobs() {
    let missing_blob_id = hex_to_id("dddddddddddddddddddddddddddddddddddddddd");
    let mut missing = HashMap::default();
    let mut check = Connectivity::new(MemoryDb::default(), |oid: &ObjectId, kind: Kind| {
        missing.insert(*oid, kind);
    });

    check
        .check_index_entries([(missing_blob_id, EntryKind::BlobExecutable)])
        .expect("missing index blobs are reported through the callback");

    assert_eq!(missing, [(missing_blob_id, Kind::Blob)].into_iter().collect());
}

#[test]
fn index_tree_cache_ids_are_connectivity_roots() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"cached".to_vec());
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "cached", blob_id));
    let null = ObjectId::null(gix_hash::Kind::Sha1);

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all cached tree objects are present"));
    check
        .check_index_tree_cache([null, tree_id])
        .expect("index tree-cache roots are present");

    assert!(check.unreachable(db.objects.keys()).is_empty());
}

#[test]
fn progress_counts_previously_unseen_objects() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"hello".to_vec());
    let tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(blob_id, Kind::Blob, "blob-tag"));
    let outer_tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(tag_id, Kind::Tag, "outer-tag"));

    let progress = AtomicUsize::default();
    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    check
        .check_tag_with_options(
            &outer_tag_id,
            Options {
                progress: Some(&progress),
                ..Options::default()
            },
        )
        .expect("tag chain is present");
    check
        .check_tag_with_options(
            &outer_tag_id,
            Options {
                progress: Some(&progress),
                ..Options::default()
            },
        )
        .expect("seen tag chain is skipped");

    assert_eq!(progress.load(Ordering::Relaxed), 3);
}

#[test]
fn interruption_stops_connectivity_checks() {
    let should_interrupt = AtomicBool::new(true);
    let tag_id = hex_to_id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    let mut check = Connectivity::new(MemoryDb::default(), |_, _| unreachable!("interrupted before lookup"));
    let err = check
        .check_tag_with_options(
            &tag_id,
            Options {
                should_interrupt: Some(&should_interrupt),
                ..Options::default()
            },
        )
        .expect_err("interrupt flag stops traversal");

    assert!(matches!(err, Error::Interrupted));
}

#[test]
fn hash_validation_checks_reachable_blobs() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"hello".to_vec());
    let tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(blob_id, Kind::Blob, "blob-tag"));
    db.objects
        .get_mut(&blob_id)
        .expect("blob exists")
        .1
        .copy_from_slice(b"jello");

    let mut check = Connectivity::new(&db, |_, _| unreachable!("the corrupt blob is still present"));
    check
        .check_tag(&tag_id)
        .expect("unchecked connectivity only needs presence");

    let mut check = Connectivity::new(&db, |_, _| unreachable!("the corrupt blob is still present"));
    let err = check
        .check_tag_with_options(
            &tag_id,
            Options {
                verify_hashes: true,
                ..Options::default()
            },
        )
        .expect_err("hash mismatch is rejected");

    assert!(matches!(err, Error::Checksum(_)));
}

#[test]
fn blob_entries_reject_non_blob_objects_without_hash_validation() {
    let mut db = MemoryDb::default();
    let commit_id = db.insert(
        Kind::Commit,
        MemoryDb::commit_data(ObjectId::empty_tree(gix_hash::Kind::Sha1), "not-a-blob"),
    );
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "file", commit_id));
    let tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(tree_id, Kind::Tree, "tree-tag"));

    let mut check = Connectivity::new(&db, |_, _| unreachable!("the commit object is present"));
    let err = check
        .check_tag(&tag_id)
        .expect_err("a blob-mode tree entry must point to a blob object");

    assert!(matches!(
        err,
        gix_object::find::existing_object::Error::ObjectKind {
            oid,
            actual: Kind::Commit,
            expected: Kind::Blob,
        } if oid == commit_id
    ));
}

#[test]
fn previously_seen_object_is_revalidated_when_referenced_as_a_blob() {
    let mut db = MemoryDb::default();
    let commit_id = db.insert(
        Kind::Commit,
        MemoryDb::commit_data(ObjectId::empty_tree(gix_hash::Kind::Sha1), "not-a-blob"),
    );
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100644", "file", commit_id));

    let mut check = Connectivity::new(&db, |_, _| unreachable!("the commit object is present"));
    check.check_commit(&commit_id).expect("commit root is valid");
    let err = check
        .check_index_tree_cache([tree_id])
        .expect_err("a seen commit object still cannot satisfy a blob entry");

    assert!(matches!(
        err,
        gix_object::find::existing_object::Error::ObjectKind {
            oid,
            actual: Kind::Commit,
            expected: Kind::Blob,
        } if oid == commit_id
    ));
}

#[test]
fn previously_seen_object_is_revalidated_when_referenced_as_a_tree() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"not-a-tree".to_vec());

    let mut check = Connectivity::new(&db, |_, _| unreachable!("the blob object is present"));
    check.check_object(&blob_id).expect("blob root is valid");
    let err = check
        .check_index_tree_cache([blob_id])
        .expect_err("a seen blob object still cannot satisfy a tree entry");

    assert!(matches!(
        err,
        gix_object::find::existing_object::Error::ObjectKind {
            oid,
            actual: Kind::Blob,
            expected: Kind::Tree,
        } if oid == blob_id
    ));
}

#[test]
fn skipped_objects_are_ignored() {
    let mut db = MemoryDb::default();
    let missing_blob = hex_to_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(missing_blob, Kind::Blob, "missing-blob"));
    let skip_objects = [missing_blob].into_iter().collect::<HashSet>();

    let mut missing = HashMap::default();
    let mut check = Connectivity::new(&db, |oid: &ObjectId, kind: Kind| {
        missing.insert(*oid, kind);
    });
    check
        .check_tag_with_options(
            &tag_id,
            Options {
                skip_objects: Some(&skip_objects),
                ..Options::default()
            },
        )
        .expect("skipped missing target is ignored");

    assert_eq!(missing, HashMap::default());
}

#[test]
fn strict_mode_rejects_group_writable_tree_entries() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"hello".to_vec());
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100664", "file", blob_id));
    let tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(tree_id, Kind::Tree, "tree-tag"));

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    check.check_tag(&tag_id).expect("non-strict mode accepts the tree");

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    let err = check
        .check_tag_with_options(
            &tag_id,
            Options {
                strict: true,
                ..Options::default()
            },
        )
        .expect_err("strict mode rejects group-writable entries");

    assert!(matches!(
        err,
        Error::StrictMode {
            tree_id: actual_tree_id,
            mode,
            ..
        } if actual_tree_id == tree_id && mode.value() == 0o100664
    ));
}

#[test]
fn tree_order_validation_rejects_unsorted_tree_entries() {
    let mut db = MemoryDb::default();
    let first_blob_id = db.insert(Kind::Blob, b"first".to_vec());
    let second_blob_id = db.insert(Kind::Blob, b"second".to_vec());
    let tree_id = db.insert(
        Kind::Tree,
        MemoryDb::tree_entries(&[("100644", "z", first_blob_id), ("100644", "a", second_blob_id)]),
    );

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    let err = check
        .check_index_tree_cache_with_options(
            [tree_id],
            Options {
                validate_tree_order: true,
                ..Options::default()
            },
        )
        .expect_err("tree order validation rejects unsorted entries");

    assert!(matches!(
        err,
        Error::TreeNotSorted {
            tree_id: actual_tree_id,
            ..
        } if actual_tree_id == tree_id
    ));
}

#[test]
fn tree_order_validation_rejects_duplicate_tree_entries() {
    let mut db = MemoryDb::default();
    let first_blob_id = db.insert(Kind::Blob, b"first".to_vec());
    let second_blob_id = db.insert(Kind::Blob, b"second".to_vec());
    let tree_id = db.insert(
        Kind::Tree,
        MemoryDb::tree_entries(&[("100644", "dup", first_blob_id), ("100644", "dup", second_blob_id)]),
    );

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    let err = check
        .check_index_tree_cache_with_options(
            [tree_id],
            Options {
                validate_tree_order: true,
                ..Options::default()
            },
        )
        .expect_err("tree order validation rejects duplicate entries");

    assert!(matches!(
        err,
        Error::DuplicateTreeEntry {
            tree_id: actual_tree_id,
            ..
        } if actual_tree_id == tree_id
    ));
}

#[test]
fn tree_mode_validation_rejects_non_canonical_file_modes() {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"hello".to_vec());
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("100700", "file", blob_id));

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    let err = check
        .check_index_tree_cache_with_options(
            [tree_id],
            Options {
                validate_tree_modes: true,
                ..Options::default()
            },
        )
        .expect_err("tree mode validation rejects non-canonical file modes");

    assert!(matches!(
        err,
        Error::InvalidTreeEntryMode {
            tree_id: actual_tree_id,
            mode,
            ..
        } if actual_tree_id == tree_id && mode.value() == 0o100700
    ));
}

#[test]
fn tree_mode_validation_rejects_zero_padded_tree_modes() {
    let mut db = MemoryDb::default();
    let child_tree_id = db.insert(Kind::Tree, Vec::new());
    let tree_id = db.insert(Kind::Tree, MemoryDb::tree_data("040000", "dir", child_tree_id));

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    let err = check
        .check_index_tree_cache_with_options(
            [tree_id],
            Options {
                validate_tree_modes: true,
                ..Options::default()
            },
        )
        .expect_err("tree mode validation rejects zero-padded tree modes");

    assert!(matches!(
        err,
        Error::InvalidTreeEntryMode {
            tree_id: actual_tree_id,
            mode,
            ..
        } if actual_tree_id == tree_id && mode.value() == 0o040000
    ));
}

#[test]
fn unreachable_objects_are_reported_after_traversal() {
    let mut db = MemoryDb::default();
    let reachable_blob = db.insert(Kind::Blob, b"reachable".to_vec());
    let unreachable_blob = db.insert(Kind::Blob, b"unreachable".to_vec());
    let tag_id = db.insert(Kind::Tag, MemoryDb::tag_data(reachable_blob, Kind::Blob, "blob-tag"));

    let mut check = Connectivity::new(&db, |_, _| unreachable!("all objects are present"));
    check.check_tag(&tag_id).expect("reachable tag is present");

    assert_eq!(check.unreachable(db.objects.keys()), vec![unreachable_blob]);
}

#[test]
fn dangling_objects_are_written_to_lost_found() -> gix_testtools::Result {
    let mut db = MemoryDb::default();
    let blob_id = db.insert(Kind::Blob, b"dangling blob".to_vec());
    let commit_data = format!("tree {}\n\norphaned\n", ObjectId::empty_tree(gix_hash::Kind::Sha1));
    let commit_id = db.insert(Kind::Commit, commit_data.as_bytes().to_owned());

    let git_dir = gix_testtools::tempfile::TempDir::new()?;
    let written = write_lost_found(git_dir.path(), &db, [&commit_id, &blob_id])?;

    assert_eq!(written, 2);
    assert_eq!(
        std::fs::read(
            git_dir
                .path()
                .join("lost-found")
                .join("commit")
                .join(commit_id.to_string())
        )?,
        commit_data.as_bytes()
    );
    assert_eq!(
        std::fs::read(
            git_dir
                .path()
                .join("lost-found")
                .join("other")
                .join(blob_id.to_string())
        )?,
        b"dangling blob"
    );

    let written = write_lost_found(git_dir.path(), &db, [&commit_id, &blob_id])?;
    assert_eq!(written, 0, "existing lost-found files are left in place");
    Ok(())
}
