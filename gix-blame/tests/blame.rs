use std::{collections::BTreeMap, path::PathBuf};

use gix_blame::BlameRanges;
use gix_hash::ObjectId;
use gix_object::bstr;

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

    // These fields are used by `git` in its porcelain output.
    const HEADER_FIELDS: [&str; 12] = [
        // https://github.com/git/git/blob/6258f68c3c1092c901337895c864073dcdea9213/builtin/blame.c#L256-L280
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
        // https://github.com/git/git/blob/6258f68c3c1092c901337895c864073dcdea9213/builtin/blame.c#L239-L248
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
                    // Each group consists of a header and one or more lines. We break from the
                    // loop, thus returning a `BlameEntry` from `next` once we have seen the number
                    // of lines starting with "\t" as indicated in the group's header.
                    skip_lines -= 1;

                    if skip_lines == 0 {
                        break;
                    } else {
                        continue;
                    }
                }

                let fields: Vec<&str> = line.to_str().unwrap().split(' ').collect();
                if fields.len() == 4 {
                    // We're possibly dealing with a group header.
                    // If we can't parse the first field as an `ObjectId`, we know this is not a
                    // group header, so we continue. This can yield false positives, but for
                    // testing purposes, we don't bother.
                    commit_id = match ObjectId::from_hex(fields[0].as_bytes()) {
                        Ok(id) => id,
                        Err(_) => continue,
                    };

                    let line_number_in_source_file = fields[1].parse::<u32>().unwrap();
                    let line_number_in_final_file = fields[2].parse::<u32>().unwrap();
                    // The last field indicates the number of lines this group contains info for
                    // (this is not equal to the number of lines in git blame's porcelain output).
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
                    // We need to store `source_file_name` as it is not repeated for subsequent
                    // hunks that have the same `commit_id`.
                    source_file_name = Some(fields[1].into());

                    self.filenames.insert(commit_id, fields[1].into());
                } else if !is_known_header_field(&fields[0]) && ObjectId::from_hex(fields[0].as_bytes()).is_err() {
                    panic!("unexpected line: '{:?}'", line.as_bstr());
                }
            }

            let Some((range_in_blamed_file, range_in_source_file)) = ranges else {
                // No new lines were parsed, so we assume the iterator is finished.
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

struct Fixture {
    odb: gix_odb::Handle,
    resource_cache: gix_diff::blob::Platform,
    suspect: ObjectId,
}

impl Fixture {
    fn new() -> gix_testtools::Result<Fixture> {
        Self::for_worktree_path(fixture_path()?)
    }

    fn for_worktree_path(worktree_path: PathBuf) -> gix_testtools::Result<Fixture> {
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

        // Needed for `peel_to_id`.
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
        })
    }

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
}

macro_rules! mktest {
    ($name:ident, $case:expr, $number_of_lines:literal) => {
        #[test]
        fn $name() -> gix_testtools::Result {
            let Fixture {
                odb,
                mut resource_cache,
                suspect,
            } = Fixture::new()?;

            let source_file_name: gix_object::bstr::BString = format!("{}.txt", $case).into();

            let lines_blamed = gix_blame::file(
                &odb,
                suspect,
                None,
                &mut resource_cache,
                source_file_name.as_ref(),
                gix_blame::Options {
                    diff_algorithm: gix_diff::blob::Algorithm::Histogram,
                    ranges: BlameRanges::default(),
                    since: None,
                    rewrites: Some(gix_diff::Rewrites::default()),
                    debug_track_path: false,
                    ignore_revs: Vec::new(),
                    worktree_blob: None,
                    oldest_commit: None,
                },
                &std::sync::atomic::AtomicBool::new(false),
            )?
            .entries;

            assert_eq!(lines_blamed.len(), $number_of_lines);

            let git_dir = fixture_path()?.join(".git");
            let baseline = Baseline::collect(git_dir.join(format!("{}.baseline", $case)), source_file_name)?;

            assert_eq!(baseline.len(), $number_of_lines);
            pretty_assertions::assert_eq!(lines_blamed, baseline);
            Ok(())
        }
    };
}

mktest!(simple_case, "simple", 4);
mktest!(multiline_hunks, "multiline-hunks", 3);
mktest!(deleted_lines, "deleted-lines", 1);
mktest!(deleted_lines_multiple_hunks, "deleted-lines-multiple-hunks", 2);
mktest!(changed_lines, "changed-lines", 1);
mktest!(
    changed_line_between_unchanged_lines,
    "changed-line-between-unchanged-lines",
    3
);
mktest!(added_lines, "added-lines", 2);
mktest!(added_lines_around, "added-lines-around", 3);
mktest!(switched_lines, "switched-lines", 4);
mktest!(added_line_before_changed_line, "added-line-before-changed-line", 3);
mktest!(same_line_changed_twice, "same-line-changed-twice", 2);
mktest!(coalesce_adjacent_hunks, "coalesce-adjacent-hunks", 1);

mktest!(sub_directory, "sub-directory/sub-directory", 3);

mktest!(after_rename, "after-rename", 1);
mktest!(after_second_rename, "after-second-rename", 1);
mktest!(after_rewrite, "after-rewrite", 3);
mktest!(
    after_move_to_sub_directory,
    "sub-directory/after-move-to-sub-directory",
    1
);

mktest!(resolved_conflict, "resolved-conflict", 2);
mktest!(file_in_one_chain_of_ancestors, "file-in-one-chain-of-ancestors", 1);
mktest!(
    different_file_in_another_chain_of_ancestors,
    "different-file-in-another-chain-of-ancestors",
    1
);
mktest!(file_only_changed_in_branch, "file-only-changed-in-branch", 2);
mktest!(file_changed_in_two_branches, "file-changed-in-two-branches", 3);
mktest!(
    file_topo_order_different_than_date_order,
    "file-topo-order-different-than-date-order",
    3
);

/// As of 2025-12-07, both algorithms are expected to pass. They use `imara-diff` 0.2 under the
/// hood. One of them failed with `imara-diff` 0.1.
///
/// Context: https://github.com/Byron/gitoxide/pull/1453#issuecomment-2371013904
#[test]
fn diff_algorithm_parity() {
    for (case, diff_algorithm) in [
        ("empty-lines-myers", gix_diff::blob::Algorithm::Myers),
        ("empty-lines-histogram", gix_diff::blob::Algorithm::Histogram),
    ] {
        let Fixture {
            odb,
            mut resource_cache,
            suspect,
        } = Fixture::new().unwrap();

        let source_file_name: gix_object::bstr::BString = format!("{case}.txt").into();

        let lines_blamed = gix_blame::file(
            &odb,
            suspect,
            None,
            &mut resource_cache,
            source_file_name.as_ref(),
            gix_blame::Options {
                diff_algorithm,
                ranges: BlameRanges::default(),
                since: None,
                rewrites: Some(gix_diff::Rewrites::default()),
                debug_track_path: false,
                ignore_revs: Vec::new(),
                worktree_blob: None,
                oldest_commit: None,
            },
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap()
        .entries;

        assert_eq!(lines_blamed.len(), 5);

        let git_dir = fixture_path().unwrap().join(".git");
        let baseline = Baseline::collect(git_dir.join(format!("{case}.baseline")), source_file_name).unwrap();

        pretty_assertions::assert_eq!(lines_blamed, baseline, "{case}");
    }
}

#[test]
fn file_that_was_added_in_two_branches() -> gix_testtools::Result {
    let worktree_path = gix_testtools::scripted_fixture_read_only("make_blame_two_roots_repo.sh")?;

    let Fixture {
        odb,
        mut resource_cache,
        suspect,
    } = Fixture::for_worktree_path(worktree_path.to_path_buf())?;

    let source_file_name = "file-with-two-roots.txt";
    let lines_blamed = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.into(),
        gix_blame::Options::default(),
        &std::sync::atomic::AtomicBool::new(false),
    )?
    .entries;

    assert_eq!(lines_blamed.len(), 4);

    let git_dir = worktree_path.join(".git");
    let baseline = Baseline::collect(git_dir.join("file-with-two-roots.baseline"), source_file_name.into())?;

    pretty_assertions::assert_eq!(lines_blamed, baseline);

    Ok(())
}

#[test]
fn since() -> gix_testtools::Result {
    let Fixture {
        odb,
        mut resource_cache,
        suspect,
    } = Fixture::new()?;

    let source_file_name: gix_object::bstr::BString = "simple.txt".into();

    let lines_blamed = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: Some(
                gix_date::parse("2025-01-31", None).expect("TODO: should be able to to retrieve inner from Exn"),
            ),
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: None,
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?
    .entries;

    assert_eq!(lines_blamed.len(), 1);

    let git_dir = fixture_path()?.join(".git");
    let baseline = Baseline::collect(git_dir.join("simple-since.baseline"), source_file_name)?;

    pretty_assertions::assert_eq!(lines_blamed, baseline);

    Ok(())
}

mod blame_ranges {
    use crate::{fixture_path, Baseline, Fixture};
    use gix_blame::BlameRanges;

    #[test]
    fn line_range() -> gix_testtools::Result {
        let Fixture {
            odb,
            mut resource_cache,
            suspect,
        } = Fixture::new()?;

        let source_file_name: gix_object::bstr::BString = "simple.txt".into();

        let lines_blamed = gix_blame::file(
            &odb,
            suspect,
            None,
            &mut resource_cache,
            source_file_name.as_ref(),
            gix_blame::Options {
                diff_algorithm: gix_diff::blob::Algorithm::Histogram,
                ranges: BlameRanges::from_one_based_inclusive_range(1..=2).unwrap(),
                since: None,
                rewrites: Some(gix_diff::Rewrites::default()),
                debug_track_path: false,
                ignore_revs: Vec::new(),
                worktree_blob: None,
                oldest_commit: None,
            },
            &std::sync::atomic::AtomicBool::new(false),
        )?
        .entries;

        assert_eq!(lines_blamed.len(), 2);

        let git_dir = fixture_path()?.join(".git");
        let baseline = Baseline::collect(git_dir.join("simple-lines-1-2.baseline"), source_file_name)?;

        pretty_assertions::assert_eq!(lines_blamed, baseline);

        Ok(())
    }

    #[test]
    fn multiple_ranges_using_add_range() -> gix_testtools::Result {
        let Fixture {
            odb,
            mut resource_cache,
            suspect,
        } = Fixture::new()?;

        let ranges = BlameRanges::from_one_based_inclusive_ranges(vec![
            1..=2, // Lines 1-2
            1..=1, // Duplicate range, should be ignored
            4..=4, // Line 4
        ])
        .unwrap();

        let source_file_name: gix_object::bstr::BString = "simple.txt".into();

        let lines_blamed = gix_blame::file(
            &odb,
            suspect,
            None,
            &mut resource_cache,
            source_file_name.as_ref(),
            gix_blame::Options {
                diff_algorithm: gix_diff::blob::Algorithm::Histogram,
                ranges,
                since: None,
                rewrites: None,
                debug_track_path: false,
                ignore_revs: Vec::new(),
                worktree_blob: None,
                oldest_commit: None,
            },
            &std::sync::atomic::AtomicBool::new(false),
        )?
        .entries;

        assert_eq!(lines_blamed.len(), 3); // Should have 3 lines total (2 from first range + 1 from second range)

        let git_dir = fixture_path()?.join(".git");
        let baseline = Baseline::collect(
            git_dir.join("simple-lines-multiple-1-2-and-4.baseline"),
            source_file_name,
        )?;

        pretty_assertions::assert_eq!(lines_blamed, baseline);

        Ok(())
    }

    #[test]
    fn multiple_ranges_using_from_ranges() -> gix_testtools::Result {
        let Fixture {
            odb,
            mut resource_cache,
            suspect,
        } = Fixture::new()?;

        let ranges = BlameRanges::from_one_based_inclusive_ranges(vec![1..=2, 1..=1, 4..=4]).unwrap();

        let source_file_name: gix_object::bstr::BString = "simple.txt".into();

        let lines_blamed = gix_blame::file(
            &odb,
            suspect,
            None,
            &mut resource_cache,
            source_file_name.as_ref(),
            gix_blame::Options {
                diff_algorithm: gix_diff::blob::Algorithm::Histogram,
                ranges,
                since: None,
                rewrites: None,
                debug_track_path: false,
                ignore_revs: Vec::new(),
                worktree_blob: None,
                oldest_commit: None,
            },
            &std::sync::atomic::AtomicBool::new(false),
        )?
        .entries;

        assert_eq!(lines_blamed.len(), 3); // Should have 3 lines total (2 from first range + 1 from second range)

        let git_dir = fixture_path()?.join(".git");
        let baseline = Baseline::collect(
            git_dir.join("simple-lines-multiple-1-2-and-4.baseline"),
            source_file_name,
        )?;

        pretty_assertions::assert_eq!(lines_blamed, baseline);

        Ok(())
    }
}

mod rename_tracking {
    use gix_blame::BlameRanges;

    use crate::{Baseline, Fixture};

    #[test]
    fn source_file_name_is_tracked_per_hunk() -> gix_testtools::Result {
        let worktree_path = gix_testtools::scripted_fixture_read_only("make_blame_rename_tracking_repo.sh")?;

        let Fixture {
            odb,
            mut resource_cache,
            suspect,
        } = Fixture::for_worktree_path(worktree_path.to_path_buf())?;

        let source_file_name = "after-rename.txt";
        let lines_blamed = gix_blame::file(
            &odb,
            suspect,
            None,
            &mut resource_cache,
            source_file_name.into(),
            gix_blame::Options {
                diff_algorithm: gix_diff::blob::Algorithm::Histogram,
                ranges: BlameRanges::default(),
                since: None,
                rewrites: Some(gix_diff::Rewrites::default()),
                debug_track_path: false,
                ignore_revs: Vec::new(),
                worktree_blob: None,
                oldest_commit: None,
            },
            &std::sync::atomic::AtomicBool::new(false),
        )?
        .entries;

        assert_eq!(lines_blamed.len(), 3);

        let git_dir = worktree_path.join(".git");
        let baseline = Baseline::collect(git_dir.join("after-rename.baseline"), source_file_name.into())?;

        pretty_assertions::assert_eq!(lines_blamed, baseline);

        Ok(())
    }

    #[test]
    fn rename_and_change_in_merge_commit() -> gix_testtools::Result {
        let worktree_path = gix_testtools::scripted_fixture_read_only("make_blame_rename_tracking_repo.sh")?;

        let mut fixture = Fixture::for_worktree_path(worktree_path.to_path_buf())?;
        let source_file_name = "change-and-renamed.txt";

        let lines_blamed = fixture
            .blame_file(
                source_file_name.into(),
                gix_blame::Options {
                    diff_algorithm: gix_diff::blob::Algorithm::Histogram,
                    ranges: BlameRanges::default(),
                    since: None,
                    rewrites: Some(gix_diff::Rewrites::default()),
                    debug_track_path: false,
                    ignore_revs: Vec::new(),
                    worktree_blob: None,
                    oldest_commit: None,
                },
            )?
            .entries;

        assert_eq!(lines_blamed.len(), 4);

        let git_dir = worktree_path.join(".git");
        let baseline = Baseline::collect(git_dir.join("change-and-renamed.baseline"), source_file_name.into())?;

        pretty_assertions::assert_eq!(lines_blamed, baseline);

        Ok(())
    }
}

#[test]
fn ignore_revs() -> gix_testtools::Result {
    let Fixture {
        odb,
        mut resource_cache,
        suspect,
    } = Fixture::new()?;

    let source_file_name: gix_object::bstr::BString = "simple.txt".into();

    // First, do a normal blame to find out which commits are responsible for each line.
    let normal_result = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: None,
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?;

    // simple.txt has 4 lines, each added in a different commit (c1, c2, c3, c4).
    assert_eq!(normal_result.entries.len(), 4);
    let commit_for_line3 = normal_result.entries[2].commit_id;

    // Now blame again, ignoring the commit that introduced line 3.
    // Since line 3 was *added* (not just modified) by c3, it can't be mapped to
    // the parent (which doesn't have it). The current implementation "pins" such lines
    // to the commit that introduced them, matching C Git's fallback behavior when no
    // fuzzy line match is found.
    let ignored_result = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: vec![commit_for_line3],
            worktree_blob: None,
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?;

    // Even with --ignore-rev, all 4 lines should still be accounted for.
    let total_lines: u32 = ignored_result.entries.iter().map(|e| e.len.get()).sum();
    assert_eq!(total_lines, 4, "all 4 lines should be covered by blame entries");

    // Lines that were *not* changed by the ignored commit (lines 1, 2, 4) should
    // have the same attribution as the normal blame.
    let line1_normal = normal_result
        .entries
        .iter()
        .find(|e| e.start_in_blamed_file == 0)
        .unwrap();
    let line1_ignored = ignored_result
        .entries
        .iter()
        .find(|e| e.start_in_blamed_file == 0)
        .unwrap();
    assert_eq!(
        line1_normal.commit_id, line1_ignored.commit_id,
        "line 1 attribution should be unchanged when ignoring a different commit"
    );

    // Line 3 is pinned to the ignored commit since it was an addition with no
    // corresponding line in the parent.
    let line3_entry = ignored_result
        .entries
        .iter()
        .find(|e| {
            let range = e.range_in_blamed_file();
            range.contains(&2)
        })
        .expect("there should be an entry covering line 3 (0-indexed: 2)");
    assert_eq!(
        line3_entry.commit_id, commit_for_line3,
        "added lines should be pinned to the ignored commit (C Git fallback behavior)"
    );

    Ok(())
}

/// Test that `file_with_progress` increments the progress counter for each commit traversed.
#[test]
fn progress_reporting() -> gix_testtools::Result {
    let Fixture {
        odb,
        mut resource_cache,
        suspect,
    } = Fixture::new()?;

    let source_file_name: gix_object::bstr::BString = "simple.txt".into();
    let counter = std::sync::atomic::AtomicUsize::new(0);

    let outcome = gix_blame::file_with_progress(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: None,
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
        Some(&counter),
    )?;

    let progress_count = counter.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        progress_count > 0,
        "progress counter should have been incremented at least once"
    );
    assert_eq!(
        progress_count, outcome.statistics.commits_traversed,
        "progress counter should match the number of commits traversed"
    );

    Ok(())
}

/// Test that providing a worktree_blob that differs from HEAD attributes changed lines
/// to the null ObjectId (uncommitted changes), while unchanged lines are still attributed
/// to the original commits.
#[test]
fn worktree_changes_attributes_modified_lines_to_null_commit() -> gix_testtools::Result {
    let Fixture {
        odb,
        mut resource_cache,
        suspect,
    } = Fixture::new()?;

    let source_file_name: gix_object::bstr::BString = "simple.txt".into();

    // First, get the normal blame to understand the baseline.
    let normal_outcome = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: None,
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?;

    // The normal blame should have 4 entries (lines), all with real commit ids.
    assert_eq!(normal_outcome.entries.len(), 4);
    let null_oid = ObjectId::null(gix_hash::Kind::Sha1);
    for entry in &normal_outcome.entries {
        assert_ne!(
            entry.commit_id, null_oid,
            "normal blame should not have null commit ids"
        );
    }

    // Now create a modified worktree blob. The original blob is in normal_outcome.blob.
    // We modify the first line by prepending "MODIFIED: " to it.
    let original_blob = &normal_outcome.blob;
    let mut worktree_blob = Vec::new();
    let mut first_line = true;
    for line in original_blob.split_inclusive(|&b| b == b'\n') {
        if first_line {
            worktree_blob.extend_from_slice(b"MODIFIED: ");
            worktree_blob.extend_from_slice(line);
            first_line = false;
        } else {
            worktree_blob.extend_from_slice(line);
        }
    }

    let worktree_outcome = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: Some(worktree_blob),
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?;

    // The first line should now be attributed to the null commit (uncommitted changes).
    let first_entry = worktree_outcome
        .entries
        .iter()
        .find(|e| e.start_in_blamed_file == 0)
        .expect("there should be an entry starting at line 0");
    assert_eq!(
        first_entry.commit_id, null_oid,
        "modified first line should be attributed to the null commit (uncommitted changes)"
    );

    // Lines that were not modified should still have real commit ids.
    for entry in &worktree_outcome.entries {
        if entry.start_in_blamed_file > 0 {
            assert_ne!(
                entry.commit_id, null_oid,
                "unchanged lines should still be attributed to real commits"
            );
        }
    }

    Ok(())
}

/// Test that providing a worktree_blob identical to HEAD produces the same results as
/// not providing one.
#[test]
fn worktree_blob_identical_to_head_is_no_op() -> gix_testtools::Result {
    let Fixture {
        odb,
        mut resource_cache,
        suspect,
    } = Fixture::new()?;

    let source_file_name: gix_object::bstr::BString = "simple.txt".into();

    let normal_outcome = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: None,
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?;

    // Pass the HEAD blob as the worktree blob -- should produce identical results.
    let worktree_outcome = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: Some(normal_outcome.blob.clone()),
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?;

    assert_eq!(
        normal_outcome.entries, worktree_outcome.entries,
        "passing the HEAD blob as worktree_blob should produce identical blame entries"
    );

    Ok(())
}

/// Test that setting `should_interrupt` to `true` before calling `file()` causes it
/// to return `Error::Interrupted` immediately.
#[test]
fn interrupted_immediately_when_should_interrupt_is_set() -> gix_testtools::Result {
    let Fixture {
        odb,
        mut resource_cache,
        suspect,
    } = Fixture::new()?;

    let source_file_name: gix_object::bstr::BString = "simple.txt".into();

    let result = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options::default(),
        &std::sync::atomic::AtomicBool::new(true),
    );

    assert!(
        result.is_err(),
        "blame should fail when should_interrupt is already set"
    );
    assert!(
        matches!(result.unwrap_err(), gix_blame::Error::Interrupted),
        "error should be Error::Interrupted"
    );

    Ok(())
}

/// Root commit entries should have `boundary: true` since the blame traversal
/// stopped there (no further parents to traverse).
#[test]
fn boundary_marks_root_commit_entries() -> gix_testtools::Result {
    let Fixture {
        odb,
        mut resource_cache,
        suspect,
    } = Fixture::new()?;

    let source_file_name: gix_object::bstr::BString = "simple.txt".into();

    let outcome = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: None,
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?;

    // simple.txt has 4 lines, each from a different commit. The first line
    // was introduced by the root commit, which should be a boundary.
    assert_eq!(outcome.entries.len(), 4);

    // The root commit entry (the last one in traversal order, line 1) should be boundary.
    let root_entry = outcome
        .entries
        .iter()
        .find(|e| e.boundary)
        .expect("there should be at least one boundary entry (root commit)");

    // The root commit introduces line 1 (0-indexed: start_in_blamed_file == 0)
    assert_eq!(
        root_entry.start_in_blamed_file, 0,
        "root commit entry should be for line 1"
    );

    // Non-root entries should not be boundary
    let non_boundary_count = outcome.entries.iter().filter(|e| !e.boundary).count();
    assert_eq!(non_boundary_count, 3, "3 entries should not be boundary");

    Ok(())
}

/// Test that `oldest_commit` stops traversal at the specified commit and marks
/// remaining entries as boundary.
#[test]
fn oldest_commit_stops_traversal() -> gix_testtools::Result {
    let Fixture {
        odb,
        mut resource_cache,
        suspect,
    } = Fixture::new()?;

    let source_file_name: gix_object::bstr::BString = "simple.txt".into();

    // First, do a normal blame to find commit IDs.
    let normal = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: None,
            oldest_commit: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?;

    assert_eq!(normal.entries.len(), 4);

    // Use the commit that introduced line 2 (index 1) as oldest_commit.
    // This means we stop before going further back, so line 1 (from the root)
    // should be attributed to the commit that introduced line 2, as a boundary.
    let commit_for_line2 = normal.entries[1].commit_id;

    let outcome = gix_blame::file(
        &odb,
        suspect,
        None,
        &mut resource_cache,
        source_file_name.as_ref(),
        gix_blame::Options {
            diff_algorithm: gix_diff::blob::Algorithm::Histogram,
            ranges: BlameRanges::default(),
            since: None,
            rewrites: Some(gix_diff::Rewrites::default()),
            debug_track_path: false,
            ignore_revs: Vec::new(),
            worktree_blob: None,
            oldest_commit: Some(commit_for_line2),
        },
        &std::sync::atomic::AtomicBool::new(false),
    )?;

    // All 4 lines should still be covered.
    let total_lines: u32 = outcome.entries.iter().map(|e| e.len.get()).sum();
    assert_eq!(total_lines, 4, "all 4 lines should be covered");

    // Lines 2-4 should still be attributed to their original commits (not boundary).
    // Line 1 should be attributed as boundary (traversal stopped before reaching the root).
    let boundary_entries: Vec<_> = outcome.entries.iter().filter(|e| e.boundary).collect();
    assert!(
        !boundary_entries.is_empty(),
        "there should be at least one boundary entry when oldest_commit is set"
    );

    // Lines that were introduced AFTER the oldest_commit should not be boundary.
    for entry in &outcome.entries {
        if entry.commit_id != commit_for_line2 && !entry.boundary {
            // These entries are from commits between HEAD and oldest_commit's parent,
            // so they should have real attributions.
            assert_ne!(
                entry.commit_id,
                ObjectId::null(gix_hash::Kind::Sha1),
                "non-boundary entries should have real commit ids"
            );
        }
    }

    Ok(())
}

fn fixture_path() -> gix_testtools::Result<PathBuf> {
    gix_testtools::scripted_fixture_read_only("make_blame_repo.sh")
}
