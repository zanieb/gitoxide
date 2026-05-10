use gix::Repository;

fn blob_id(repo: &Repository, data: &[u8]) -> gix_hash::ObjectId {
    gix_object::compute_hash(repo.object_hash(), gix_object::Kind::Blob, data).expect("valid object hash")
}

#[cfg(feature = "blame")]
mod blame;
mod config;
#[cfg(feature = "excludes")]
mod excludes;
#[cfg(feature = "attributes")]
mod filter;
#[cfg(feature = "hooks")]
mod hook;
#[cfg(feature = "merge")]
mod merge;
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

#[cfg(feature = "revision")]
mod revision {
    use crate::util::hex_to_id;

    #[test]
    fn date() -> crate::Result {
        let repo = crate::named_repo("make_rev_parse_repo.sh")?;
        let actual = repo
            .rev_parse_single("old@{20 years ago}")
            .expect("it returns the oldest possible rev when overshooting");
        assert_eq!(actual, hex_to_id("be2f093f0588eaeb71e1eff7451b18c2a9b1d765"));

        let actual = repo
            .rev_parse_single("old@{1732184844}")
            .expect("it finds something in the middle");
        assert_eq!(
            actual,
            hex_to_id("b29405fe9147a3a366c4048fbe295ea04de40fa6"),
            "It also figures out that we don't mean an index, but a date"
        );
        Ok(())
    }
}

#[cfg(feature = "index")]
mod index {
    use gix::bstr::ByteSlice;

    #[test]
    fn basics() -> crate::Result {
        let repo = crate::named_subrepo_opts("make_basic_repo.sh", "unborn", gix::open::Options::isolated())?;
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

            let outcome = repo.add_to_index([b"keep.txt".as_bstr(), b"ignored.log".as_bstr()], Default::default())?;

            assert_eq!(outcome.added_entries, 1);
            assert_eq!(outcome.ignored_entries, [b"ignored.log".as_bstr()]);
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
    use std::sync::atomic::AtomicBool;

    use gix_dir::{entry::Kind::*, walk::EmissionMode};

    #[test]
    fn basics() -> crate::Result {
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
        let expected = [
            ("all-untracked".to_string(), Repository),
            ("bare-repo-with-index.git".to_string(), Directory),
            ("bare.git".into(), Directory),
            ("empty-core-excludes".into(), Repository),
            ("non-bare-repo-without-index".into(), Repository),
            ("non-bare-without-worktree".into(), Directory),
            ("some".into(), Directory),
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
            .map(Result::unwrap)
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
    // Network-client features add protocol permission caching to `Repository::config`,
    // which grows the type by one more cached cell.
    let limit = 1300;
    assert!(
        actual_size <= limit,
        "size of Repository shouldn't change without us noticing, it's meant to be cloned: should have been below {limit:?}, was {actual_size}"
    );
}

#[test]
#[cfg(feature = "parallel")]
fn thread_safe_repository_is_sync() -> crate::Result {
    fn f<T: Send + Sync + Clone>(_t: T) {}
    f(crate::util::basic_repo()?.into_sync());
    Ok(())
}

#[test]
#[cfg(feature = "parallel")]
fn repository_is_send() -> crate::Result {
    fn f<T: Send + Clone>(_t: T) {}
    f(crate::util::basic_repo()?);
    Ok(())
}
