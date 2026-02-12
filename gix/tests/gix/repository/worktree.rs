use gix_ref::bstr;

#[cfg(target_pointer_width = "64")]
#[cfg(feature = "worktree-stream")]
const EXPECTED_BUFFER_LENGTH: usize = 102;
#[cfg(target_pointer_width = "32")]
#[cfg(feature = "worktree-stream")]
const EXPECTED_BUFFER_LENGTH: usize = 86;

#[test]
#[cfg(feature = "worktree-stream")]
fn stream() -> crate::Result {
    let repo = crate::named_repo("make_packed_and_loose.sh")?;
    let mut stream = repo.worktree_stream(repo.head_commit()?.tree_id()?)?.0.into_read();
    assert_eq!(
        std::io::copy(&mut stream, &mut std::io::sink())?,
        EXPECTED_BUFFER_LENGTH as u64,
        "there is some content in the stream, it works"
    );
    Ok(())
}

#[test]
#[cfg(feature = "worktree-archive")]
fn archive() -> crate::Result {
    let repo = crate::named_repo("make_packed_and_loose.sh")?;
    let (stream, _index) = repo.worktree_stream(repo.head_commit()?.tree_id()?)?;
    let mut buf = Vec::<u8>::new();

    repo.worktree_archive(
        stream,
        std::io::Cursor::new(&mut buf),
        gix_features::progress::Discard,
        &std::sync::atomic::AtomicBool::default(),
        Default::default(),
    )?;
    assert_eq!(buf.len(), EXPECTED_BUFFER_LENGTH, "default format is internal");
    Ok(())
}

mod with_core_worktree_config {
    use std::io::BufRead;

    #[test]
    #[cfg(feature = "index")]
    fn relative() -> crate::Result {
        for (name, is_relative) in [("absolute-worktree", false), ("relative-worktree", true)] {
            let repo = repo(name);

            if is_relative {
                assert_eq!(
                    repo.workdir().unwrap(),
                    repo.git_dir().parent().unwrap().parent().unwrap().join("worktree"),
                    "{name}|{is_relative}: work_dir is set to core.worktree config value, relative paths are appended to `git_dir() and made absolute`"
                );
            } else {
                assert_eq!(
                    repo.workdir().unwrap(),
                    gix_path::realpath(repo.git_dir().parent().unwrap().parent().unwrap().join("worktree"))?,
                    "absolute workdirs are left untouched"
                );
            }

            assert_eq!(
                repo.worktree().expect("present").base(),
                repo.workdir().unwrap(),
                "current worktree is based on work-tree dir"
            );

            let baseline = crate::repository::worktree::Baseline::collect(repo.git_dir())?;
            assert_eq!(baseline.len(), 1, "git lists the main worktree");
            assert_eq!(
                baseline[0].root,
                gix_path::realpath(repo.git_dir().parent().unwrap())?,
                "git lists the original worktree, to which we have no access anymore"
            );
            assert_eq!(
                repo.worktrees()?.len(),
                0,
                "we only list linked worktrees, and there are none"
            );
            assert_eq!(
                repo.index()?.entries().len(),
                count_deleted(repo.git_dir()),
                "git considers all worktree entries missing as the overridden worktree is an empty dir"
            );
            assert_eq!(repo.index()?.entries().len(), 3, "just to be sure");
        }
        Ok(())
    }

    #[test]
    fn non_existing_relative() {
        let repo = repo("relative-nonexisting-worktree");
        assert_eq!(
            count_deleted(repo.git_dir()),
            0,
            "git can't chdir into missing worktrees, has no error handling there"
        );

        assert!(
            !repo.workdir().expect("configured").exists(),
            "non-existing or invalid worktrees (this one is a file) are taken verbatim and \
            may lead to errors later - just like in `git` and we explicitly do not try to be smart about it"
        );
    }

    #[test]
    fn relative_file() {
        let repo = repo("relative-worktree-file");
        assert_eq!(count_deleted(repo.git_dir()), 0, "git can't chdir into a file");

        assert!(
            repo.workdir().expect("configured").is_file(),
            "non-existing or invalid worktrees (this one is a file) are taken verbatim and \
            may lead to errors later - just like in `git` and we explicitly do not try to be smart about it"
        );
    }

    #[test]
    #[cfg(feature = "index")]
    fn bare_relative() -> crate::Result {
        let repo = repo("bare-relative-worktree");

        assert_eq!(
            count_deleted(repo.git_dir()),
            0,
            "git refuses to mix bare with core.worktree"
        );
        assert!(
            repo.workdir().is_none(),
            "we simply don't load core.worktree in bare repos either to match this behaviour"
        );
        assert!(repo.try_index()?.is_none());
        assert!(repo.index_or_empty()?.entries().is_empty());
        Ok(())
    }

    fn repo(name: &str) -> gix::Repository {
        let dir = gix_testtools::scripted_fixture_read_only("make_core_worktree_repo.sh").unwrap();
        gix::open_opts(dir.join(name), crate::restricted()).unwrap()
    }

    fn count_deleted(git_dir: &std::path::Path) -> usize {
        std::fs::read(git_dir.join("status.baseline"))
            .unwrap()
            .lines()
            .map_while(Result::ok)
            .filter(|line| line.contains(" D "))
            .count()
    }
}

struct Baseline<'a> {
    lines: bstr::Lines<'a>,
}

mod baseline {
    use std::{
        borrow::Cow,
        path::{Path, PathBuf},
    };

    use gix::bstr::{BString, ByteSlice};
    use gix_object::bstr::BStr;

    use super::Baseline;

    impl Baseline<'_> {
        pub fn collect(dir: impl AsRef<Path>) -> std::io::Result<Vec<Worktree>> {
            let content = std::fs::read(dir.as_ref().join("worktree-list.baseline"))?;
            Ok(Baseline { lines: content.lines() }.collect())
        }
    }

    pub type Reason = BString;

    #[derive(Debug)]
    #[allow(dead_code)]
    pub struct Worktree {
        pub root: PathBuf,
        pub bare: bool,
        pub locked: Option<Reason>,
        pub peeled: gix_hash::ObjectId,
        pub branch: Option<BString>,
        pub prunable: Option<Reason>,
    }

    impl Iterator for Baseline<'_> {
        type Item = Worktree;

        fn next(&mut self) -> Option<Self::Item> {
            let root = gix_path::from_bstr(Cow::Borrowed(fields(self.lines.next()?).1)).into_owned();
            let mut bare = false;
            let mut branch = None;
            let mut peeled = gix_hash::ObjectId::null(gix_hash::Kind::Sha1);
            let mut locked = None;
            let mut prunable = None;
            for line in self.lines.by_ref() {
                if line.is_empty() {
                    break;
                }
                if line == b"bare" {
                    bare = true;
                    continue;
                } else if line == b"detached" {
                    continue;
                }
                let (field, value) = fields(line);
                match field {
                    f if f == "HEAD" => peeled = gix_hash::ObjectId::from_hex(value).expect("valid hash"),
                    f if f == "branch" => branch = Some(value.to_owned()),
                    f if f == "locked" => locked = Some(value.to_owned()),
                    f if f == "prunable" => prunable = Some(value.to_owned()),
                    _ => unreachable!("unknown field: {}", field),
                }
            }
            Some(Worktree {
                root,
                bare,
                locked,
                peeled,
                branch,
                prunable,
            })
        }
    }

    fn fields(line: &[u8]) -> (&BStr, &BStr) {
        let (a, b) = line.split_at(line.find_byte(b' ').expect("at least a space"));
        (a.as_bstr(), b[1..].as_bstr())
    }
}

#[test]
fn from_bare_parent_repo() {
    if gix_testtools::should_skip_as_git_version_is_smaller_than(2, 31, 0) {
        return;
    }
    let dir = gix_testtools::scripted_fixture_read_only_with_args("make_worktree_repo.sh", ["bare"]).unwrap();
    let repo = gix::open(dir.join("repo.git")).unwrap();

    run_assertions(repo, true /* bare */);
}

#[test]
fn from_nonbare_parent_repo() {
    if gix_testtools::should_skip_as_git_version_is_smaller_than(2, 31, 0) {
        return;
    }
    let dir = gix_testtools::scripted_fixture_read_only("make_worktree_repo.sh").unwrap();
    let repo = gix::open(dir.join("repo")).unwrap();

    run_assertions(repo, false /* bare */);
}

#[test]
fn from_nonbare_parent_repo_set_workdir() -> gix_testtools::Result {
    if gix_testtools::should_skip_as_git_version_is_smaller_than(2, 31, 0) {
        return Ok(());
    }

    let dir = gix_testtools::scripted_fixture_read_only("make_worktree_repo.sh").unwrap();
    let mut repo = gix::open(dir.join("repo")).unwrap();

    assert!(repo.worktree().is_some_and(|wt| wt.is_main()), "we have main worktree");

    let worktrees = repo.worktrees()?;
    assert_eq!(worktrees.len(), 6);

    let linked_wt_dir = worktrees.first().unwrap().base().expect("this linked worktree exists");
    repo.set_workdir(linked_wt_dir).expect("works as the dir exists");

    assert!(
        repo.worktree().is_some_and(|wt| wt.is_main()),
        "it's still the main worktree as that depends on the git_dir"
    );

    let mut wt_repo = repo.worktrees()?.first().unwrap().clone().into_repo()?;
    assert!(
        wt_repo.worktree().is_some_and(|wt| !wt.is_main()),
        "linked worktrees are never main"
    );

    wt_repo.set_workdir(Some(repo.workdir().unwrap().to_owned()))?;
    assert!(
        wt_repo.worktree().is_some_and(|wt| !wt.is_main()),
        "it's still the linked worktree as that depends on the git_dir"
    );

    Ok(())
}

/// Tests for worktree add and remove operations
#[cfg(feature = "worktree-mutation")]
mod mutation {
    use gix::bstr::ByteSlice;

    fn repo_rw() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
        let dir = gix_testtools::scripted_fixture_writable("make_worktree_repo_for_mutation.sh")?;
        let repo = gix::open_opts(dir.path().join("repo"), crate::restricted())?;
        Ok((repo, dir))
    }

    fn bare_repo_rw() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
        let dir = gix_testtools::scripted_fixture_writable("make_worktree_repo_for_mutation.sh")?;
        let repo = gix::open_opts(dir.path().join("repo.git"), crate::restricted())?;
        Ok((repo, dir))
    }

    mod add {
        use std::path::PathBuf;

        use super::*;

        #[test]
        fn detached_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-detached");

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;
            let base = proxy.base()?;

            assert_eq!(base, worktree_path);
            assert!(worktree_path.join(".git").is_file(), ".git file should exist");
            assert!(worktree_path.join("a").is_file(), "files should be checked out");

            // Verify HEAD is detached
            let wt_repo = proxy.into_repo()?;
            let head = wt_repo.head()?;
            assert!(
                head.referent_name().is_none(),
                "HEAD should be detached, not pointing to a branch"
            );
            Ok(())
        }

        #[test]
        fn with_existing_branch() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-feature");

            let options = gix::worktree::add::Options {
                branch: Some(b"feature-1".as_bstr()),
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;
            let wt_repo = proxy.into_repo()?;

            // Verify HEAD points to the branch
            let head_name = wt_repo.head_name()?.expect("should have branch");
            assert_eq!(head_name.as_bstr(), "refs/heads/feature-1");
            Ok(())
        }

        #[test]
        fn with_new_branch() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-new-branch");

            let options = gix::worktree::add::Options {
                new_branch: Some(b"my-new-branch".as_bstr()),
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;

            // Verify branch was created in main repo
            assert!(
                repo.try_find_reference("refs/heads/my-new-branch")?.is_some(),
                "new branch should exist in main repo"
            );

            // Verify worktree HEAD points to new branch
            let wt_repo = proxy.into_repo()?;
            let head_name = wt_repo.head_name()?.expect("should have branch");
            assert_eq!(head_name.as_bstr(), "refs/heads/my-new-branch");
            Ok(())
        }

        #[test]
        fn with_lock() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-locked");

            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;

            assert!(proxy.is_locked(), "worktree should be locked");
            assert!(proxy.git_dir().join("locked").is_file(), "locked file should exist");
            Ok(())
        }

        #[test]
        fn no_checkout() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-no-checkout");

            let options = gix::worktree::add::Options {
                detach: true,
                no_checkout: true,
                ..Default::default()
            };

            let _proxy = repo.worktree_add(&worktree_path, options)?;

            assert!(worktree_path.join(".git").is_file(), ".git file should exist");
            assert!(!worktree_path.join("a").exists(), "files should NOT be checked out");
            Ok(())
        }

        #[test]
        fn fails_on_existing_non_empty_path() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-existing");

            // Create a non-empty directory
            std::fs::create_dir_all(&worktree_path)?;
            std::fs::write(worktree_path.join("file"), "content")?;

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };

            let result = repo.worktree_add(&worktree_path, options);
            assert!(
                matches!(result, Err(gix::worktree::add::Error::PathExists { .. })),
                "should fail with PathExists error, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn succeeds_on_empty_directory() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-empty-dir");

            // Create an empty directory
            std::fs::create_dir_all(&worktree_path)?;

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;
            assert!(proxy.base()?.exists());
            Ok(())
        }

        #[test]
        fn fails_when_branch_already_checked_out() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            // Create first worktree with feature-1 branch
            let wt1_path = repo.workdir().unwrap().parent().unwrap().join("wt-feature1");
            let options = gix::worktree::add::Options {
                branch: Some(b"feature-1".as_bstr()),
                ..Default::default()
            };
            let _proxy1 = repo.worktree_add(&wt1_path, options)?;

            // Try to create another worktree with the same branch
            let wt2_path = repo.workdir().unwrap().parent().unwrap().join("wt-feature1-dup");
            let options = gix::worktree::add::Options {
                branch: Some(b"feature-1".as_bstr()),
                ..Default::default()
            };
            let result = repo.worktree_add(&wt2_path, options);

            assert!(
                matches!(result, Err(gix::worktree::add::Error::BranchCheckedOut { .. })),
                "should fail because branch is already checked out, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn fails_with_nonexistent_branch() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-nonexistent");

            let options = gix::worktree::add::Options {
                branch: Some(b"nonexistent-branch".as_bstr()),
                ..Default::default()
            };

            let result = repo.worktree_add(&worktree_path, options);
            assert!(
                matches!(result, Err(gix::worktree::add::Error::BranchNotFound { .. })),
                "should fail with BranchNotFound error, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn fails_with_mutually_exclusive_options() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-invalid");

            let options = gix::worktree::add::Options {
                branch: Some(b"feature-1".as_bstr()),
                detach: true, // Can't have both branch and detach
                ..Default::default()
            };

            let result = repo.worktree_add(&worktree_path, options);
            assert!(
                matches!(result, Err(gix::worktree::add::Error::MutuallyExclusiveOptions)),
                "should fail with MutuallyExclusiveOptions error, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn from_bare_repo_with_new_branch() -> crate::Result {
            let (repo, _keep) = bare_repo_rw()?;
            let worktree_path: PathBuf = repo.git_dir().parent().unwrap().join("wt-from-bare");

            let options = gix::worktree::add::Options {
                new_branch: Some(b"bare-branch".as_bstr()),
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;

            assert!(worktree_path.join(".git").is_file());
            assert!(worktree_path.join("a").is_file(), "files should be checked out");

            let wt_repo = proxy.into_repo()?;
            let head_name = wt_repo.head_name()?.expect("should have branch");
            assert_eq!(head_name.as_bstr(), "refs/heads/bare-branch");
            Ok(())
        }

        #[test]
        fn generates_unique_id_for_duplicate_names() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let parent = repo.workdir().unwrap().parent().unwrap();

            // Create worktrees with the same basename from different paths
            let wt1_path = parent.join("wt-same");
            let wt2_path = parent.join("subdir").join("wt-same");
            std::fs::create_dir_all(wt2_path.parent().unwrap())?;

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };

            let proxy1 = repo.worktree_add(&wt1_path, options.clone())?;
            let proxy2 = repo.worktree_add(&wt2_path, options)?;

            // Both should exist with different IDs
            assert_ne!(proxy1.id(), proxy2.id(), "IDs should be different");
            Ok(())
        }

        #[test]
        fn with_lock_and_reason() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-locked-reason");

            let lock_reason = "this is why it's locked";
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                lock_reason: Some(lock_reason.into()),
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;

            assert!(proxy.is_locked(), "worktree should be locked");
            assert_eq!(
                proxy.lock_reason().as_deref().map(|s| s.as_bstr()),
                Some(lock_reason.as_bytes().as_bstr()),
                "lock reason should match"
            );

            // Also verify the locked file contains the reason
            let locked_content = std::fs::read_to_string(proxy.git_dir().join("locked"))?;
            assert_eq!(locked_content, lock_reason);
            Ok(())
        }

        #[test]
        fn reason_without_lock_still_locks() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-reason-only");

            // Providing a lock_reason without lock=true should still lock
            let options = gix::worktree::add::Options {
                detach: true,
                lock_reason: Some(b"reason implies lock".as_bstr()),
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;
            assert!(proxy.is_locked(), "providing lock_reason should imply lock");
            Ok(())
        }

        #[test]
        fn from_linked_checkout() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            // First create a linked worktree
            let first_wt_path = repo.workdir().unwrap().parent().unwrap().join("wt-first");
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let first_proxy = repo.worktree_add(&first_wt_path, options)?;
            let first_wt_repo = first_proxy.into_repo()?;

            // Now add another worktree from the linked checkout
            let second_wt_path = repo.workdir().unwrap().parent().unwrap().join("wt-nested");
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let second_proxy = first_wt_repo.worktree_add(&second_wt_path, options)?;

            assert!(second_wt_path.join(".git").is_file(), ".git file should exist");
            assert!(second_wt_path.join("a").is_file(), "files should be checked out");

            // The second worktree should be visible from the main repo
            let worktrees = repo.worktrees()?;
            assert_eq!(worktrees.len(), 2, "both worktrees should be visible from main repo");

            // Both should be visible from the linked worktree too
            let wts_from_linked = first_wt_repo.worktrees()?;
            assert_eq!(
                wts_from_linked.len(),
                2,
                "both worktrees should be visible from linked wt"
            );

            // Verify the second worktree is a valid repo
            let second_repo = second_proxy.into_repo()?;
            assert!(second_repo.head().is_ok());
            Ok(())
        }

        #[test]
        fn from_subdir() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            // Test adding a worktree using a path that is a sibling of the workdir.
            // Instead of changing the process-global CWD (which breaks parallel tests),
            // we compute an absolute path that a relative "../sibling-wt" would resolve to.
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("sibling-wt");

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;

            // Verify the worktree was created correctly
            let base = proxy.base()?;
            assert!(base.is_dir());
            assert!(base.join("a").is_file(), "files should be checked out");
            Ok(())
        }

        #[test]
        fn with_start_point() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-start-point");

            // Get the parent commit of HEAD
            let head_commit = repo.head_commit()?;
            let parent_id = head_commit.parent_ids().next().expect("should have parent");
            let parent_id_str = parent_id.to_string();

            let options = gix::worktree::add::Options {
                detach: true,
                start_point: Some(parent_id_str.as_str().into()),
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;
            let wt_repo = proxy.into_repo()?;

            // Verify HEAD is at the parent commit
            let wt_head_id = wt_repo.head()?.try_peel_to_id()?.expect("should have id").detach();
            assert_eq!(wt_head_id, parent_id, "HEAD should be at start_point");
            Ok(())
        }

        #[test]
        fn new_branch_with_start_point() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-new-branch-sp");

            // Get the parent commit of HEAD
            let head_commit = repo.head_commit()?;
            let parent_id = head_commit.parent_ids().next().expect("should have parent");
            let parent_id_str = parent_id.to_string();

            let options = gix::worktree::add::Options {
                new_branch: Some(b"branch-at-parent".as_bstr()),
                start_point: Some(parent_id_str.as_str().into()),
                ..Default::default()
            };

            let proxy = repo.worktree_add(&worktree_path, options)?;
            let wt_repo = proxy.into_repo()?;

            // Verify HEAD points to the new branch
            let head_name = wt_repo.head_name()?.expect("should have branch");
            assert_eq!(head_name.as_bstr(), "refs/heads/branch-at-parent");

            // Verify the branch is at the parent commit
            let wt_head_id = wt_repo.head_id()?;
            assert_eq!(wt_head_id, parent_id, "branch should be at start_point");
            Ok(())
        }
    }

    mod remove {
        use super::*;

        #[test]
        fn removes_worktree_and_git_dir() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-to-remove");

            // First add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();
            let git_dir = proxy.git_dir().to_owned();

            assert!(worktree_path.exists());
            assert!(git_dir.exists());

            // Now remove it
            repo.worktree_remove(worktree_id.as_bstr(), Default::default())?;

            assert!(!worktree_path.exists(), "worktree directory should be removed");
            assert!(!git_dir.exists(), "worktree git directory should be removed");
            Ok(())
        }

        #[test]
        fn fails_on_locked_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-locked-remove");

            // Add a locked worktree
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Try to remove without force
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());

            assert!(
                matches!(result, Err(gix::worktree::remove::Error::Locked { .. })),
                "should fail because worktree is locked, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn force_removes_locked_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-force-remove");

            // Add a locked worktree
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Remove with force level 2 (required for locked worktrees)
            let remove_options = gix::worktree::remove::Options { force: 2 };
            repo.worktree_remove(worktree_id.as_bstr(), remove_options)?;

            assert!(
                !worktree_path.exists(),
                "locked worktree should be removed with force level 2"
            );
            Ok(())
        }

        #[test]
        fn fails_on_nonexistent_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            let result = repo.worktree_remove(b"nonexistent".as_bstr(), Default::default());

            assert!(
                matches!(result, Err(gix::worktree::remove::Error::NotFound { .. })),
                "should fail with NotFound error, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn handles_missing_worktree_directory() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-missing-dir");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();
            let git_dir = proxy.git_dir().to_owned();

            // Manually delete the worktree directory (simulating prunable state)
            std::fs::remove_dir_all(&worktree_path)?;

            // Remove should still work (cleaning up the git dir)
            repo.worktree_remove(worktree_id.as_bstr(), Default::default())?;

            assert!(!git_dir.exists(), "worktree git directory should be removed");
            Ok(())
        }

        #[test]
        fn cleans_up_worktrees_dir_when_last_removed() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktrees_dir = repo.common_dir().join("worktrees");

            // Initially there should be no worktrees directory
            assert!(!worktrees_dir.exists(), "worktrees dir should not exist initially");

            // Add a worktree
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-cleanup-test");
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            assert!(worktrees_dir.exists(), "worktrees dir should exist after add");

            // Remove the worktree
            repo.worktree_remove(worktree_id.as_bstr(), Default::default())?;

            assert!(
                !worktrees_dir.exists(),
                "worktrees dir should be removed when last worktree is removed"
            );
            Ok(())
        }

        #[test]
        fn does_not_remove_worktrees_dir_when_others_remain() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktrees_dir = repo.common_dir().join("worktrees");

            // Add two worktrees
            let wt1_path = repo.workdir().unwrap().parent().unwrap().join("wt-remain-1");
            let wt2_path = repo.workdir().unwrap().parent().unwrap().join("wt-remain-2");

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy1 = repo.worktree_add(&wt1_path, options.clone())?;
            let wt1_id = proxy1.id().to_owned();
            let _proxy2 = repo.worktree_add(&wt2_path, options)?;

            // Remove only the first worktree
            repo.worktree_remove(wt1_id.as_bstr(), Default::default())?;

            assert!(
                worktrees_dir.exists(),
                "worktrees dir should still exist when other worktrees remain"
            );
            Ok(())
        }

        #[test]
        fn fails_on_missing_but_locked_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-missing-locked");

            // Add a locked worktree
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                lock_reason: Some(b"important work".as_bstr()),
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Manually delete the worktree directory (simulating prunable state)
            std::fs::remove_dir_all(&worktree_path)?;

            // Remove should fail because it's locked (even though the dir is missing)
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::remove::Error::Locked { .. })),
                "should fail because worktree is locked, got {:?}",
                result
            );

            // But with force level 2 it should succeed
            repo.worktree_remove(worktree_id.as_bstr(), gix::worktree::remove::Options { force: 2 })?;
            Ok(())
        }

        #[test]
        fn lock_reason_shown_in_error() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-lock-reason-error");

            let lock_reason = "my custom lock reason";
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                lock_reason: Some(lock_reason.into()),
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Try to remove without force
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());

            // Verify the error contains the lock reason
            match result {
                Err(gix::worktree::remove::Error::Locked { id, reason }) => {
                    assert_eq!(id.as_bstr(), worktree_id.as_bstr());
                    assert_eq!(
                        reason.as_ref().map(|s| s.as_bstr()),
                        Some(lock_reason.as_bytes().as_bstr()),
                        "lock reason should be in error"
                    );
                }
                other => panic!("expected Locked error, got {:?}", other),
            }
            Ok(())
        }

        #[test]
        fn fails_on_dirty_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-dirty-remove");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Make the worktree dirty by modifying a tracked file
            std::fs::write(worktree_path.join("a"), "modified content")?;

            // Try to remove without force - should fail
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::remove::Error::DirtyWorktree { .. })),
                "should fail because worktree is dirty, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn force_level_1_removes_dirty_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-dirty-force1");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Make the worktree dirty
            std::fs::write(worktree_path.join("a"), "modified content")?;

            // Remove with force level 1 - should succeed
            let remove_options = gix::worktree::remove::Options { force: 1 };
            repo.worktree_remove(worktree_id.as_bstr(), remove_options)?;

            assert!(!worktree_path.exists(), "dirty worktree should be removed with force 1");
            Ok(())
        }

        #[test]
        fn fails_on_untracked_files() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-untracked-remove");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Create an untracked file (not added to git)
            std::fs::write(worktree_path.join("untracked-file.txt"), "untracked content")?;

            // Try to remove without force - should fail
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::remove::Error::UntrackedFiles { .. })),
                "should fail because worktree has untracked files, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn force_level_1_removes_worktree_with_untracked_files() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-untracked-force1");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Create an untracked file
            std::fs::write(worktree_path.join("untracked-file.txt"), "untracked content")?;

            // Remove with force level 1 - should succeed
            let remove_options = gix::worktree::remove::Options { force: 1 };
            repo.worktree_remove(worktree_id.as_bstr(), remove_options)?;

            assert!(
                !worktree_path.exists(),
                "worktree with untracked files should be removed with force 1"
            );
            Ok(())
        }

        #[test]
        fn force_level_1_still_fails_on_locked() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-locked-force1");

            // Add a locked worktree
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Remove with force level 1 - should still fail because locked
            let remove_options = gix::worktree::remove::Options { force: 1 };
            let result = repo.worktree_remove(worktree_id.as_bstr(), remove_options);
            assert!(
                matches!(result, Err(gix::worktree::remove::Error::Locked { .. })),
                "force level 1 should still fail on locked worktrees, got {:?}",
                result
            );
            Ok(())
        }
    }

    mod lock_unlock {
        use super::*;

        #[test]
        fn lock_main_worktree_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            // The main worktree's id would need to be found via worktree()
            // but since we can't lock main worktree, we test via the API
            // Main worktree has no entry in worktrees/ so worktree_proxy_by_id won't find it
            let result = repo.worktree_lock(b"main".as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::lock::Error::NotFound { .. })),
                "main worktree should not be lockable (not found in worktrees/), got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn unlock_main_worktree_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            let result = repo.worktree_unlock(b"main".as_bstr());
            assert!(
                matches!(result, Err(gix::worktree::lock::UnlockError::NotFound { .. })),
                "main worktree should not be unlockable (not found in worktrees/), got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn lock_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-lock-test");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Verify not locked initially
            assert!(!proxy.is_locked());

            // Lock it
            repo.worktree_lock(worktree_id.as_bstr(), Default::default())?;

            // Verify locked via proxy
            let proxy = repo.worktree_proxy_by_id(worktree_id.as_bstr()).unwrap();
            assert!(proxy.is_locked());
            Ok(())
        }

        #[test]
        fn lock_with_reason() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-lock-reason");

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            let lock_reason = "external drive";
            repo.worktree_lock(
                worktree_id.as_bstr(),
                gix::worktree::lock::Options {
                    reason: Some(lock_reason.into()),
                },
            )?;

            let proxy = repo.worktree_proxy_by_id(worktree_id.as_bstr()).unwrap();
            assert!(proxy.is_locked());
            assert_eq!(
                proxy.lock_reason().as_deref().map(|s| s.as_bstr()),
                Some(lock_reason.as_bytes().as_bstr())
            );
            Ok(())
        }

        #[test]
        fn lock_already_locked_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-already-locked");

            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Try to lock again
            let result = repo.worktree_lock(worktree_id.as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::lock::Error::AlreadyLocked { .. })),
                "should fail because already locked, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn unlock_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-unlock-test");

            // Add a locked worktree
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            assert!(proxy.is_locked());

            // Unlock it
            repo.worktree_unlock(worktree_id.as_bstr())?;

            // Verify unlocked
            let proxy = repo.worktree_proxy_by_id(worktree_id.as_bstr()).unwrap();
            assert!(!proxy.is_locked());
            Ok(())
        }

        #[test]
        fn unlock_not_locked_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-unlock-not-locked");

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Try to unlock when not locked
            let result = repo.worktree_unlock(worktree_id.as_bstr());
            assert!(
                matches!(result, Err(gix::worktree::lock::UnlockError::NotLocked { .. })),
                "should fail because not locked, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn lock_nonexistent_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            let result = repo.worktree_lock(b"nonexistent".as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::lock::Error::NotFound { .. })),
                "should fail with NotFound, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn unlock_nonexistent_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            let result = repo.worktree_unlock(b"nonexistent".as_bstr());
            assert!(
                matches!(result, Err(gix::worktree::lock::UnlockError::NotFound { .. })),
                "should fail with NotFound, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn lock_from_another_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            // Create two worktrees
            let wt1_path = repo.workdir().unwrap().parent().unwrap().join("wt-lock-from-1");
            let wt2_path = repo.workdir().unwrap().parent().unwrap().join("wt-lock-from-2");

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy1 = repo.worktree_add(&wt1_path, options.clone())?;
            let wt1_id = proxy1.id().to_owned();
            let proxy2 = repo.worktree_add(&wt2_path, options)?;
            let wt2_repo = proxy2.into_repo()?;

            // Lock wt1 from wt2
            wt2_repo.worktree_lock(wt1_id.as_bstr(), Default::default())?;

            // Verify it's locked
            let proxy1 = repo.worktree_proxy_by_id(wt1_id.as_bstr()).unwrap();
            assert!(proxy1.is_locked(), "worktree should be locked from another worktree");
            Ok(())
        }

        #[test]
        fn lock_from_within_locked_worktree_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-self-lock");

            // Create and lock a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let wt_id = proxy.id().to_owned();
            let wt_repo = proxy.into_repo()?;

            // Try to lock it again from within itself
            let result = wt_repo.worktree_lock(wt_id.as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::lock::Error::AlreadyLocked { .. })),
                "should fail because already locked, got {:?}",
                result
            );
            Ok(())
        }
    }

    mod prune {
        use super::*;

        #[test]
        fn prune_missing_worktree_path() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-prune-missing");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();
            let git_dir = proxy.git_dir().to_owned();

            // Manually delete the worktree directory
            std::fs::remove_dir_all(&worktree_path)?;

            // Prune should remove the stale entry
            let pruned = repo.worktree_prune(Default::default())?;

            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].id.as_bstr(), worktree_id.as_bstr());
            assert!(!git_dir.exists(), "worktree git dir should be removed by prune");
            Ok(())
        }

        #[test]
        fn prune_dry_run() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-prune-dry-run");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let worktree_id = proxy.id().to_owned();
            let git_dir = proxy.git_dir().to_owned();

            // Manually delete the worktree directory
            std::fs::remove_dir_all(&worktree_path)?;

            // Dry run prune
            let pruned = repo.worktree_prune(gix::worktree::prune::Options { dry_run: true })?;

            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].id.as_bstr(), worktree_id.as_bstr());
            assert!(git_dir.exists(), "worktree git dir should NOT be removed in dry run");
            Ok(())
        }

        #[test]
        fn prune_skips_locked() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-prune-locked");

            // Add a locked worktree
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let git_dir = proxy.git_dir().to_owned();

            // Manually delete the worktree directory
            std::fs::remove_dir_all(&worktree_path)?;

            // Prune should NOT remove the locked entry
            let pruned = repo.worktree_prune(Default::default())?;

            assert!(pruned.is_empty(), "locked worktrees should not be pruned");
            assert!(git_dir.exists(), "locked worktree git dir should remain");
            Ok(())
        }

        #[test]
        fn prune_does_nothing_when_all_valid() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-prune-valid");

            // Add a valid worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let _proxy = repo.worktree_add(&worktree_path, options)?;

            // Prune should do nothing
            let pruned = repo.worktree_prune(Default::default())?;

            assert!(pruned.is_empty(), "valid worktrees should not be pruned");
            assert!(worktree_path.exists(), "worktree should still exist");
            Ok(())
        }

        #[test]
        fn prune_cleans_empty_worktrees_dir() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-prune-cleanup");
            let worktrees_dir = repo.common_dir().join("worktrees");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let _proxy = repo.worktree_add(&worktree_path, options)?;

            assert!(worktrees_dir.exists());

            // Delete the worktree directory
            std::fs::remove_dir_all(&worktree_path)?;

            // Prune should clean up
            let _pruned = repo.worktree_prune(Default::default())?;

            assert!(
                !worktrees_dir.exists(),
                "empty worktrees dir should be removed after prune"
            );
            Ok(())
        }

        #[test]
        fn prune_directories_without_gitdir() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktrees_dir = repo.common_dir().join("worktrees");

            // Manually create a worktree entry without a gitdir file
            let fake_wt_dir = worktrees_dir.join("fake-wt");
            std::fs::create_dir_all(&fake_wt_dir)?;
            // Don't create a gitdir file

            // Prune should remove it
            let pruned = repo.worktree_prune(Default::default())?;

            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].id.as_bstr(), b"fake-wt".as_bstr());
            assert!(
                matches!(pruned[0].reason, gix::worktree::prune::PruneReason::MissingGitdir),
                "should be pruned for missing gitdir"
            );
            assert!(!fake_wt_dir.exists(), "fake worktree dir should be removed");
            Ok(())
        }

        #[test]
        fn prune_directories_with_invalid_gitdir() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktrees_dir = repo.common_dir().join("worktrees");

            // Manually create a worktree entry with an empty gitdir file
            let fake_wt_dir = worktrees_dir.join("invalid-gitdir-wt");
            std::fs::create_dir_all(&fake_wt_dir)?;
            std::fs::write(fake_wt_dir.join("gitdir"), "")?; // Empty content is invalid

            // Prune should remove it
            let pruned = repo.worktree_prune(Default::default())?;

            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].id.as_bstr(), b"invalid-gitdir-wt".as_bstr());
            assert!(
                matches!(pruned[0].reason, gix::worktree::prune::PruneReason::InvalidGitdir),
                "should be pruned for invalid gitdir"
            );
            Ok(())
        }

        #[test]
        fn prune_files_inside_worktrees_dir() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktrees_dir = repo.common_dir().join("worktrees");

            // Create worktrees dir and put a file (not a directory) in it
            std::fs::create_dir_all(&worktrees_dir)?;
            std::fs::write(worktrees_dir.join("some-file"), "content")?;

            // Also create a valid worktree entry
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-with-file");
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let _proxy = repo.worktree_add(&worktree_path, options)?;

            // Prune should not fail and should not touch the file (we skip non-directories)
            let pruned = repo.worktree_prune(Default::default())?;

            // The file is not a directory, so it won't be considered a worktree entry
            assert!(pruned.is_empty(), "valid worktrees should not be pruned");
            // The file should still exist (we only process directories)
            assert!(worktrees_dir.join("some-file").exists());
            Ok(())
        }
    }

    mod move_worktree {
        use super::*;

        #[test]
        fn move_worktree_to_new_path() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let original_path = repo.workdir().unwrap().parent().unwrap().join("wt-to-move");
            let new_path = repo.workdir().unwrap().parent().unwrap().join("wt-moved");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&original_path, options)?;
            let worktree_id = proxy.id().to_owned();

            assert!(original_path.exists());
            assert!(!new_path.exists());

            // Move it
            repo.worktree_move(worktree_id.as_bstr(), &new_path)?;

            assert!(!original_path.exists(), "original path should not exist after move");
            assert!(new_path.exists(), "new path should exist after move");
            assert!(new_path.join(".git").exists(), ".git file should exist in new location");
            Ok(())
        }

        #[test]
        fn move_updates_gitdir_file() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let original_path = repo.workdir().unwrap().parent().unwrap().join("wt-move-gitdir");
            let new_path = repo.workdir().unwrap().parent().unwrap().join("wt-move-gitdir-new");

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&original_path, options)?;
            let worktree_id = proxy.id().to_owned();
            let gitdir_path = proxy.git_dir().join("gitdir");

            // Move it
            repo.worktree_move(worktree_id.as_bstr(), &new_path)?;

            // Verify gitdir file points to new location
            let gitdir_content = std::fs::read_to_string(&gitdir_path)?;
            assert!(
                gitdir_content.contains("wt-move-gitdir-new"),
                "gitdir should point to new location, got: {}",
                gitdir_content
            );
            Ok(())
        }

        #[test]
        fn move_locked_worktree_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let original_path = repo.workdir().unwrap().parent().unwrap().join("wt-move-locked");
            let new_path = repo.workdir().unwrap().parent().unwrap().join("wt-move-locked-new");

            // Add a locked worktree
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&original_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Try to move without force
            let result = repo.worktree_move(worktree_id.as_bstr(), &new_path);
            assert!(
                matches!(result, Err(gix::worktree::r#move::Error::Locked { .. })),
                "should fail because worktree is locked, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn move_locked_worktree_with_force() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let original_path = repo.workdir().unwrap().parent().unwrap().join("wt-move-locked-force");
            let new_path = repo
                .workdir()
                .unwrap()
                .parent()
                .unwrap()
                .join("wt-move-locked-force-new");

            // Add a locked worktree
            let options = gix::worktree::add::Options {
                detach: true,
                lock: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&original_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Move with force level 2
            let move_options = gix::worktree::r#move::Options { force: 2 };
            repo.worktree_move_opts(worktree_id.as_bstr(), &new_path, move_options)?;

            assert!(!original_path.exists(), "original should be gone");
            assert!(new_path.exists(), "new path should exist");
            Ok(())
        }

        #[test]
        fn move_to_existing_path_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let original_path = repo.workdir().unwrap().parent().unwrap().join("wt-move-exists");
            let existing_path = repo.workdir().unwrap().parent().unwrap().join("wt-move-exists-dest");

            // Create destination path
            std::fs::create_dir_all(&existing_path)?;

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&original_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Try to move to existing path
            let result = repo.worktree_move(worktree_id.as_bstr(), &existing_path);
            assert!(
                matches!(result, Err(gix::worktree::r#move::Error::DestinationExists { .. })),
                "should fail because destination exists, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn move_nonexistent_worktree_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let new_path = repo.workdir().unwrap().parent().unwrap().join("wt-nonexistent-dest");

            let result = repo.worktree_move(b"nonexistent".as_bstr(), &new_path);
            assert!(
                matches!(result, Err(gix::worktree::r#move::Error::NotFound { .. })),
                "should fail with NotFound, got {:?}",
                result
            );
            Ok(())
        }

        #[test]
        fn move_creates_parent_directories() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let original_path = repo.workdir().unwrap().parent().unwrap().join("wt-move-parent");
            let new_path = repo
                .workdir()
                .unwrap()
                .parent()
                .unwrap()
                .join("nested")
                .join("deep")
                .join("wt-moved");

            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&original_path, options)?;
            let worktree_id = proxy.id().to_owned();

            // Move to nested path (parent directories don't exist)
            repo.worktree_move(worktree_id.as_bstr(), &new_path)?;

            assert!(new_path.exists(), "worktree should be moved to nested path");
            Ok(())
        }
    }

    mod repair {
        use super::*;

        #[test]
        fn repair_missing_dot_git_file() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-repair-dotgit");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let _proxy = repo.worktree_add(&worktree_path, options)?;

            // Delete the .git file to simulate corruption
            std::fs::remove_file(worktree_path.join(".git"))?;
            assert!(!worktree_path.join(".git").exists());

            // Repair
            let repairs = repo.worktree_repair(&[worktree_path.clone()])?;

            assert_eq!(repairs.len(), 1);
            assert!(
                matches!(repairs[0].kind, gix::worktree::repair::RepairKind::GitFile),
                "should repair .git file"
            );
            assert!(worktree_path.join(".git").exists(), ".git file should be restored");
            Ok(())
        }

        #[test]
        fn repair_incorrect_gitdir_file() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-repair-gitdir");

            // Add a worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let proxy = repo.worktree_add(&worktree_path, options)?;
            let gitdir_path = proxy.git_dir().join("gitdir");

            // Move worktree manually (without using worktree_move)
            let new_path = repo.workdir().unwrap().parent().unwrap().join("wt-repair-gitdir-moved");
            std::fs::rename(&worktree_path, &new_path)?;

            // Repair should fix the gitdir file
            let repairs = repo.worktree_repair(&[new_path.clone()])?;

            // Should repair both .git file (now pointing to wrong place) and gitdir
            assert!(!repairs.is_empty(), "should have performed repairs");

            // Verify gitdir now points to correct location
            let gitdir_content = std::fs::read_to_string(&gitdir_path)?;
            assert!(
                gitdir_content.contains("wt-repair-gitdir-moved"),
                "gitdir should point to new location"
            );
            Ok(())
        }

        #[test]
        fn repair_nothing_when_valid() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-repair-valid");

            // Add a valid worktree
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let _proxy = repo.worktree_add(&worktree_path, options)?;

            // Repair should do nothing
            let repairs = repo.worktree_repair(&[worktree_path])?;

            assert!(repairs.is_empty(), "valid worktree should not need repair");
            Ok(())
        }

        #[test]
        fn repair_all_worktrees() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let wt1_path = repo.workdir().unwrap().parent().unwrap().join("wt-repair-all-1");
            let wt2_path = repo.workdir().unwrap().parent().unwrap().join("wt-repair-all-2");

            // Add two worktrees
            let options = gix::worktree::add::Options {
                detach: true,
                ..Default::default()
            };
            let _proxy1 = repo.worktree_add(&wt1_path, options.clone())?;
            let _proxy2 = repo.worktree_add(&wt2_path, options)?;

            // Delete .git files from both
            std::fs::remove_file(wt1_path.join(".git"))?;
            std::fs::remove_file(wt2_path.join(".git"))?;

            // Repair all (empty paths means repair all)
            let repairs = repo.worktree_repair(&[])?;

            assert_eq!(repairs.len(), 2, "should repair both worktrees");
            Ok(())
        }

        #[test]
        fn repair_nonexistent_path_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let nonexistent = repo.workdir().unwrap().parent().unwrap().join("nonexistent-repair");

            let result = repo.worktree_repair(&[nonexistent]);
            assert!(
                matches!(result, Err(gix::worktree::repair::Error::InvalidPath { .. })),
                "should fail with InvalidPath, got {:?}",
                result
            );
            Ok(())
        }
    }
}

fn run_assertions(main_repo: gix::Repository, should_be_bare: bool) {
    assert_eq!(main_repo.is_bare(), should_be_bare);
    assert_eq!(main_repo.kind(), gix::repository::Kind::Common);
    let mut baseline = Baseline::collect(
        main_repo
            .workdir()
            .map_or_else(|| main_repo.git_dir().parent(), std::path::Path::parent)
            .expect("a temp dir as parent"),
    )
    .unwrap();
    let expected_main = baseline.remove(0);
    assert_eq!(expected_main.bare, should_be_bare);

    if should_be_bare {
        assert!(main_repo.worktree().is_none());
    } else {
        assert_eq!(
            main_repo.workdir().expect("non-bare").canonicalize().unwrap(),
            expected_main.root.canonicalize().unwrap()
        );
        assert_eq!(main_repo.head_id().unwrap(), expected_main.peeled);
        assert_eq!(
            main_repo.head_name().unwrap().expect("no detached head").as_bstr(),
            expected_main.branch.unwrap()
        );
        let worktree = main_repo.worktree().expect("not bare");
        assert!(
            worktree.lock_reason().is_none(),
            "main worktrees, bare or not, are never locked"
        );
        assert!(!worktree.is_locked());
        assert!(worktree.is_main());
    }
    assert_eq!(main_repo.main_repo().unwrap(), main_repo, "main repo stays main repo");

    let actual = main_repo.worktrees().unwrap();
    assert_eq!(actual.len(), baseline.len());

    for actual in actual {
        let base = actual.base().unwrap();
        let expected = baseline
            .iter()
            .find(|exp| exp.root == base)
            .expect("we get the same root and it matches");
        assert!(
            !expected.bare,
            "only the main worktree can be bare, and we don't see it in this loop"
        );
        let proxy_lock_reason = actual.lock_reason();
        assert_eq!(proxy_lock_reason, expected.locked);
        let proxy_is_locked = actual.is_locked();
        assert_eq!(proxy_is_locked, proxy_lock_reason.is_some());
        // TODO: check id of expected worktree, but need access to .gitdir from worktree base
        let proxy_id = actual.id().to_owned();
        assert_eq!(
            base.is_dir(),
            expected.prunable.is_none(),
            "in our case prunable repos have no worktree base"
        );

        assert_eq!(
            main_repo.worktree_proxy_by_id(actual.id()).expect("exists").git_dir(),
            actual.git_dir(),
            "we can basically get the same proxy by its ID explicitly"
        );

        let repo = if base.is_dir() {
            let repo = actual.clone().into_repo().unwrap();
            assert_eq!(
                &gix::open(base).unwrap(),
                &repo,
                "repos are considered the same no matter if opened from worktree or from git dir"
            );
            repo
        } else {
            assert!(
                matches!(
                    actual.clone().into_repo(),
                    Err(gix::worktree::proxy::into_repo::Error::MissingWorktree { .. })
                ),
                "missing bases are detected"
            );
            actual.clone().into_repo_with_possibly_inaccessible_worktree().unwrap()
        };
        let worktree = repo.worktree().expect("linked worktrees have at least a base path");
        assert!(!worktree.is_main());
        assert_eq!(worktree.lock_reason(), proxy_lock_reason);
        assert_eq!(worktree.is_locked(), proxy_is_locked);
        assert_eq!(worktree.id(), Some(proxy_id.as_ref()));
        assert_eq!(
            repo.main_repo().unwrap(),
            main_repo,
            "main repo from worktree repo is the actual main repo"
        );

        let proxy_by_id = repo
            .worktree_proxy_by_id(actual.id())
            .expect("can get the proxy from a linked repo as well");
        assert_ne!(
            proxy_by_id.git_dir(),
            actual.git_dir(),
            "The git directories might not look the same…"
        );
        assert_eq!(
            gix_path::realpath(proxy_by_id.git_dir()).ok(),
            gix_path::realpath(actual.git_dir()).ok(),
            "…but they are the same effectively"
        );
    }
}
