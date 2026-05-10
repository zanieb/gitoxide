use gix::Repository;
#[cfg(feature = "parallel")]
use gix_testtools::TestResult;

fn blob_id(repo: &Repository, data: &[u8]) -> gix_hash::ObjectId {
    gix_object::compute_hash(repo.object_hash(), gix_object::Kind::Blob, data).expect("valid object hash")
}

#[cfg(feature = "attributes")]
mod attributes;
#[cfg(feature = "blame")]
mod blame;
mod branch;
pub(crate) mod config;
#[cfg(feature = "excludes")]
mod excludes;
#[cfg(feature = "attributes")]
mod filter;
mod location;
#[cfg(feature = "mailmap")]
mod mailmap;
#[cfg(feature = "merge")]
mod merge;
#[cfg(feature = "notes")]
mod note;
mod object;
mod open;
#[cfg(feature = "attributes")]
mod pathspec;
mod reference;
mod remote;
mod shallow;
mod state;
#[cfg(feature = "attributes")]
mod submodule;
mod worktree;

mod format_version {
    use gix::repository::FormatVersion;
    use gix_error::MetadataValue;

    #[test]
    fn upgrade_to_v1_accepts_grandfathered_extensions() -> gix_testtools::TestResult {
        for input in [
            "",
            "[core]\nrepositoryFormatVersion = 0\n",
            "[EXTENSIONS]\nNoOp = true\nPreciousObjects = true\nPartialClone = origin\nWorktreeConfig = true\n",
            "[extensions]\nnoop = false\n[extensions]\nworktreeConfig = true\n",
        ] {
            let config = gix_config::File::try_from(input)?;
            FormatVersion::V0.validate_upgrade_to_v1(&config)?;
        }
        Ok(())
    }

    #[test]
    fn upgrade_to_v1_rejects_unknown_and_v1_only_extensions() -> gix_testtools::TestResult {
        for name in ["futureExtension", "objectFormat", "relativeWorktrees", "refStorage"] {
            let input = format!("[extensions]\nnoop = true\n[extensions]\n{name} = false\n");
            let config = gix_config::File::try_from(input.as_str())?;
            let error = FormatVersion::V0
                .validate_upgrade_to_v1(&config)
                .expect_err("upgrading must not activate an extension ignored in version 0");
            assert!(
                error.is_unsupported(),
                "extension activation requires a different strategy"
            );
            assert!(!error.is_validation(), "extension names are not malformed input");
            assert_eq!(
                error.metadata().next().and_then(|metadata| metadata.get("extension")),
                Some(&MetadataValue::from(name)),
                "the rejected extension is identified even if its value is false"
            );
        }
        Ok(())
    }

    #[test]
    fn upgrade_to_v1_rejects_extension_subsections() -> gix_testtools::TestResult {
        for subsection in ["", "scope"] {
            let input = format!("[extensions \"{subsection}\"]\nnoop = true\n");
            let config = gix_config::File::try_from(input.as_str())?;
            let error = FormatVersion::V0
                .validate_upgrade_to_v1(&config)
                .expect_err("even known extension names cannot have a subsection");
            assert!(error.is_validation(), "extension subsections prevent a safe upgrade");
            let metadata = error.metadata().next().expect("the invalid extension has metadata");
            assert_eq!(
                metadata.get("extension"),
                Some(&MetadataValue::from("noop")),
                "the otherwise-supported extension name remains visible in the diagnostic"
            );
            assert_eq!(
                metadata.get("subsection"),
                Some(&MetadataValue::from(subsection.as_bytes())),
                "the subsection is preserved, including an explicitly empty one"
            );
        }
        Ok(())
    }

    #[test]
    fn already_v1_does_not_repeat_upgrade_validation() -> gix_testtools::TestResult {
        let config = gix_config::File::try_from(
            "[extensions]\nobjectFormat = sha256\nrelativeWorktrees = true\nfutureExtension = true\n\
             [extensions \"scope\"]\nnoop = true\n",
        )?;
        FormatVersion::V1.validate_upgrade_to_v1(&config)?;
        Ok(())
    }
}

#[cfg(feature = "revision")]
mod revision;

#[cfg(feature = "index")]
mod index {
    use gix_testtools::TestResult;

    #[test]
    fn missing_shared_index_is_an_error() -> TestResult {
        let (repo, _tmp) = crate::basic_rw_repo()?;
        let workdir = repo.workdir().expect("the fixture has a worktree");
        gix_testtools::git(workdir, "update-index --split-index")?;
        assert_eq!(
            repo.open_index()?.entries().len(),
            1,
            "the split index is initially valid"
        );
        let shared_index = gix_testtools::git(workdir, "rev-parse --shared-index-path")?;
        std::fs::remove_file(workdir.join(shared_index.trim()))?;

        assert!(
            !gix_testtools::run_git(workdir, &["ls-files"])?.success(),
            "Git rejects a split index whose shared file is missing"
        );
        assert!(repo.try_index().is_err(), "the primary index still exists");
        assert!(repo.index_or_empty().is_err(), "a broken index must not become empty");
        assert!(
            repo.index_or_load_from_head().is_err(),
            "a broken index must not be replaced with HEAD"
        );
        assert!(
            repo.index_or_load_from_head_or_empty().is_err(),
            "neither fallback applies to a broken index"
        );
        Ok(())
    }

    #[test]
    fn basics() -> TestResult {
        let repo = crate::named_subrepo_opts("make_basic_repo.sh", "unborn", gix::open::Options::isolated())?;
        let err = repo.index().expect_err("the fixture has no index");
        insta::assert_debug_snapshot!(gix_testtools::redact_debug_snapshot(&err, &[(&repo.current_dir().join(repo.git_dir()).to_string_lossy(), "<git-dir>")]), "a missing index has standard not-found classification", @r#"
        Message {
            message: "Could not find index file at \"<git-dir>/index\" for opening.",
            class: NotFound,
        }
        "#);
        assert!(
            err.is_not_found(),
            "a missing index has standard not-found classification"
        );
        assert!(
            repo.index_or_load_from_head().is_err(),
            "can't read index if `HEAD^{{tree}}` can't be resolved"
        );
        assert!(
            repo.index_or_load_from_head_or_empty()?.entries().is_empty(),
            "an empty index is created on the fly"
        );
        assert_eq!(
            repo.is_pristine(),
            Some(false),
            "not pristine as it things the initial ref was changed to 'main'"
        );
        assert_eq!(
            repo.refs.is_pristine("refs/heads/main".try_into()?),
            Some(true),
            "This is a quirk of default values in gix and the way we override the initial branch for test fixtures"
        );
        Ok(())
    }

    #[cfg(feature = "dirwalk")]
    mod add {
        use super::*;

        fn repo_rw() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
            let tmp = gix_testtools::tempfile::tempdir()?;
            let repo = gix::ThreadSafeRepository::init_opts(
                tmp.path(),
                gix::create::Kind::WithWorktree,
                Default::default(),
                crate::restricted(),
            )?
            .to_thread_local();
            Ok((repo, tmp))
        }

        #[test]
        fn respects_gitignore_unless_forced() -> crate::Result {
            let (repo, _tmp) = repo_rw()?;
            let workdir = repo.workdir().expect("worktree");
            std::fs::write(workdir.join(".gitignore"), "*.log\n")?;
            std::fs::write(workdir.join("keep.txt"), "keep\n")?;
            std::fs::write(workdir.join("ignored.log"), "ignored\n")?;

            let err = repo
                .add_to_index([b"keep.txt".as_bstr(), b"ignored.log".as_bstr()], Default::default())
                .unwrap_err();
            match err {
                gix::repository::add_to_index::Error::Ignored { paths } => {
                    assert_eq!(paths, [b"ignored.log".as_bstr()]);
                }
                err => panic!("unexpected error: {err}"),
            }
            let index = repo.open_index()?;
            assert!(index.entry_by_path(b"keep.txt".as_bstr()).is_some());
            assert!(index.entry_by_path(b"ignored.log".as_bstr()).is_none());

            let outcome = repo.add_to_index(
                [b"ignored.log".as_bstr()],
                gix::repository::add_to_index::Options { force_ignored: true },
            )?;

            assert_eq!(outcome.added_entries, 1);
            let index = repo.open_index()?;
            assert!(index.entry_by_path(b"ignored.log".as_bstr()).is_some());
            Ok(())
        }

        #[test]
        fn refreshes_tracked_file_content() -> crate::Result {
            let (repo, _tmp) = repo_rw()?;
            let workdir = repo.workdir().expect("worktree");
            std::fs::write(workdir.join("tracked.txt"), "old\n")?;
            repo.add_to_index([b"tracked.txt".as_bstr()], Default::default())?;
            let old_id = repo
                .open_index()?
                .entry_by_path(b"tracked.txt".as_bstr())
                .expect("tracked")
                .id;

            std::fs::write(workdir.join("tracked.txt"), "new\n")?;
            let outcome = repo.add_to_index([b"tracked.txt".as_bstr()], Default::default())?;

            assert_eq!(outcome.added_entries, 1);
            let new_id = repo
                .open_index()?
                .entry_by_path(b"tracked.txt".as_bstr())
                .expect("tracked")
                .id;
            assert_ne!(old_id, new_id);
            assert_eq!(new_id, repo.write_blob("new\n")?.detach());
            Ok(())
        }

        #[test]
        fn stages_tracked_deletions() -> crate::Result {
            let (repo, _tmp) = repo_rw()?;
            let workdir = repo.workdir().expect("worktree");
            std::fs::write(workdir.join("gone.txt"), "gone\n")?;
            repo.add_to_index([b"gone.txt".as_bstr()], Default::default())?;
            std::fs::remove_file(workdir.join("gone.txt"))?;

            let outcome = repo.add_to_index([b"gone.txt".as_bstr()], Default::default())?;

            assert_eq!(outcome.removed_entries, 1);
            assert!(repo.open_index()?.entry_by_path(b"gone.txt".as_bstr()).is_none());
            Ok(())
        }

        #[test]
        fn errors_when_pathspec_matches_nothing() -> crate::Result {
            let (repo, _tmp) = repo_rw()?;

            let err = repo
                .add_to_index([b"missing.txt".as_bstr()], Default::default())
                .unwrap_err();

            match err {
                gix::repository::add_to_index::Error::PathspecsDidNotMatch { pathspecs } => {
                    assert_eq!(pathspecs, [b"missing.txt".as_bstr()]);
                }
                err => panic!("unexpected error: {err}"),
            }
            Ok(())
        }

        #[test]
        fn errors_for_explicit_ignored_path_without_force() -> crate::Result {
            let (repo, _tmp) = repo_rw()?;
            let workdir = repo.workdir().expect("worktree");
            std::fs::write(workdir.join(".gitignore"), "*.log\n")?;
            std::fs::write(workdir.join("ignored.log"), "ignored\n")?;

            let err = repo
                .add_to_index([b"ignored.log".as_bstr()], Default::default())
                .unwrap_err();

            match err {
                gix::repository::add_to_index::Error::Ignored { paths } => {
                    assert_eq!(paths, [b"ignored.log".as_bstr()]);
                }
                err => panic!("unexpected error: {err}"),
            }
            Ok(())
        }
    }
}

#[cfg(feature = "dirwalk")]
mod dirwalk {
    use gix_testtools::TestResult;
    use std::sync::atomic::AtomicBool;

    use gix_dir::{entry::Kind::*, walk::EmissionMode};

    #[test]
    fn basics() -> TestResult {
        let repo = crate::named_repo("make_basic_repo.sh")?;
        let untracked_only = repo.dirwalk_options()?.emit_untracked(EmissionMode::CollapseDirectory);
        let mut collect = gix::dir::walk::delegate::Collect::default();
        let index = repo.index()?;
        repo.dirwalk(
            &index,
            None::<&str>,
            &AtomicBool::default(),
            untracked_only,
            &mut collect,
        )?;
        // `some/` (a tree of only empty directories) is skipped now that empty trees collapse
        // to an empty directory and aren't emitted by default, matching Git which treats it as clean (#2490).
        let expected = [
            ("all-untracked".to_string(), Repository),
            ("bare-repo-with-index.git".to_string(), Directory),
            ("bare.git".into(), Directory),
            ("empty-core-excludes".into(), Repository),
            ("non-bare-repo-without-index".into(), Repository),
            ("non-bare-without-worktree".into(), Directory),
            ("repo.git".into(), Repository),
            ("some-with-file".into(), Directory),
            ("unborn".into(), Repository),
        ];
        assert_eq!(
            collect
                .into_entries_by_path()
                .into_iter()
                .map(|e| (e.0.rela_path.to_string(), e.0.disk_kind.expect("kind is known")))
                .collect::<Vec<_>>(),
            expected,
            "note how bare repos are just directories by default"
        );
        let mut iter = repo.dirwalk_iter(index, None::<&str>, Default::default(), untracked_only)?;
        let mut actual: Vec<_> = iter
            .by_ref()
            .map(std::result::Result::unwrap)
            .map(|item| {
                (
                    item.entry.rela_path.to_string(),
                    item.entry.disk_kind.expect("kind is known"),
                )
            })
            .collect();
        actual.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(actual, expected, "the iterator works the same");
        let out = iter.into_outcome().expect("iteration done and no error");
        assert_eq!(
            out.dirwalk.returned_entries,
            expected.len(),
            "just a minor sanity check, assuming everything else works as well"
        );
        Ok(())
    }
}

#[test]
fn size_in_memory() {
    let actual_size = std::mem::size_of::<Repository>();
    // The selected index path adds one `PathBuf` to the repository.
    // Network-client features add protocol permission caching to `Repository::config`,
    // which grows the type by one more cached cell.
    let limit = 1500;
    assert!(
        actual_size <= limit,
        "size of Repository shouldn't change without us noticing, it's meant to be cloned: should have been below {limit:?}, was {actual_size}"
    );
}

#[test]
#[cfg(feature = "parallel")]
fn thread_safe_repository_is_sync() -> TestResult {
    fn f<T: Send + Sync + Clone>(_t: T) {}
    f(crate::util::basic_repo()?.into_sync());
    Ok(())
}

#[test]
#[cfg(feature = "parallel")]
fn repository_is_send() -> TestResult {
    fn f<T: Send + Clone>(_t: T) {}
    f(crate::util::basic_repo()?);
    Ok(())
}
