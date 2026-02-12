//! Interop tests: compare gix-blame output line-by-line against C Git's `git blame --porcelain`.
//!
//! These tests exercise scenarios ported from C Git's test suite:
//! - t8003-blame-corner-cases.sh
//! - t8009-blame-vs-topicbranches.sh
//! - t8013-blame-ignore-revs.sh

use std::{collections::BTreeMap, path::PathBuf};

use gix_blame::BlameRanges;
use gix_hash::ObjectId;
use gix_object::bstr;

// ---------------------------------------------------------------------------
// Baseline parser (reused from blame.rs, adapted here to be standalone)
// ---------------------------------------------------------------------------

struct Baseline<'a> {
    lines: bstr::Lines<'a>,
    filenames: BTreeMap<ObjectId, bstr::BString>,
}

mod baseline {
    use std::{collections::BTreeMap, path::Path};

    use gix_blame::BlameEntry;
    use gix_hash::ObjectId;
    use gix_ref::bstr::ByteSlice;

    use super::Baseline;

    const HEADER_FIELDS: [&str; 12] = [
        "author",
        "author-mail",
        "author-time",
        "author-tz",
        "committer",
        "committer-mail",
        "committer-time",
        "committer-tz",
        "summary",
        "boundary",
        "previous",
        "filename",
    ];

    fn is_known_header_field(field: &&str) -> bool {
        HEADER_FIELDS.contains(field)
    }

    impl Baseline<'_> {
        pub fn collect(
            baseline_path: impl AsRef<Path>,
            source_file_name: gix_object::bstr::BString,
        ) -> std::io::Result<Vec<BlameEntry>> {
            let content = std::fs::read(baseline_path)?;
            let baseline = Baseline {
                lines: content.lines(),
                filenames: BTreeMap::default(),
            };

            Ok(baseline
                .map(|entry| {
                    let source_file_name = if entry.source_file_name.as_ref() == Some(&source_file_name) {
                        None
                    } else {
                        entry.source_file_name
                    };

                    BlameEntry {
                        source_file_name,
                        ..entry
                    }
                })
                .collect())
        }
    }

    impl Iterator for Baseline<'_> {
        type Item = BlameEntry;

        fn next(&mut self) -> Option<Self::Item> {
            let mut ranges = None;
            let mut commit_id = gix_hash::Kind::Sha1.null();
            let mut skip_lines: u32 = 0;
            let mut source_file_name: Option<gix_object::bstr::BString> = None;
            let mut is_boundary = false;

            for line in self.lines.by_ref() {
                if line.starts_with(b"\t") {
                    skip_lines -= 1;

                    if skip_lines == 0 {
                        break;
                    } else {
                        continue;
                    }
                }

                let fields: Vec<&str> = line.to_str().unwrap().split(' ').collect();
                if fields.len() == 4 {
                    commit_id = match ObjectId::from_hex(fields[0].as_bytes()) {
                        Ok(id) => id,
                        Err(_) => continue,
                    };

                    let line_number_in_source_file = fields[1].parse::<u32>().unwrap();
                    let line_number_in_final_file = fields[2].parse::<u32>().unwrap();
                    let number_of_lines_in_group = fields[3].parse::<u32>().unwrap();

                    skip_lines = number_of_lines_in_group;

                    let source_range =
                        (line_number_in_source_file - 1)..(line_number_in_source_file + number_of_lines_in_group - 1);
                    let blame_range =
                        (line_number_in_final_file - 1)..(line_number_in_final_file + number_of_lines_in_group - 1);
                    assert!(ranges.is_none(), "should not overwrite existing ranges");
                    ranges = Some((blame_range, source_range));
                } else if fields[0] == "boundary" {
                    is_boundary = true;
                } else if fields[0] == "filename" {
                    source_file_name = Some(fields[1].into());

                    self.filenames.insert(commit_id, fields[1].into());
                } else if !is_known_header_field(&fields[0]) && ObjectId::from_hex(fields[0].as_bytes()).is_err() {
                    panic!("unexpected line: '{:?}'", line.as_bstr());
                }
            }

            let Some((range_in_blamed_file, range_in_source_file)) = ranges else {
                return None;
            };
            let mut entry = BlameEntry::new(
                range_in_blamed_file,
                range_in_source_file,
                commit_id,
                source_file_name.or_else(|| self.filenames.get(&commit_id).cloned()),
            );
            entry.boundary = is_boundary;
            Some(entry)
        }
    }
}

// ---------------------------------------------------------------------------
// Fixture helper: set up odb, resource_cache, and resolve refs
// ---------------------------------------------------------------------------

struct Fixture {
    odb: gix_odb::Handle,
    resource_cache: gix_diff::blob::Platform,
    suspect: ObjectId,
    store: gix_ref::file::Store,
    worktree_path: PathBuf,
}

impl Fixture {
    fn new(worktree_path: PathBuf) -> gix_testtools::Result<Fixture> {
        use gix_ref::store::WriteReflog;

        let store = gix_ref::file::Store::at(
            worktree_path.join(".git"),
            gix_ref::store::init::Options {
                write_reflog: WriteReflog::Disable,
                ..Default::default()
            },
        );
        let odb = gix_odb::at(worktree_path.join(".git/objects"))?;

        let mut reference = gix_ref::file::Store::find(&store, "HEAD")?;

        use gix_ref::file::ReferenceExt;
        let head_id = reference.peel_to_id(&store, &odb)?;

        let git_dir = worktree_path.join(".git");
        let index = gix_index::File::at(git_dir.join("index"), gix_hash::Kind::Sha1, false, Default::default())?;
        let stack = gix_worktree::Stack::from_state_and_ignore_case(
            worktree_path.clone(),
            false,
            gix_worktree::stack::State::AttributesAndIgnoreStack {
                attributes: Default::default(),
                ignore: Default::default(),
            },
            &index,
            index.path_backing(),
        );
        let capabilities = gix_fs::Capabilities::probe(&git_dir);
        let resource_cache = gix_diff::blob::Platform::new(
            Default::default(),
            gix_diff::blob::Pipeline::new(
                gix_diff::blob::pipeline::WorktreeRoots {
                    old_root: None,
                    new_root: None,
                },
                gix_filter::Pipeline::new(Default::default(), Default::default()),
                vec![],
                gix_diff::blob::pipeline::Options {
                    large_file_threshold_bytes: 0,
                    fs: capabilities,
                },
            ),
            gix_diff::blob::pipeline::Mode::ToGit,
            stack,
        );
        Ok(Fixture {
            odb,
            resource_cache,
            suspect: head_id,
            store,
            worktree_path,
        })
    }

    /// Resolve a ref name (e.g. "refs/tags/A") to an ObjectId.
    fn resolve_ref(&self, refname: &str) -> gix_testtools::Result<ObjectId> {
        let mut reference = gix_ref::file::Store::find(&self.store, refname)?;
        use gix_ref::file::ReferenceExt;
        let id = reference.peel_to_id(&self.store, &self.odb)?;
        Ok(id)
    }

    /// Run blame on a file with the given options.
    fn blame_file(
        &mut self,
        source_file_name: &bstr::BStr,
        options: gix_blame::Options,
    ) -> Result<gix_blame::Outcome, gix_blame::Error> {
        gix_blame::file(
            &self.odb,
            self.suspect,
            None,
            &mut self.resource_cache,
            source_file_name,
            options,
            &std::sync::atomic::AtomicBool::new(false),
        )
    }

    /// Run blame on a file with the given options starting from a specific commit.
    fn blame_file_at(
        &mut self,
        suspect: ObjectId,
        source_file_name: &bstr::BStr,
        options: gix_blame::Options,
    ) -> Result<gix_blame::Outcome, gix_blame::Error> {
        gix_blame::file(
            &self.odb,
            suspect,
            None,
            &mut self.resource_cache,
            source_file_name,
            options,
            &std::sync::atomic::AtomicBool::new(false),
        )
    }

    fn git_dir(&self) -> PathBuf {
        self.worktree_path.join(".git")
    }
}

fn interop_repo_path() -> gix_testtools::Result<PathBuf> {
    gix_testtools::scripted_fixture_read_only("make_blame_interop_repo.sh")
}

fn default_options() -> gix_blame::Options {
    gix_blame::Options {
        diff_algorithm: gix_diff::blob::Algorithm::Histogram,
        ranges: BlameRanges::default(),
        since: None,
        rewrites: Some(gix_diff::Rewrites::default()),
        debug_track_path: false,
        ignore_revs: Vec::new(),
        worktree_blob: None,
        oldest_commit: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Scenario 1: basic blame — multiple authors modify the same file, line-by-line
/// comparison with C Git's porcelain output.
#[test]
fn multi_author_each_line_from_different_commit() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let source_file_name = "multi-author.txt";
    let outcome = fixture.blame_file(source_file_name.into(), default_options())?;

    assert_eq!(
        outcome.entries.len(),
        5,
        "each of the 5 lines should be a separate entry"
    );

    let baseline = Baseline::collect(fixture.git_dir().join("multi-author.baseline"), source_file_name.into())?;
    assert_eq!(baseline.len(), 5);

    pretty_assertions::assert_eq!(outcome.entries, baseline);
    Ok(())
}

/// Scenario 2: merge from topic branch — changes on both main and topic should be
/// correctly attributed after merge.
#[test]
fn merge_from_topic_branch() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let source_file_name = "topicbranch.txt";
    let outcome = fixture.blame_file(source_file_name.into(), default_options())?;

    let baseline = Baseline::collect(fixture.git_dir().join("topicbranch.baseline"), source_file_name.into())?;

    pretty_assertions::assert_eq!(outcome.entries, baseline);
    Ok(())
}

/// Scenario 3: coalesce test (ported from t8003) — adding then removing a SPLIT line
/// should coalesce back to the original author.
#[test]
fn coalesce_after_split_removal() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let source_file_name = "coalesce-interop.txt";
    let outcome = fixture.blame_file(source_file_name.into(), default_options())?;

    let baseline = Baseline::collect(
        fixture.git_dir().join("coalesce-interop.baseline"),
        source_file_name.into(),
    )?;

    pretty_assertions::assert_eq!(outcome.entries, baseline);
    Ok(())
}

/// Scenario 4: path that was a directory, then became a regular file (t8003).
/// gix-blame currently does not support this case — it panics with "can't be deleted"
/// because the tree diff sees the transition from directory to file as a deletion.
/// This test documents the limitation.
#[test]
fn path_was_directory_then_file_is_unsupported() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let source_file_name = "path-was-dir";
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fixture.blame_file(source_file_name.into(), default_options())
    }));

    assert!(
        result.is_err(),
        "gix-blame should panic (or error) on a path that transitioned from directory to file"
    );
    Ok(())
}

/// Scenario 5: blame at a specific (non-HEAD) revision using a tag.
#[test]
fn blame_at_specific_revision() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let tag_b = fixture.resolve_ref("refs/tags/B")?;
    let source_file_name = "ignore-revs-file.txt";
    let outcome = fixture.blame_file_at(tag_b, source_file_name.into(), default_options())?;

    let baseline = Baseline::collect(
        fixture.git_dir().join("ignore-revs-file-at-B.baseline"),
        source_file_name.into(),
    )?;

    pretty_assertions::assert_eq!(outcome.entries, baseline);
    Ok(())
}

/// Scenario 6: ignore-revs — when X modified lines originally from A and B,
/// ignoring X should re-attribute to A and B (C Git behavior) or pin them to
/// X (gix behavior without fuzzy heuristics).
///
/// C Git uses `guess_line_blames()` heuristics to map modified lines back to
/// ancestors. gix does not implement these heuristics yet, so lines modified by
/// the ignored commit stay "pinned" to it. This test verifies the gix behavior:
/// all lines are still covered, and the total count matches.
#[test]
fn ignore_revs_changing_lines() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let tag_x = fixture.resolve_ref("refs/tags/X")?;
    let source_file_name = "ignore-revs-file.txt";

    // Normal blame at X: both lines should be attributed to X.
    let outcome_normal = fixture.blame_file_at(tag_x, source_file_name.into(), default_options())?;
    let total_normal: u32 = outcome_normal.entries.iter().map(|e| e.len.get()).sum();
    assert_eq!(total_normal, 2, "normal blame should cover 2 lines");

    // Blame at X with X ignored: gix pins modified lines to X (no fuzzy heuristic).
    let outcome_ignored = fixture.blame_file_at(
        tag_x,
        source_file_name.into(),
        gix_blame::Options {
            ignore_revs: vec![tag_x],
            ..default_options()
        },
    )?;

    let total_ignored: u32 = outcome_ignored.entries.iter().map(|e| e.len.get()).sum();
    assert_eq!(total_ignored, 2, "ignored blame should still cover 2 lines");

    // C Git would attribute line 1 to A and line 2 to B.
    // gix pins both to X (the ignored commit itself, since it's a modification with no
    // fuzzy match to parent). Verify at least that the entries are valid.
    for entry in &outcome_ignored.entries {
        assert!(entry.len.get() > 0, "all blame entries should have positive length");
    }

    Ok(())
}

/// Scenario 7: ignore-revs with unblamable added lines — lines 3,4 added by Y
/// that have no match in parent should be pinned to Y even when Y is ignored.
///
/// C Git's behavior: lines 1-2 are attributed to X (the parent of Y's changes),
/// lines 3-4 are pinned to Y. gix's behavior: all changed lines from Y are
/// pinned to Y (no fuzzy heuristic for re-attribution). This test verifies the
/// shared invariant: all lines are covered, and the added lines (3,4) are pinned to Y.
#[test]
fn ignore_revs_unblamable_lines_pinned() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let tag_y = fixture.resolve_ref("refs/tags/Y")?;
    let source_file_name = "ignore-revs-file.txt";

    let outcome = fixture.blame_file(
        source_file_name.into(),
        gix_blame::Options {
            ignore_revs: vec![tag_y],
            ..default_options()
        },
    )?;

    let total_lines: u32 = outcome.entries.iter().map(|e| e.len.get()).sum();
    assert_eq!(total_lines, 4, "all 4 lines should be covered by blame entries");

    // Lines 3 and 4 (0-indexed: 2, 3) were added by Y and have no parent equivalent.
    // Both C Git and gix should pin these to Y.
    let entry_covering_line3 = outcome
        .entries
        .iter()
        .find(|e| {
            let range = e.range_in_blamed_file();
            range.contains(&2)
        })
        .expect("there should be an entry covering line 3 (0-indexed: 2)");
    assert_eq!(
        entry_covering_line3.commit_id, tag_y,
        "line 3 (added by Y, no parent match) should be pinned to Y"
    );

    let entry_covering_line4 = outcome
        .entries
        .iter()
        .find(|e| {
            let range = e.range_in_blamed_file();
            range.contains(&3)
        })
        .expect("there should be an entry covering line 4 (0-indexed: 3)");
    assert_eq!(
        entry_covering_line4.commit_id, tag_y,
        "line 4 (added by Y, no parent match) should be pinned to Y"
    );

    Ok(())
}

/// Scenario 8: boundary check — A--B--C where B adds lines in the middle,
/// C changes one line. Ignoring B tests the boundary checks.
#[test]
fn boundary_check_normal() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let source_file_name = "boundary-check.txt";
    let outcome = fixture.blame_file(source_file_name.into(), default_options())?;

    let baseline = Baseline::collect(
        fixture.git_dir().join("boundary-check.baseline"),
        source_file_name.into(),
    )?;

    pretty_assertions::assert_eq!(outcome.entries, baseline);
    Ok(())
}

/// Scenario 8b: boundary check with ignore-rev on B.
#[test]
fn boundary_check_ignore_b() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let tag_b = fixture.resolve_ref("refs/tags/boundary-B")?;
    let source_file_name = "boundary-check.txt";
    let outcome = fixture.blame_file(
        source_file_name.into(),
        gix_blame::Options {
            ignore_revs: vec![tag_b],
            ..default_options()
        },
    )?;

    let baseline = Baseline::collect(
        fixture.git_dir().join("boundary-check-ignore-B.baseline"),
        source_file_name.into(),
    )?;

    pretty_assertions::assert_eq!(outcome.entries, baseline);
    Ok(())
}

/// Scenario 9: ignore merge — A--B--M and A--C--M. Ignoring M should still
/// attribute line 1 to B and line 9 to C.
#[test]
fn ignore_merge() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let source_file_name = "ignore-merge.txt";

    // Normal blame should match baseline
    let outcome_normal = fixture.blame_file(source_file_name.into(), default_options())?;
    let baseline_normal = Baseline::collect(fixture.git_dir().join("ignore-merge.baseline"), source_file_name.into())?;
    pretty_assertions::assert_eq!(outcome_normal.entries, baseline_normal);

    // Blame ignoring merge-M
    let tag_m = fixture.resolve_ref("refs/tags/merge-M")?;
    let outcome_ignored = fixture.blame_file(
        source_file_name.into(),
        gix_blame::Options {
            ignore_revs: vec![tag_m],
            ..default_options()
        },
    )?;
    let baseline_ignored = Baseline::collect(
        fixture.git_dir().join("ignore-merge-ignore-M.baseline"),
        source_file_name.into(),
    )?;
    pretty_assertions::assert_eq!(outcome_ignored.entries, baseline_ignored);

    Ok(())
}

/// Scenario 10: empty file — blame should produce no entries.
#[test]
fn empty_file() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let source_file_name = "empty-file.txt";
    let outcome = fixture.blame_file(source_file_name.into(), default_options())?;

    assert!(
        outcome.entries.is_empty(),
        "blaming an empty file should produce no entries, got {} entries",
        outcome.entries.len()
    );
    Ok(())
}

/// Scenario 11: blame with line range (-L 3,5) on multi-author file.
#[test]
fn blame_with_line_range() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let source_file_name = "multi-author.txt";
    let outcome = fixture.blame_file(
        source_file_name.into(),
        gix_blame::Options {
            ranges: BlameRanges::from_one_based_inclusive_range(3..=5).expect("valid range"),
            ..default_options()
        },
    )?;

    let baseline = Baseline::collect(
        fixture.git_dir().join("multi-author-L3-5.baseline"),
        source_file_name.into(),
    )?;

    pretty_assertions::assert_eq!(outcome.entries, baseline);
    Ok(())
}

/// Verify that `entries_with_lines()` produces lines matching the actual file content.
#[test]
fn entries_with_lines_consistency() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    let source_file_name = "multi-author.txt";
    let outcome = fixture.blame_file(source_file_name.into(), default_options())?;

    // Collect all lines from entries_with_lines
    let mut all_lines: Vec<bstr::BString> = Vec::new();
    for (_entry, lines) in outcome.entries_with_lines() {
        all_lines.extend(lines);
    }

    // The concatenation of all lines should reconstruct the file content
    let reconstructed: Vec<u8> = all_lines
        .iter()
        .flat_map(|l| AsRef::<[u8]>::as_ref(l))
        .copied()
        .collect();
    assert_eq!(
        reconstructed, outcome.blob,
        "entries_with_lines should cover the entire blamed file content"
    );

    Ok(())
}

/// Verify total line coverage: sum of all entry lengths should equal the file's line count.
#[test]
fn total_line_coverage() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    for file_name in &[
        "multi-author.txt",
        "topicbranch.txt",
        "coalesce-interop.txt",
        "boundary-check.txt",
        "ignore-merge.txt",
        "ignore-revs-file.txt",
    ] {
        let outcome = fixture.blame_file((*file_name).into(), default_options())?;

        let total_lines: u32 = outcome.entries.iter().map(|e| e.len.get()).sum();

        // Count lines in blob
        let blob_line_count = if outcome.blob.is_empty() {
            0u32
        } else {
            outcome.blob.split(|&b| b == b'\n').count() as u32 - if outcome.blob.last() == Some(&b'\n') { 1 } else { 0 }
        };

        assert_eq!(
            total_lines, blob_line_count,
            "total blamed lines should match file line count for {file_name}"
        );
    }

    Ok(())
}

/// Entries should be sorted by start_in_blamed_file with no gaps or overlaps.
#[test]
fn entries_are_contiguous_and_sorted() -> gix_testtools::Result {
    let worktree_path = interop_repo_path()?;
    let mut fixture = Fixture::new(worktree_path)?;

    for file_name in &[
        "multi-author.txt",
        "topicbranch.txt",
        "coalesce-interop.txt",
        "boundary-check.txt",
        "ignore-merge.txt",
    ] {
        let outcome = fixture.blame_file((*file_name).into(), default_options())?;

        let mut expected_start = 0u32;
        for entry in &outcome.entries {
            assert_eq!(
                entry.start_in_blamed_file, expected_start,
                "entries should be contiguous for {file_name}: expected start {expected_start}, got {}",
                entry.start_in_blamed_file
            );
            expected_start += entry.len.get();
        }
    }

    Ok(())
}
