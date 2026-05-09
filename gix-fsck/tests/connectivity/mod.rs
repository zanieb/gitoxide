use gix_fsck::{Connectivity, Error, Options};
use gix_hash::ObjectId;
use gix_hashtable::{HashMap, HashSet};
use gix_object::{Data, Kind};
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

#[derive(Default)]
struct MemoryDb {
    objects: HashMap<ObjectId, (Kind, Vec<u8>)>,
}

impl MemoryDb {
    fn insert(&mut self, kind: Kind, data: impl Into<Vec<u8>>) -> ObjectId {
        let data = data.into();
        let id = gix_object::compute_hash(gix_hash::Kind::Sha1, kind, &data).expect("hashing works");
        self.objects.insert(id, (kind, data));
        id
    }

    fn tag_data(target: ObjectId, target_kind: Kind, name: &str) -> Vec<u8> {
        format!("object {target}\ntype {target_kind}\ntag {name}\n\n{name}\n").into_bytes()
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
            Data::new(*kind, buffer)
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
