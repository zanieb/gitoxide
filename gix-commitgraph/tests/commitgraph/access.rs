use std::path::{Path, PathBuf};

use crate::{check_common, graph_and_expected, graph_and_expected_named};

#[test]
fn single_parent() {
    let (cg, refs) = graph_and_expected("single_parent.sh", &["parent", "child"]);
    check_common(&cg, &refs);

    assert_eq!(cg.commit_at(refs["parent"].pos()).generation(), 1);
    assert_eq!(cg.commit_at(refs["child"].pos()).generation(), 2);
}

#[test]
fn corrected_commit_dates_are_available() {
    let (cg, refs) = graph_and_expected("corrected_commit_dates.sh", &["parent", "child"]);
    check_common(&cg, &refs);

    let parent = cg.commit_at(refs["parent"].pos());
    let child = cg.commit_at(refs["child"].pos());
    assert_eq!(
        parent.corrected_committer_timestamp(),
        Some(parent.committer_timestamp()),
        "root corrected commit date equals its committer timestamp"
    );
    assert_eq!(
        child.committer_timestamp(),
        parent.committer_timestamp(),
        "fixture pins both commits to the same committer timestamp"
    );
    assert_eq!(
        child.corrected_committer_timestamp(),
        parent.committer_timestamp().checked_add(1),
        "child corrected commit date is one greater than its parent's corrected commit date"
    );
}

#[test]
fn corrected_commit_date_overflow_is_available() {
    let (cg, refs) = graph_and_expected("corrected_commit_date_overflow.sh", &["parent", "child"]);
    check_common(&cg, &refs);

    let parent = cg.commit_at(refs["parent"].pos());
    let child = cg.commit_at(refs["child"].pos());
    let child_corrected = child
        .corrected_committer_timestamp()
        .expect("generation data is present");

    assert_eq!(
        child.committer_timestamp(),
        946684800,
        "fixture pins the child to a much older committer timestamp"
    );
    assert_eq!(
        parent.corrected_committer_timestamp(),
        Some(parent.committer_timestamp()),
        "root corrected commit date equals its committer timestamp"
    );
    assert_eq!(
        child_corrected,
        parent.committer_timestamp() + 1,
        "child corrected commit date is stored via GDO2 overflow"
    );
    assert!(
        child_corrected - child.committer_timestamp() > (1_u64 << 31) - 1,
        "the corrected-date offset must be too large for the 31-bit GDA2 inline form"
    );
}

#[test]
fn changed_path_filters_are_available() {
    let (cg, refs) = graph_and_expected("changed_path_filters.sh", &["base", "child"]);
    check_common(&cg, &refs);

    let base = cg.commit_at(refs["base"].pos());
    let child = cg.commit_at(refs["child"].pos());
    let base_filter = base.changed_path_filter().expect("base commit has a Bloom filter");
    let child_filter = child.changed_path_filter().expect("child commit has a Bloom filter");

    assert_eq!(base_filter.settings.hash_version, 1);
    assert!(base_filter.settings.num_hashes > 0);
    assert!(base_filter.settings.bits_per_entry > 0);
    assert_eq!(
        child_filter.settings, base_filter.settings,
        "all filters in one file share settings"
    );
    assert!(!base_filter.bytes().is_empty());
    assert!(!child_filter.bytes().is_empty());
}

#[test]
fn changed_path_filter_hash_version_two_is_available() {
    let (_tmp, path) = changed_path_filter_graph_with_bloom_hash_version(2);
    let file = gix_commitgraph::File::at(path).expect("version 2 Bloom filters are supported");

    assert_eq!(
        file.bloom_filter_settings()
            .expect("Bloom settings present")
            .hash_version,
        2
    );
    assert_eq!(
        file.commit_at(gix_commitgraph::file::Position(0))
            .changed_path_filter()
            .expect("Bloom filter present")
            .settings
            .hash_version,
        2
    );
}

#[test]
fn changed_path_filter_unknown_hash_version_is_rejected() {
    let (_tmp, path) = changed_path_filter_graph_with_bloom_hash_version(3);
    let err = gix_commitgraph::File::at(path).expect_err("unknown Bloom hash versions are rejected");

    assert!(
        err.to_string().contains("supported versions are 1 and 2"),
        "error should mention supported Bloom hash versions: {err}"
    );
}

#[test]
fn single_commit_huge_dates_generation_v2_also_do_not_allow_huge_dates() {
    let (cg, refs) = graph_and_expected_named("single_commit_huge_dates.sh", "v2", &["HEAD"]);
    let info = &refs["HEAD"];
    let actual = cg.commit_by_id(info.id).expect("present");
    assert_eq!(
        actual.committer_timestamp(),
        1,
        "overflow happened, can't represent huge dates"
    );
    assert_eq!(
        info.time.seconds, 68719476737,
        "this is the value we would want to see, but it's not possible in V2 either, as that is just about generations"
    );
    assert_eq!(actual.generation(), 1, "generations are fine though");
}

#[test]
fn single_commit_huge_dates_overflow_v1() {
    let (cg, refs) = graph_and_expected_named("single_commit_huge_dates.sh", "v1", &["HEAD"]);
    let info = &refs["HEAD"];
    let actual = cg.commit_by_id(info.id).expect("present");
    assert_eq!(actual.committer_timestamp(), 1, "overflow happened");
    assert_eq!(
        info.time.seconds, 68719476737,
        "this is the value we would want to see, but it's not possible in V1"
    );
    assert_eq!(actual.generation(), 1, "generations are fine though");
}

#[test]
fn single_commit_future_64bit_dates_work() {
    let (cg, refs) = graph_and_expected_named("single_commit_huge_dates.sh", "max-date", &["HEAD"]);
    let info = &refs["HEAD"];
    let actual = cg.commit_by_id(info.id).expect("present");
    assert_eq!(
        actual.committer_timestamp(),
        info.time.seconds.try_into().expect("timestamps in bound"),
        "this is close the highest representable value in the graph, like year 2500, so we are good for longer than I should care about"
    );
    assert_eq!(actual.generation(), 1);
}

#[test]
fn generation_numbers_overflow_is_handled_in_chained_graph() {
    let names = ["extra", "old-2", "future-2", "old-1", "future-1"];
    let (cg, mut refs) = graph_and_expected("generation_number_overflow.sh", &names);
    for (r, expected) in names
        .iter()
        .map(|n| refs.remove(n.to_owned()).expect("present"))
        .zip((1..=5).rev())
    {
        assert_eq!(
            cg.commit_by_id(r.id).expect("present").generation(),
            expected,
            "actually, this test seems to have valid generation numbers from the get-go. How to repro the actual issue?"
        );
    }
}

#[test]
fn verify_integrity_rejects_duplicate_ids_across_split_graphs() {
    let repo_dir =
        gix_testtools::scripted_fixture_read_only("split_chain.sh").expect("split-chain fixture can be created");
    let graph_dir = repo_dir.join(".git").join("objects").join("info").join("commit-graphs");
    let graph_paths = split_graph_paths(&graph_dir);
    let tmp = gix_testtools::tempfile::TempDir::new().expect("temporary directory can be created");
    let duplicate_path = graph_with_duplicated_first_oid(&graph_paths[0], &graph_paths[1], tmp.path());

    let graph = gix_commitgraph::Graph::new(vec![
        gix_commitgraph::File::at(&graph_paths[0]).expect("base graph opens"),
        gix_commitgraph::File::at(&duplicate_path).expect("mutated graph opens"),
    ])
    .expect("graph chain can be constructed");

    let err = graph
        .verify_integrity(|_| Ok::<_, gix_error::Message>(()))
        .expect_err("duplicate commit IDs should be rejected");
    let error = format!("{err:#?}");
    assert!(
        error.contains("appears more than once"),
        "error should mention the duplicate commit ID: {error}"
    );
}

#[test]
fn octopus_merges() {
    let (cg, refs) = graph_and_expected(
        "octopus_merges.sh",
        &[
            "root",
            "parent1",
            "parent2",
            "parent3",
            "parent4",
            "three_parents",
            "four_parents",
        ],
    );
    check_common(&cg, &refs);

    assert_eq!(cg.commit_at(refs["root"].pos()).generation(), 1);
    assert_eq!(cg.commit_at(refs["parent1"].pos()).generation(), 2);
    assert_eq!(cg.commit_at(refs["parent2"].pos()).generation(), 2);
    assert_eq!(cg.commit_at(refs["parent3"].pos()).generation(), 2);
    assert_eq!(cg.commit_at(refs["parent4"].pos()).generation(), 2);
    assert_eq!(cg.commit_at(refs["three_parents"].pos()).generation(), 3);
    assert_eq!(cg.commit_at(refs["four_parents"].pos()).generation(), 3);
}

#[test]
fn single_commit() {
    let (cg, refs) = graph_and_expected("single_commit.sh", &["commit"]);
    check_common(&cg, &refs);

    assert_eq!(cg.commit_at(refs["commit"].pos()).generation(), 1);
}

#[test]
fn two_parents() {
    let (cg, refs) = graph_and_expected("two_parents.sh", &["parent1", "parent2", "child"]);
    check_common(&cg, &refs);

    assert_eq!(cg.commit_at(refs["parent1"].pos()).generation(), 1);
    assert_eq!(cg.commit_at(refs["parent2"].pos()).generation(), 1);
    assert_eq!(cg.commit_at(refs["child"].pos()).generation(), 2);
}

fn changed_path_filter_graph_with_bloom_hash_version(hash_version: u32) -> (gix_testtools::tempfile::TempDir, PathBuf) {
    let repo_dir = gix_testtools::scripted_fixture_read_only("changed_path_filters.sh")
        .expect("changed-path fixture can be created");
    let graph_path = repo_dir.join(".git").join("objects").join("info").join("commit-graph");
    let mut data = std::fs::read(&graph_path).expect("commit-graph can be read");
    let bloom_data_offset = chunk_offset(&data, b"BDAT");
    data[bloom_data_offset..][..4].copy_from_slice(&hash_version.to_be_bytes());

    let tmp = gix_testtools::tempfile::TempDir::new().expect("temporary directory can be created");
    let path = tmp.path().join("commit-graph");
    std::fs::write(&path, data).expect("mutated commit-graph can be written");
    (tmp, path)
}

fn chunk_offset(data: &[u8], wanted_id: &[u8; 4]) -> usize {
    let chunk_count = usize::from(data[6]);
    let table_start = 8;
    for idx in 0..chunk_count {
        let entry_start = table_start + idx * 12;
        let chunk_id = &data[entry_start..][..4];
        if chunk_id == wanted_id {
            let offset = u64::from_be_bytes(data[entry_start + 4..][..8].try_into().unwrap());
            return usize::try_from(offset).expect("chunk offset fits usize");
        }
    }
    panic!(
        "chunk {} not found",
        std::str::from_utf8(wanted_id).expect("ASCII chunk id")
    );
}

fn split_graph_paths(graph_dir: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(graph_dir.join("commit-graph-chain"))
        .expect("commit-graph-chain can be read")
        .lines()
        .map(|hash| graph_dir.join(format!("graph-{hash}.graph")))
        .collect()
}

fn graph_with_duplicated_first_oid(base_path: &Path, graph_path: &Path, output_dir: &Path) -> PathBuf {
    let base = std::fs::read(base_path).expect("base graph can be read");
    let mut data = std::fs::read(graph_path).expect("graph can be read");
    let hash_len = gix_hash::Kind::Sha1.len_in_bytes();
    let base_oid_lookup = chunk_offset(&base, b"OIDL");
    let graph_oid_lookup = chunk_offset(&data, b"OIDL");

    data[graph_oid_lookup..][..hash_len].copy_from_slice(&base[base_oid_lookup..][..hash_len]);

    let checksum_offset = data.len() - hash_len;
    let mut hasher = gix_hash::hasher(gix_hash::Kind::Sha1);
    hasher.update(&data[..checksum_offset]);
    let checksum = hasher.try_finalize().expect("sha1 can finalize");
    data[checksum_offset..].copy_from_slice(checksum.as_bytes());

    let path = output_dir.join(format!("graph-{}.graph", checksum.to_hex()));
    std::fs::write(&path, data).expect("mutated graph can be written");
    path
}
