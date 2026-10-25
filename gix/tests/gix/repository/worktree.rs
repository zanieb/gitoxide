use gix_ref::bstr;
use gix_testtools::{Result, TestResult};

#[cfg(feature = "worktree-mutation")]
mod lock {
    use gix::bstr::ByteSlice;

    #[test]
    fn set_lock_through_worktree_and_proxy_interoperates_with_git() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let linked = repo
            .prepare_add_worktree(
                destinations.path().join("linked"),
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                &std::sync::atomic::AtomicBool::default(),
            )?
            .persist()?;
        let worktree = linked.worktree().expect("linked worktree");
        let proxy = repo.worktree_proxy_by_id("linked")?.expect("persisted registration");
        let main = repo.workdir().expect("main checkout");

        worktree.set_lock(Some("external storage".into()))?;
        assert!(proxy.is_locked(), "the proxy sees the worktree's lock");
        assert_eq!(
            proxy.lock_reason(),
            Some("external storage".into()),
            "proxy reads the reason"
        );
        assert!(
            gix_testtools::git(main, "worktree list --porcelain")?.contains("locked external storage"),
            "Git reads the reason written through the worktree API"
        );
        gix_testtools::git(main, "worktree unlock linked")?;
        assert!(!worktree.is_locked(), "the worktree observes Git unlocking it");

        gix_testtools::git(main, "worktree lock --reason git-reason linked")?;
        let git_contents = std::fs::read(proxy.git_dir().join("locked"))?;
        assert_eq!(
            worktree.lock_reason(),
            Some("git-reason".into()),
            "worktree reads Git's reason"
        );
        proxy.set_lock(Some("git-reason".into()))?;
        assert_eq!(
            std::fs::read(proxy.git_dir().join("locked"))?,
            git_contents,
            "the setter uses Git's on-disk encoding"
        );
        proxy.set_lock(Some(b"".as_bstr()))?;
        assert!(worktree.is_locked(), "an empty reason still locks the worktree");
        assert_eq!(worktree.lock_reason(), Some("".into()), "the empty reason is retained");
        proxy.set_lock(None)?;
        worktree.set_lock(None)?;
        assert!(!proxy.is_locked(), "unlocking is idempotent across handles");

        std::fs::remove_dir_all(proxy.base()?)?;
        proxy.set_lock(Some("offline".into()))?;
        assert!(
            !proxy.is_prunable(),
            "an offline worktree can be protected from pruning"
        );
        proxy.set_lock(None)?;
        assert!(proxy.is_prunable(), "unlocking an offline worktree permits pruning");
        Ok(())
    }

    #[test]
    fn set_lock_rejects_the_main_worktree_without_changing_files() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let worktree = repo.worktree().expect("main worktree");
        let lock_path = repo.git_dir().join("locked");
        for reason in [Some("keep".into()), None] {
            let err = worktree
                .set_lock(reason)
                .expect_err("the main worktree cannot be locked");
            assert!(err.is_validation(), "main-worktree locking is invalid input");
            assert!(!lock_path.exists(), "rejected operations create no lock file");
        }
        std::fs::write(&lock_path, b"unrelated contents")?;
        assert!(
            !worktree.is_locked(),
            "an unrelated locked file does not lock the main worktree"
        );
        assert_eq!(
            worktree.lock_reason(),
            None,
            "the main worktree has no lock reason, regardless of unrelated files"
        );
        worktree
            .set_lock(None)
            .expect_err("the main worktree cannot be unlocked");
        assert_eq!(
            std::fs::read(lock_path)?,
            b"unrelated contents",
            "rejection preserves files"
        );
        Ok(())
    }
}

#[cfg(feature = "worktree-mutation")]
mod add {
    #[test]
    fn detached_worktree_without_a_configured_committer() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::util::repo_rw_opts("make_basic_repo.sh", gix::open::Options::isolated())?;
        assert!(repo.committer().is_none());
        let destination = gix_testtools::tempfile::TempDir::new()?;
        let commit = repo.head_id()?.detach();
        let (linked, _) = repo.add_worktree(
            destination.path().join("linked"),
            gix::worktree::add::Head::Detached(commit),
            gix::progress::Discard,
            &std::sync::atomic::AtomicBool::default(),
        )?;
        assert_eq!(linked.head_id()?.detach(), commit);
        let reopened = gix::open_opts(linked.git_dir(), gix::open::Options::isolated())?;
        assert!(reopened.committer().is_none());
        Ok(())
    }

    #[test]
    fn returned_repositories_and_preparation_have_absolute_paths() -> gix_testtools::TestResult {
        let (source, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let mut repo = gix::open_opts(
            source.workdir().expect("source checkout"),
            crate::restricted().config_overrides([
                "user.name=gitoxide",
                "user.email=gitoxide@localhost",
                "gitoxide.core.indexFile=selected-index",
            ]),
        )?;
        let commit_id = repo.head_id()?.detach();
        repo.config_snapshot_mut()
            .set_raw_value("worktree.useRelativePaths", "true")?;
        let main = repo.main_repo()?;
        let prepared = repo.prepare_add_worktree(
            destinations.path().join("linked"),
            gix::worktree::add::Head::Detached(commit_id),
            &AtomicBool::default(),
        )?;
        assert!(
            prepared.repo().index_path().is_absolute(),
            "preparation already has an absolute selected index path"
        );
        let linked = prepared.persist()?;
        assert_eq!(
            linked.index_path(),
            linked.git_dir().join("selected-index"),
            "the linked worktree selects its index in its private Git directory, not the common directory"
        );
        assert_ne!(
            linked.index_path(),
            linked.common_dir().join("selected-index"),
            "main and linked worktrees do not share the configured index"
        );
        let (checked_out, _) = repo.add_worktree(
            destinations.path().join("checked-out"),
            gix::worktree::add::Head::Detached(commit_id),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let proxy = repo.worktree_proxy_by_id("linked")?.expect("persisted registration");
        let strict = proxy.clone().into_repo()?;
        std::fs::remove_dir_all(proxy.base()?)?;
        let inaccessible = proxy.into_repo_with_possibly_inaccessible_worktree()?;
        let reopened_main = linked.main_repo()?;
        for returned in [main, linked, checked_out, strict, inaccessible, reopened_main] {
            assert!(
                returned.git_dir().is_absolute()
                    && returned.common_dir().is_absolute()
                    && returned.workdir().is_none_or(std::path::Path::is_absolute)
                    && returned.index_path().is_absolute()
                    && returned.objects.store_ref().path().is_absolute(),
                "every repository returned through worktree APIs has anchored paths"
            );
            assert_eq!(
                returned.index_path(),
                returned.git_dir().join("selected-index"),
                "each selected index uses its repository's private Git directory"
            );
        }
        Ok(())
    }

    #[test]
    fn staged_registration_persists_without_checkout_and_checks_out_later() -> gix_testtools::TestResult {
        for detached in [false, true] {
            for relative in [false, true] {
                let (mut repo, _fixture) = crate::basic_rw_repo()?;
                let mut config = repo.config_file_mut(repo.common_dir().join("config"))?;
                config.set_raw_value("worktree.useRelativePaths", if relative { "true" } else { "false" })?;
                config.commit()?;
                repo.reload()?;
                let destinations = gix_testtools::tempfile::TempDir::new()?;
                let path = destinations.path().join("staged");
                let commit_id = repo.head_id()?.detach();
                let head = if detached {
                    gix::worktree::add::Head::Detached(commit_id)
                } else {
                    let topic = branch("staged");
                    repo.reference(
                        topic.clone(),
                        commit_id,
                        PreviousValue::MustNotExist,
                        "create staged branch",
                    )?;
                    gix::worktree::add::Head::Attached(topic)
                };
                let prepared = repo.prepare_add_worktree(&path, head, &AtomicBool::default())?;
                assert_eq!(
                    prepared.repo().head_id()?.detach(),
                    commit_id,
                    "preparation installs HEAD"
                );
                assert!(
                    !path.join("this").exists() && !prepared.repo().index_path().exists(),
                    "registration does not check out files or write an index"
                );
                assert!(
                    prepared.repo().git_dir().join("locked").exists(),
                    "unfinished registration stays locked"
                );
                let linked = prepared.persist()?;
                assert!(
                    !linked.git_dir().join("locked").exists(),
                    "persistence completes registration"
                );
                let git_supports_registration = !relative || *gix_testtools::GIT_VERSION >= (2, 48, 0);
                if git_supports_registration {
                    assert!(
                        gix_testtools::git(repo.workdir().expect("source checkout"), "worktree list --porcelain")?
                            .contains("staged"),
                        "Git recognizes registration without checkout"
                    );
                }
                std::fs::remove_dir_all(&path)?;
                assert_eq!(
                    linked.head_id()?.detach(),
                    commit_id,
                    "repository data remains accessible without its checkout directory"
                );
                linked
                    .worktree()
                    .expect("persisted linked worktree remains available without its directory")
                    .checkout(gix::progress::Discard, &AtomicBool::default())?;
                assert!(path.join("this").is_file(), "later checkout restores tracked files");
                assert!(linked.index_path().is_file(), "later checkout writes its index");
                if git_supports_registration {
                    assert_eq!(
                        gix_testtools::git(&path, "status --porcelain")?,
                        "",
                        "later checkout recreates links, files and index"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn staged_drop_and_failed_checkout_clean_unpersisted_roots() -> gix_testtools::TestResult {
        for failure in ["drop", "cancel", "index-write"] {
            let (repo, _fixture) = crate::basic_rw_repo()?;
            let destinations = gix_testtools::tempfile::TempDir::new()?;
            let path = destinations.path().join("unfinished");
            let prepared = repo.prepare_add_worktree(
                &path,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                &AtomicBool::default(),
            )?;
            let git_dir = prepared.repo().git_dir().to_owned();
            if failure == "drop" {
                drop(prepared);
            } else {
                if failure == "index-write" {
                    std::fs::create_dir(prepared.repo().index_path())?;
                }
                prepared
                    .checkout(gix::progress::Discard, &AtomicBool::new(failure == "cancel"))
                    .expect_err("cancellation or an unwritable index fails checkout");
            }
            assert!(
                !path.exists() && !git_dir.exists(),
                "unfinished registration is rolled back"
            );
        }
        Ok(())
    }

    #[test]
    fn interruption_during_checkout_never_writes_an_index() -> gix_testtools::TestResult {
        use crate::util::InterruptCheckout;
        use std::sync::Arc;

        for persisted in [false, true] {
            let (repo, _fixture) = crate::basic_rw_repo()?;
            let destinations = gix_testtools::tempfile::TempDir::new()?;
            let path = destinations.path().join("interrupted");
            let should_interrupt = Arc::new(AtomicBool::default());
            let prepared = repo.prepare_add_worktree(
                &path,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                &should_interrupt,
            )?;
            let git_dir = prepared.repo().git_dir().to_owned();
            let index_path = prepared.repo().index_path().to_owned();
            let progress = InterruptCheckout(should_interrupt.clone());
            let result = if persisted {
                prepared
                    .persist()?
                    .worktree()
                    .expect("persisted linked repository has a worktree")
                    .checkout(progress, &should_interrupt)
            } else {
                prepared
                    .checkout(progress, &should_interrupt)
                    .map(|(_, outcome)| outcome)
            };
            let err = result.expect_err("interruption during checkout must be reported");
            assert!(
                matches!(
                    err.downcast_any_ref::<gix::worktree::add::Error>(),
                    Some(gix::worktree::add::Error::Interrupted)
                ),
                "the checkout reports cancellation rather than persisting an incomplete index"
            );
            assert!(!index_path.exists(), "an interrupted checkout never writes its index");
            assert_eq!(
                path.exists(),
                persisted,
                "only unpersisted checkout roots are rolled back"
            );
            assert_eq!(
                git_dir.exists(),
                persisted,
                "persisted registration survives cancellation"
            );
        }
        Ok(())
    }

    #[test]
    fn later_checkout_refuses_existing_data_and_retains_persisted_registration() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let path = destinations.path().join("persisted");
        let linked = repo
            .prepare_add_worktree(
                &path,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                &AtomicBool::default(),
            )?
            .persist()?;
        let keep = path.join("keep");
        std::fs::write(&keep, b"user data")?;
        linked
            .worktree()
            .expect("linked repository has a worktree")
            .checkout(gix::progress::Discard, &AtomicBool::default())
            .expect_err("existing data is not an initial checkout destination");
        assert_eq!(std::fs::read(&keep)?, b"user data", "checkout must preserve user files");
        assert!(
            linked.git_dir().is_dir(),
            "failed persisted checkout keeps administration"
        );
        std::fs::remove_file(keep)?;
        linked
            .worktree()
            .expect("linked repository has a worktree")
            .checkout(gix::progress::Discard, &AtomicBool::new(true))
            .expect_err("cancellation retains persistence");
        assert!(
            linked.git_dir().is_dir(),
            "cancelled persisted checkout keeps administration"
        );
        linked
            .worktree()
            .expect("linked repository has a worktree")
            .checkout(gix::progress::Discard, &AtomicBool::default())?;
        linked
            .worktree()
            .expect("linked repository has a worktree")
            .checkout(gix::progress::Discard, &AtomicBool::default())
            .expect_err("initial checkout does not overwrite an existing index");
        Ok(())
    }

    #[test]
    fn later_checkout_refuses_the_main_worktree() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let err = repo
            .worktree()
            .expect("source repository has a main worktree")
            .checkout(gix::progress::Discard, &AtomicBool::default())
            .expect_err("initial checkout is restricted to linked worktrees");
        assert!(
            err.to_string().contains("Initial checkout requires a linked worktree"),
            "the main worktree is rejected before inspecting its index or contents"
        );
        Ok(())
    }

    #[test]
    fn later_checkout_refuses_a_foreign_link() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let path = destinations.path().join("foreign-link");
        let linked = repo
            .prepare_add_worktree(
                &path,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                &AtomicBool::default(),
            )?
            .persist()?;
        let foreign = format!("gitdir: {}\n", repo.git_dir().display());
        std::fs::write(path.join(".git"), &foreign)?;
        linked
            .worktree()
            .expect("linked repository has a worktree")
            .checkout(gix::progress::Discard, &AtomicBool::default())
            .expect_err("a foreign repository link cannot be overwritten");
        assert_eq!(
            std::fs::read_to_string(path.join(".git"))?,
            foreign,
            "foreign link is preserved"
        );
        assert!(!linked.index_path().exists(), "refusal writes no index");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn later_checkout_refuses_symlink_destinations_and_links() -> gix_testtools::TestResult {
        for link_only in [false, true] {
            let (repo, _fixture) = crate::basic_rw_repo()?;
            let destinations = gix_testtools::tempfile::TempDir::new()?;
            let path = destinations.path().join("symlink");
            let linked = repo
                .prepare_add_worktree(
                    &path,
                    gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                    &AtomicBool::default(),
                )?
                .persist()?;
            let target = destinations.path().join("target");
            let symlink = if link_only {
                std::fs::rename(path.join(".git"), &target)?;
                path.join(".git")
            } else {
                std::fs::rename(&path, &target)?;
                path.clone()
            };
            std::os::unix::fs::symlink(&target, &symlink)?;
            linked
                .worktree()
                .expect("linked repository has a worktree")
                .checkout(gix::progress::Discard, &AtomicBool::default())
                .expect_err("initial checkout cannot follow a destination or link symlink");
            assert_eq!(std::fs::read_link(&symlink)?, target, "symlink is preserved");
            assert!(!linked.index_path().exists(), "refusal writes no index");
        }
        Ok(())
    }

    use std::sync::atomic::AtomicBool;

    use gix::bstr::ByteSlice;
    use gix::refs::{FullName, transaction::PreviousValue};
    use gix_path::{into_bstr, to_unix_separators_on_windows};

    #[test]
    fn initial_head_reflog_respects_configuration_like_git() -> gix_testtools::TestResult {
        for (log_all_ref_updates, worktree_config) in [
            (None, false),
            (Some("true"), false),
            (Some("false"), false),
            (Some("always"), false),
            (Some("true"), true),
            (Some("false"), true),
        ] {
            let (mut source, _fixture) = crate::basic_rw_repo()?;
            let work_dir = source.workdir().expect("source checkout").to_owned();
            let commit_id = source.head_id()?.detach();
            let topic = branch("topic");
            source.reference(topic.clone(), commit_id, PreviousValue::MustNotExist, "create topic")?;
            let logging_enabled = log_all_ref_updates != Some("false");
            let mut config = source.config_file_mut(source.common_dir().join("config"))?;
            if worktree_config {
                config.set_raw_value("extensions.worktreeConfig", "true")?;
                config.set_raw_value("core.logAllRefUpdates", if logging_enabled { "false" } else { "true" })?;
                config.commit()?;
                config = source.config_file_mut(source.git_dir().join("config.worktree"))?;
            }
            if let Some(value) = log_all_ref_updates {
                config.set_raw_value("core.logAllRefUpdates", value)?;
            } else {
                config.raw_values_mut("core.logAllRefUpdates")?.delete_all();
            }
            config.commit()?;
            source.reload()?;

            for attached in [true, false] {
                gix_testtools::git(
                    &work_dir,
                    if attached {
                        "worktree add git-created topic"
                    } else {
                        "worktree add --detach git-created HEAD"
                    },
                )?;
                let git_log_path = source.common_dir().join("worktrees/git-created/logs/HEAD");
                assert_eq!(
                    git_log_path.is_file(),
                    logging_enabled,
                    "Git honors core.logAllRefUpdates={log_all_ref_updates:?} with worktreeConfig={worktree_config}"
                );
                let git_log = logging_enabled.then(|| std::fs::read(git_log_path)).transpose()?;
                gix_testtools::git(&work_dir, "worktree remove git-created")?;
                let source_log = std::fs::read(source.git_dir().join("logs/HEAD"))?;
                let branch_log = std::fs::read(source.common_dir().join("logs/refs/heads/topic"))?;
                let destination = work_dir.join(if attached { "gix-attached" } else { "gix-detached" });
                let (created, _) = source.add_worktree(
                    &destination,
                    if attached {
                        gix::worktree::add::Head::Attached(topic.clone())
                    } else {
                        gix::worktree::add::Head::Detached(commit_id)
                    },
                    gix::progress::Discard,
                    &AtomicBool::default(),
                )?;
                assert_eq!(
                    created.head_id()?,
                    commit_id,
                    "initialization preserves the target commit"
                );
                assert_eq!(
                    created.head_name()?,
                    attached.then_some(topic.clone()),
                    "HEAD retains its attachment"
                );
                let head = created.find_reference("HEAD")?;
                assert_eq!(
                    head.log_exists()?,
                    logging_enabled,
                    "new HEAD logging matches Git for attached={attached}, core.logAllRefUpdates={log_all_ref_updates:?}, worktreeConfig={worktree_config}"
                );
                if let Some(git_log) = git_log {
                    let actual_log = std::fs::read(created.git_dir().join("logs/HEAD"))?;
                    let initial = gix_ref::file::log::LineRef::from_bytes(&actual_log)?;
                    let git_initial = gix_ref::file::log::LineRef::from_bytes(&git_log)?;
                    assert_eq!(
                        initial.previous_oid(),
                        source.object_hash().null(),
                        "initial HEAD has no predecessor"
                    );
                    assert_eq!(
                        initial.new_oid(),
                        commit_id,
                        "the initial entry records the checked-out commit"
                    );
                    assert_eq!(
                        (initial.previous_oid, initial.new_oid, initial.message),
                        (git_initial.previous_oid, git_initial.new_oid, git_initial.message),
                        "the initial reflog transition matches Git"
                    );
                    assert_eq!(
                        initial.signature.name, "gitoxide",
                        "the configured committer writes the reflog"
                    );
                    assert_eq!(
                        gix_testtools::git(&destination, "rev-parse --verify 'HEAD@{0}'")?.trim(),
                        commit_id.to_string(),
                        "Git can resolve the newly initialized HEAD reflog"
                    );
                }
                assert_eq!(
                    std::fs::read(source.git_dir().join("logs/HEAD"))?,
                    source_log,
                    "the source HEAD reflog is unchanged"
                );
                assert_eq!(
                    std::fs::read(source.common_dir().join("logs/refs/heads/topic"))?,
                    branch_log,
                    "initializing HEAD does not update the branch reflog"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn bisect_and_rebase_reserve_branches_in_main_and_linked_worktrees() -> gix_testtools::TestResult {
        for operation in ["bisect", "rebase-merge", "rebase-apply", "rebase-interactive"] {
            for occupied in ["main", "linked"] {
                let fixture = gix_testtools::scripted_fixture_writable_with_args(
                    "make_worktree_reserved_branches.sh",
                    [operation, occupied],
                    gix_testtools::Creation::Execute,
                )?;
                let occupied_path = fixture.path().join(occupied);
                let branch_name = if occupied == "main" { "main" } else { "topic" };
                let name = branch(branch_name);
                let occupied_repo = gix::open_opts(&occupied_path, crate::restricted())?;
                assert!(
                    occupied_repo.head()?.is_detached(),
                    "{operation} temporarily detaches HEAD"
                );

                for caller in ["main", "linked"] {
                    let mut repo = gix::open_opts(fixture.path().join(caller), crate::restricted())?;
                    let destination = fixture.path().join("rejected");
                    let git_error = gix_testtools::git(
                        repo.workdir().expect("fixture has a checkout"),
                        &format!("worktree add ../rejected {branch_name}"),
                    )
                    .expect_err("Git reserves the original branch during the operation");
                    assert!(
                        git_error.to_string().contains("already"),
                        "Git rejects branch occupancy: {git_error}"
                    );
                    let err = repo
                        .add_worktree(
                            &destination,
                            gix::worktree::add::Head::Attached(name.clone()),
                            gix::progress::Discard,
                            &AtomicBool::default(),
                        )
                        .expect_err("the operation's original branch remains reserved");
                    match err
                        .downcast_any_ref::<gix::worktree::add::Error>()
                        .expect("occupancy errors retain the branch and worktree paths")
                    {
                        gix::worktree::add::Error::CheckedOut {
                            name: actual,
                            worktree_dirs,
                        } => {
                            assert_eq!(actual, &name);
                            assert_eq!(worktree_dirs.len(), 1, "each occupied worktree is reported once");
                            assert_eq!(
                                worktree_dirs[0].canonicalize()?,
                                occupied_path.canonicalize()?,
                                "the occupied directory is the same even when Windows uses a short path"
                            );
                        }
                        err => panic!("{operation} in {occupied}, called from {caller}: {err:?}"),
                    }
                    assert!(
                        !destination.exists(),
                        "occupancy is checked before creating the destination"
                    );
                    assert!(
                        repo.delete_local_branches([name.clone()])
                            .expect_err("the occupied branch cannot be deleted")
                            .downcast_any_ref::<gix::repository::branch::delete::CheckedOutError>()
                            .is_some(),
                        "the shared occupancy check protects branch deletion as well"
                    );
                }

                let repo = gix::open_opts(fixture.path().join("main"), crate::restricted())?;
                repo.add_worktree(
                    fixture.path().join("allowed"),
                    gix::worktree::add::Head::Attached(branch("available")),
                    gix::progress::Discard,
                    &AtomicBool::default(),
                )?;
            }
        }
        Ok(())
    }

    #[test]
    fn mailbox_application_does_not_reserve_a_rebase_branch() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        std::fs::write(repo.git_dir().join("HEAD"), format!("{}\n", repo.head_id()?))?;
        let state = repo.git_dir().join("rebase-apply");
        std::fs::create_dir(&state)?;
        std::fs::write(state.join("applying"), b"")?;
        std::fs::write(state.join("head-name"), b"refs/heads/main\n")?;

        repo.add_worktree(
            destinations.path().join("allowed"),
            gix::worktree::add::Head::Attached(branch("main")),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        Ok(())
    }

    #[test]
    fn sparse_checkout_directory_patterns_include_gitlinks_on_every_host() -> gix_testtools::TestResult {
        let fixture = gix_testtools::scripted_fixture_writable_with_args(
            "make_worktree_add_repos.sh",
            ["sparse-gitlink"],
            gix_testtools::Creation::Execute,
        )?;
        let source = gix::open_opts(fixture.path().join("main"), crate::restricted())?;
        let (created, _) = source.add_worktree(
            fixture.path().join("gix-created"),
            gix::worktree::add::Head::Detached(source.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        sparse_checkout_matches_git(&created, &fixture.path().join("git-created"))
    }

    #[test]
    fn sparse_checkout_inherits_main_and_linked_patterns_like_git() -> gix_testtools::TestResult {
        for mode in ["cone", "non-cone"] {
            let fixture = gix_testtools::scripted_fixture_writable_with_args(
                "make_worktree_add_repos.sh",
                ["sparse", mode, "inherit"],
                gix_testtools::Creation::Execute,
            )?;
            for name in ["main", "linked"] {
                let source = gix::open_opts(fixture.path().join(name), crate::restricted())?;
                assert_eq!(
                    source.index()?.is_sparse(),
                    mode == "cone",
                    "cone mode exercises a sparse source index"
                );
                let patterns = std::fs::read(source.git_dir().join("info/sparse-checkout"))?;
                if name == "linked" {
                    assert_ne!(
                        patterns,
                        std::fs::read(source.common_dir().join("info/sparse-checkout"))?,
                        "the linked source selects different paths from the main worktree"
                    );
                }
                let expected = fixture.path().join(format!("git-{name}"));
                let (created, _) = source.add_worktree(
                    fixture.path().join(format!("gix-{name}")),
                    gix::worktree::add::Head::Detached(source.find_reference("target")?.peel_to_commit()?.id),
                    gix::progress::Discard,
                    &AtomicBool::default(),
                )?;
                assert_eq!(
                    std::fs::read(created.git_dir().join("info/sparse-checkout"))?,
                    patterns,
                    "the new worktree inherits the {name} source's patterns"
                );
                sparse_checkout_matches_git(&created, &expected)?;
            }
        }
        Ok(())
    }

    #[test]
    fn sparse_checkout_preparation_copies_patterns_and_deferred_checkout_reads_destination() -> gix_testtools::TestResult
    {
        let fixture = gix_testtools::scripted_fixture_writable_with_args(
            "make_worktree_add_repos.sh",
            ["sparse", "cone", "deferred"],
            gix_testtools::Creation::Execute,
        )?;
        let source = gix::open_opts(fixture.path().join("main"), crate::restricted())?;
        let source_patterns = std::fs::read(source.git_dir().join("info/sparse-checkout"))?;
        let prepared = source.prepare_add_worktree(
            fixture.path().join("deferred"),
            gix::worktree::add::Head::Detached(source.find_reference("target")?.peel_to_commit()?.id),
            &AtomicBool::default(),
        )?;
        assert_eq!(
            std::fs::read(prepared.repo().git_dir().join("info/sparse-checkout"))?,
            source_patterns,
            "preparation copies patterns even without checkout"
        );
        let created = prepared.persist()?;
        assert!(!created.index_path().exists(), "persistence does not create the index");
        let expected = fixture.path().join("git-deferred");
        let expected_repo = gix::open_opts(&expected, crate::restricted())?;
        // Intentionally alter the sparse-checkout patterns to ensure these aren't stale.
        std::fs::copy(
            expected_repo.git_dir().join("info/sparse-checkout"),
            created.git_dir().join("info/sparse-checkout"),
        )?;
        created
            .worktree()
            .expect("persisted registration has a worktree")
            .checkout(gix::progress::Discard, &AtomicBool::default())?;
        sparse_checkout_matches_git(&created, &expected)?;
        assert_eq!(
            std::fs::read(source.git_dir().join("info/sparse-checkout"))?,
            source_patterns,
            "changing destination patterns leaves the source unchanged"
        );
        Ok(())
    }

    #[test]
    fn sparse_checkout_empty_missing_disabled_and_custom_cone_patterns_match_git() -> gix_testtools::TestResult {
        for (cone, state) in [
            (false, "missing"),
            (true, "missing"),
            (false, "empty"),
            (true, "empty"),
            (false, "disabled"),
            (true, "directory"),
            (true, "invalid-cone"),
        ] {
            let fixture = gix_testtools::scripted_fixture_writable_with_args(
                "make_worktree_add_repos.sh",
                ["sparse", if cone { "cone" } else { "non-cone" }, state],
                gix_testtools::Creation::Execute,
            )?;
            let source = gix::open_opts(fixture.path().join("main"), crate::restricted())?;
            let expected = fixture.path().join("git-created");
            let (created, _) = source.add_worktree(
                fixture.path().join("gix-created"),
                gix::worktree::add::Head::Detached(source.find_reference("target")?.peel_to_commit()?.id),
                gix::progress::Discard,
                &AtomicBool::default(),
            )?;
            assert_eq!(
                created.git_dir().join("info/sparse-checkout").exists(),
                state != "missing" && state != "disabled",
                "only an active, present patterns file is copied (cone={cone}, state={state})"
            );
            sparse_checkout_matches_git(&created, &expected)?;
        }
        Ok(())
    }

    #[test]
    fn sparse_checkout_pattern_copy_errors_roll_back() -> gix_testtools::TestResult {
        let fixture = gix_testtools::scripted_fixture_writable_with_args(
            "make_worktree_add_repos.sh",
            ["sparse", "non-cone", "unreadable"],
            gix_testtools::Creation::Execute,
        )?;
        let source = gix::open_opts(fixture.path().join("main"), crate::restricted())?;
        let patterns = source.git_dir().join("info/sparse-checkout");
        let destination = fixture.path().join("rejected");
        source
            .prepare_add_worktree(
                &destination,
                gix::worktree::add::Head::Detached(source.head_id()?.detach()),
                &AtomicBool::default(),
            )
            .expect_err("an unreadable active patterns file must fail preparation");
        assert!(!destination.exists(), "copy failure removes the checkout directory");
        assert!(source.worktrees()?.is_empty(), "copy failure removes the registration");
        assert!(patterns.is_dir(), "copy failure leaves the source untouched");
        Ok(())
    }

    fn sparse_checkout_matches_git(created: &gix::Repository, expected: &std::path::Path) -> gix_testtools::TestResult {
        use std::fmt::Write;

        let index = created.index()?;
        assert!(
            !index.is_sparse(),
            "sparse checkout can retain a full index while we can't do better"
        );
        let work_dir = created.workdir().expect("new checkout");
        let mut listing = String::new();
        for entry in index.entries() {
            let skip = entry.flags.contains(gix::index::entry::Flags::SKIP_WORKTREE);
            let path = entry.path(&index);
            writeln!(listing, "{} {}", if skip { 'S' } else { 'H' }, path)?;
            let relative_path = gix_path::from_bstr(path)?;
            assert_eq!(
                work_dir.join(&relative_path).exists(),
                !skip,
                "persisted flags agree with materialized files at {path}"
            );
            assert_eq!(
                work_dir.join(&relative_path).exists(),
                expected.join(&relative_path).exists(),
                "Git and gix materialize the same paths at {path}"
            );
            if !skip && entry.mode.is_submodule() {
                assert!(
                    work_dir.join(&relative_path).is_dir(),
                    "included gitlinks materialize as directories at {path}"
                );
            } else if !skip {
                assert_eq!(
                    std::fs::read(work_dir.join(&relative_path))?,
                    std::fs::read(expected.join(&relative_path))?,
                    "Git and gix write identical contents at {path}"
                );
            }
        }
        assert_eq!(
            listing,
            std::fs::read_to_string(expected.with_extension("ls-files.baseline"))?,
            "persisted skip-worktree flags match Git"
        );
        let items = created
            .status(gix::progress::Discard)?
            .into_iter(None)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert!(items.is_empty(), "gix recognizes a clean sparse checkout: {items:?}");
        // Git status may rewrite the full index as sparse when index.sparse is enabled.
        assert_eq!(
            gix_testtools::git(work_dir, "status --porcelain")?,
            "",
            "Git recognizes a clean sparse checkout"
        );
        Ok(())
    }

    #[test]
    fn worktree_config_is_inherited_before_checkout() -> gix_testtools::TestResult {
        // MSYS sed's text mode would discard CRLF produced before smudge filtering.
        let sed = if cfg!(windows) { "sed -b" } else { "sed" };
        let fixture = gix_testtools::scripted_fixture_writable_with_args(
            "make_worktree_add_repos.sh",
            ["config-inheritance"],
            gix_testtools::Creation::CopyFromReadOnly,
        )?;
        let mut source = gix::open_opts(fixture.path(), crate::restricted())?;
        let mut config = source.config_file_mut(source.common_dir().join("config"))?;
        config.set_raw_value("extensions.worktreeConfig", "true")?;
        config.set_raw_value("core.autocrlf", "false")?;
        config.commit()?;

        for (name, autocrlf, expected_eol, expected_filter) in [
            ("main", "true", b"hello\r\n".as_slice(), b"main\r\n".as_slice()),
            ("linked", "false", b"hello\n".as_slice(), b"linked\n".as_slice()),
        ] {
            let work_dir = source.workdir().expect("source checkout").to_owned();
            let config_path = source.git_dir().join("config.worktree");
            let mut config = source.config_file_mut(&config_path)?;
            config.set_raw_value("core.autocrlf", autocrlf)?;
            config.set_raw_value("core.bare", "false")?;
            config.set_raw_value("core.worktree", to_unix_separators_on_windows(into_bstr(&work_dir)?))?;
            config.set_raw_value("filter.inherit.smudge", format!("{sed} s/hello/{name}/"))?;
            config.set_raw_value("filter.inherit.clean", format!("{sed} s/{name}/hello/"))?;
            config.set_raw_value("filter.inherit.required", "true")?;
            config.commit()?;
            source.reload()?;
            let original_config = std::fs::read(&config_path)?;

            gix_testtools::git(&work_dir, "worktree add --detach git-created HEAD")?;
            let (created, _) = source.add_worktree(
                work_dir.join("gix-created"),
                gix::worktree::add::Head::Detached(source.head_id()?.detach()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )?;
            for (file, expected) in [("this", expected_eol), ("filtered", expected_filter)] {
                let git_contents = std::fs::read(work_dir.join("git-created").join(file))?;
                assert_eq!(git_contents, expected, "Git applies {name}'s worktree-local settings");
                assert_eq!(
                    std::fs::read(created.workdir().expect("new checkout").join(file))?,
                    git_contents,
                    "checkout inherits {name}'s line endings and filter commands before writing {file}"
                );
            }
            let copied = gix_config::File::from_path_no_includes(
                created.git_dir().join("config.worktree"),
                gix_config::Source::Worktree,
            )?;
            assert!(
                copied.raw_value("core.worktree").is_err(),
                "the source checkout path is excluded"
            );
            assert_eq!(
                copied.boolean("core.bare")?,
                Some(false),
                "an explicit non-bare setting is retained"
            );
            assert_eq!(
                std::fs::read(&config_path)?,
                original_config,
                "copying configuration leaves the source file unchanged"
            );
            assert_eq!(
                gix_testtools::git(created.workdir().expect("new checkout"), "status --porcelain")?,
                "",
                "Git agrees with the checkout and index after applying inherited filters"
            );
            source = created;
        }
        Ok(())
    }

    #[test]
    fn common_attributes_are_applied_during_checkout() -> gix_testtools::TestResult {
        // Preserve the checkout's line endings when the filter runs under MSYS.
        let sed = if cfg!(windows) { "sed -b" } else { "sed" };
        let (mut source, _fixture) = crate::basic_rw_repo()?;
        let mut config = source.config_file_mut(source.common_dir().join("config"))?;
        config.set_raw_value("core.autocrlf", "false")?;
        config.set_raw_value("filter.shared.smudge", format!("{sed} s/hello/shared/"))?;
        config.set_raw_value("filter.shared.clean", format!("{sed} s/shared/hello/"))?;
        config.set_raw_value("filter.shared.required", "true")?;
        config.commit()?;
        source.reload()?;
        std::fs::write(
            source.common_dir().join("info/attributes"),
            b"this text eol=crlf filter=shared\n",
        )?;

        for caller in ["main", "linked"] {
            let work_dir = source.workdir().expect("source checkout").to_owned();
            gix_testtools::git(&work_dir, "worktree add --detach git-created HEAD")?;
            let (created, _) = source.add_worktree(
                work_dir.join("gix-created"),
                gix::worktree::add::Head::Detached(source.head_id()?.detach()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )?;
            let git_contents = std::fs::read(work_dir.join("git-created/this"))?;
            assert_eq!(
                git_contents, b"shared\r\n",
                "Git applies the common attributes for CRLF conversion and smudge filtering"
            );
            assert_eq!(
                std::fs::read(created.workdir().expect("new checkout").join("this"))?,
                git_contents,
                "checkout from the {caller} worktree uses the common info/attributes like Git"
            );
            assert_eq!(
                gix_testtools::git(created.workdir().expect("new checkout"), "status --porcelain")?,
                "",
                "Git agrees with the checkout and index after applying the common attributes"
            );
            source = created;
        }
        Ok(())
    }

    #[test]
    fn worktree_config_is_optional_but_copy_errors_roll_back() -> gix_testtools::TestResult {
        let (mut repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let source_config = repo.git_dir().join("config.worktree");
        std::fs::write(&source_config, b"[core]\n\tautocrlf = true\n")?;
        let commit_id = repo.head_id()?.detach();
        let (disabled, _) = repo.add_worktree(
            destinations.path().join("disabled"),
            gix::worktree::add::Head::Detached(commit_id),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        assert!(
            !disabled.git_dir().join("config.worktree").exists(),
            "a dormant config.worktree file is not copied"
        );

        let mut config = repo.config_file_mut(repo.common_dir().join("config"))?;
        config.set_raw_value("extensions.worktreeConfig", "true")?;
        config.commit()?;
        std::fs::remove_file(source_config)?;
        repo.reload()?;
        let (linked, _) = repo.add_worktree(
            destinations.path().join("missing"),
            gix::worktree::add::Head::Detached(commit_id),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        assert!(
            !linked.git_dir().join("config.worktree").exists(),
            "an enabled extension does not require a worktree configuration file"
        );

        std::fs::write(linked.git_dir().join("config.worktree"), b"[invalid\n")?;
        let destination = destinations.path().join("rejected");
        let err = linked
            .add_worktree(
                &destination,
                gix::worktree::add::Head::Detached(commit_id),
                gix::progress::Discard,
                &AtomicBool::default(),
            )
            .expect_err("invalid source configuration must not be silently dropped");
        assert!(
            err.to_string()
                .contains("Could not read the source worktree configuration"),
            "parse failures identify the source worktree configuration"
        );
        assert!(
            !destination.exists(),
            "a copy failure removes the new checkout directory"
        );
        assert_eq!(
            repo.worktrees()?.len(),
            2,
            "a copy failure removes the private Git directory"
        );
        Ok(())
    }

    #[test]
    fn relative_links_survive_moving_the_repository_and_worktrees_together() -> gix_testtools::TestResult {
        use std::fs;

        let (repo, _fixture) = crate::basic_rw_repo()?;
        let tmp = gix_testtools::tempfile::TempDir::new()?;
        let group = tmp.path().join("group");
        fs::create_dir(&group)?;
        let main_path = repo.workdir().expect("non-bare fixture").to_owned();
        drop(repo);
        fs::rename(main_path, group.join("main"))?;
        let mut repo = gix::open_opts(group.join("main"), crate::restricted())?;
        let mut config = repo.config_file_mut(repo.common_dir().join("config"))?;
        config.set_raw_value("worktree.useRelativePaths", "true")?;
        config.commit()?;
        repo.reload()?;
        let commit_id = repo.head_id()?.detach();
        let (linked, _) = repo.add_worktree(
            group.join("linked"),
            gix::worktree::add::Head::Detached(commit_id),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        assert_eq!(
            fs::read_to_string(linked.git_dir().join("gitdir"))?,
            "../../../../linked/.git\n",
            "the backlink is relative to the private Git directory"
        );
        assert_eq!(
            fs::read_to_string(group.join("linked/.git"))?,
            "gitdir: ../main/.git/worktrees/linked\n",
            "the forward link is relative to the worktree"
        );
        assert_eq!(
            linked.config_snapshot().integer("core.repositoryFormatVersion"),
            Some(1)
        );
        assert_eq!(
            linked.config_snapshot().boolean("extensions.relativeWorktrees"),
            Some(true)
        );

        // Adding from a linked worktree must update/use the shared configuration too.
        linked.add_worktree(
            group.join("another"),
            gix::worktree::add::Head::Detached(commit_id),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        drop(linked);
        drop(repo);
        let moved = tmp.path().join("moved");
        fs::rename(&group, &moved)?;
        let repo = gix::open_opts(moved.join("main"), crate::restricted())?;
        assert_eq!(
            repo.worktrees()?.len(),
            2,
            "both worktrees remain registered after moving"
        );
        for proxy in repo.worktrees()? {
            assert!(
                !proxy.is_prunable(),
                "relative backlinks still locate the moved worktrees"
            );
            let linked = proxy.into_repo()?;
            assert_eq!(linked.head_id()?.detach(), commit_id);
            assert_eq!(
                gix::open(linked.workdir().expect("linked checkout"))?
                    .head_id()?
                    .detach(),
                commit_id
            );
            // Container CI can have older Git too; always exercise gix above.
            if *gix_testtools::GIT_VERSION >= (2, 48, 0) {
                assert_eq!(
                    gix_testtools::git(linked.workdir().expect("linked checkout"), "status --porcelain")?,
                    "",
                    "Git can use the moved checkout and index without repair"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn relative_worktrees_extension_does_not_enable_relative_links_by_itself() -> gix_testtools::TestResult {
        let (mut repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let mut config = repo.config_file_mut(repo.common_dir().join("config"))?;
        config.set_raw_value("core.repositoryFormatVersion", "1")?;
        config.set_raw_value("extensions.relativeWorktrees", "true")?;
        config.commit()?;
        repo.reload()?;
        for (name, setting) in [("unset", None), ("false", Some("false"))] {
            if let Some(setting) = setting {
                repo.config_snapshot_mut()
                    .set_raw_value("worktree.useRelativePaths", setting)?;
            }
            let (linked, _) = repo.add_worktree(
                destinations.path().join(name),
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )?;
            assert_eq!(
                std::fs::read_to_string(linked.git_dir().join("gitdir"))?,
                format!(
                    "{}\n",
                    to_unix_separators_on_windows(into_bstr(linked.workdir().expect("linked checkout").join(".git"))?)
                ),
                "the extension records compatibility; the worktree setting controls new links"
            );
        }
        Ok(())
    }

    #[test]
    fn invalid_repository_format_versions_reject_relative_worktrees() -> gix_testtools::TestResult {
        use gix::config::tree::Core;
        use gix_error::MetadataValue;

        let (mut repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let destination = destinations.path().join("rejected");
        let commit_id = repo.head_id()?.detach();
        repo.config_snapshot_mut()
            .set_raw_value("worktree.useRelativePaths", "true")?;
        let config_path = repo.common_dir().join("config");
        for value in ["-1", "2", "bogus"] {
            // Simulate a disk change after opening so `add_worktree()` must validate the version itself.
            let mut config = repo.config_file_mut(&config_path)?;
            config.set_raw_value(Core::REPOSITORY_FORMAT_VERSION, value)?;
            config.commit()?;
            let before = std::fs::read(&config_path)?;
            let err = repo
                .add_worktree(
                    &destination,
                    gix::worktree::add::Head::Detached(commit_id),
                    gix::progress::Discard,
                    &AtomicBool::default(),
                )
                .expect_err("relative worktrees require a supported repository format version");
            assert_eq!(
                err.is_unsupported(),
                value == "2",
                "future formats require another implementation: {err:?}"
            );
            assert_eq!(
                err.is_validation(),
                value != "2",
                "negative or malformed versions are invalid input: {err:?}"
            );
            let metadata = err
                .metadata()
                .find(|metadata| metadata.contains_key("key"))
                .expect("the format-version key provides the validation error");
            assert_eq!(
                metadata.get("key"),
                Some(&MetadataValue::from("core.repositoryFormatVersion")),
                "validation identifies the invalid configuration key"
            );
            assert!(!destination.exists(), "invalid configuration rolls back the worktree");
            assert!(
                repo.worktrees()?.is_empty(),
                "invalid configuration rolls back the private Git directory"
            );
            assert_eq!(
                std::fs::read(&config_path)?,
                before,
                "an unsupported format is not upgraded or otherwise rewritten"
            );
        }
        Ok(())
    }

    #[test]
    fn failed_relative_config_update_rolls_back_worktree_addition() -> gix_testtools::TestResult {
        for (scenario, lock_config) in [
            ("relative-config-lock", true),
            ("relative-config-unknown-extension", false),
        ] {
            let fixture = gix_testtools::scripted_fixture_writable_with_args(
                "make_worktree_add_repos.sh",
                [scenario],
                gix_testtools::Creation::CopyFromReadOnly,
            )?;
            let mut repo = gix::open_opts(fixture.path(), crate::restricted())?;
            let destinations = gix_testtools::tempfile::TempDir::new()?;
            let config_path = repo.common_dir().join("config");
            let before = std::fs::read(&config_path)?;
            repo.config_snapshot_mut()
                .set_raw_value("worktree.useRelativePaths", "true")?;
            repo.config_snapshot_mut()
                .set_raw_value("core.configLockTimeout", "0")?;
            let lock = lock_config.then(|| repo.config_file_mut(&config_path)).transpose()?;
            let destination = destinations.path().join("rejected");
            let err = repo
                .add_worktree(
                    &destination,
                    gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                    gix::progress::Discard,
                    &AtomicBool::default(),
                )
                .expect_err("the shared config cannot safely be updated");
            if lock_config {
                assert!(
                    err.to_string()
                        .contains("Could not enable relative worktrees in the shared configuration"),
                    "the config lock failure is reported: {err:?}"
                );
            } else {
                assert!(
                    err.is_unsupported(),
                    "the format version rejects the extension: {err:?}"
                );
                assert_eq!(
                    err.metadata().find_map(|metadata| metadata.get("extension")),
                    Some(&gix_error::MetadataValue::from("futureExtension")),
                    "the centralized validation identifies the unsupported extension"
                );
            }
            assert!(!destination.exists(), "failed configuration removes the new worktree");
            assert!(
                repo.worktrees()?.is_empty(),
                "failed configuration removes the private Git directory"
            );
            assert_eq!(
                std::fs::read(&config_path)?,
                before,
                "the original config remains intact"
            );
            drop(lock);
        }
        Ok(())
    }

    #[test]
    fn attached_and_detached_worktrees_are_checked_out_and_recognized_by_git() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let commit_id = repo.head_id()?.detach();
        let topic = branch("topic");
        repo.reference(
            topic.clone(),
            commit_id,
            PreviousValue::MustNotExist,
            "create worktree test branch",
        )?;

        let attached_path = destinations.path().join("attached");
        let (attached, attached_outcome) = repo.add_worktree(
            &attached_path,
            gix::worktree::add::Head::Attached(topic.clone()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        assert_eq!(attached.head_name()?, Some(topic));
        assert_eq!(attached.head_id()?.detach(), commit_id);
        assert!(attached_outcome.files_updated > 0, "the target tree was checked out");
        assert!(attached_path.join("this").is_file(), "tracked files are present");
        assert!(!attached.index()?.entries().is_empty(), "the linked index was written");
        assert_eq!(
            gix_testtools::git(&attached_path, "status --porcelain")?,
            "",
            "the checkout and its index agree"
        );

        let detached_path = destinations.path().join("detached");
        let (detached, _) = repo.add_worktree(
            &detached_path,
            gix::worktree::add::Head::Detached(commit_id),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        assert_eq!(detached.head_name()?, None);
        assert_eq!(detached.head_id()?.detach(), commit_id);

        let listing = gix_testtools::git(repo.workdir().expect("non-bare fixture"), "worktree list --porcelain")?;
        assert!(
            listing.contains(to_unix_separators_on_windows(into_bstr(&attached_path)?).to_str()?),
            "Git recognizes the attached worktree"
        );
        assert!(
            listing.contains(to_unix_separators_on_windows(into_bstr(&detached_path)?).to_str()?),
            "Git recognizes the detached worktree"
        );
        Ok(())
    }

    #[test]
    fn linked_worktree_cannot_check_out_a_branch_occupied_in_any_worktree() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        assert_eq!(
            repo.worktrees_including_main()?.collect::<gix::Result<Vec<_>>>()?,
            vec![repo.clone()],
            "the main repository is listed even without linked worktrees"
        );
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let topic = branch("topic");
        repo.reference(
            topic.clone(),
            repo.head_id()?,
            PreviousValue::MustNotExist,
            "create worktree test branch",
        )?;
        let (linked, _) = repo.add_worktree(
            destinations.path().join("linked"),
            gix::worktree::add::Head::Attached(topic),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let linked_dir = linked.workdir().expect("linked checkout");
        let destination = destinations.path().join("rejected");

        for (name, worktree_dir) in [
            ("main", repo.workdir().expect("non-bare fixture")),
            ("topic", linked_dir),
        ] {
            let git_err = gix_testtools::git(linked_dir, &format!("worktree add ../rejected {name}"))
                .expect_err("Git rejects a branch already checked out in either worktree");
            assert!(
                git_err.to_string().contains("already"),
                "Git reports that the branch is occupied: {git_err}"
            );
            let expected_dirs = vec![gix_path::realpath(worktree_dir)?];
            let name = branch(name);
            let err = linked
                .add_worktree(
                    &destination,
                    gix::worktree::add::Head::Attached(name.clone()),
                    gix::progress::Discard,
                    &AtomicBool::default(),
                )
                .expect_err("branches checked out in the main or linked worktree are protected");
            assert!(
                matches!(err.downcast_any_ref::<gix::worktree::add::Error>(), Some(gix::worktree::add::Error::CheckedOut { name: actual, worktree_dirs })
                    if actual == &name && worktree_dirs == &expected_dirs),
                "the occupied branch and its single checkout are reported: {err:?}"
            );
            let err = linked
                .clone()
                .delete_local_branches([name.clone()])
                .expect_err("branch deletion uses the same complete worktree listing");
            assert!(
                matches!(err.downcast_any_ref::<gix::repository::branch::delete::CheckedOutError>(), Some(gix::repository::branch::delete::CheckedOutError { name: actual, worktree_dirs })
                    if actual == &name && worktree_dirs == &expected_dirs),
                "branch deletion protects the same single checkout: {err:?}"
            );
            assert!(!destination.exists(), "branch validation leaves the destination absent");
        }
        assert_eq!(repo.worktrees()?.len(), 1, "no extra worktree was registered");
        Ok(())
    }

    #[test]
    fn lock_suffix_only_destination_names_match_git() -> gix_testtools::TestResult {
        let (source, _fixture) = crate::basic_rw_repo()?;
        let work_dir = source.workdir().expect("source checkout");
        gix_testtools::git(work_dir, "worktree add --detach git/.lock.lock HEAD")?;
        let git_work_dir = work_dir.join("git/.lock.lock");
        let git_contents = std::fs::read(git_work_dir.join("this"))?;
        let git_repo = gix::open_opts(&git_work_dir, crate::restricted())?;
        assert_eq!(
            git_repo.git_dir().file_name(),
            Some("-lock".as_ref()),
            "Git rewrites the leading dot before stripping repeated lock suffixes"
        );
        drop(git_repo);
        gix_testtools::git(work_dir, "worktree remove git/.lock.lock")?;

        let destination = work_dir.join("gix/.lock.lock");
        let (created, _) = source.add_worktree(
            &destination,
            gix::worktree::add::Head::Detached(source.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        assert_eq!(
            created.git_dir().file_name(),
            Some("-lock".as_ref()),
            "the private Git directory uses the same non-empty ID as Git"
        );
        assert_eq!(
            std::fs::read(destination.join("this"))?,
            git_contents,
            "checkout succeeds at the requested destination"
        );
        assert_eq!(
            gix_testtools::git(&destination, "status --porcelain")?,
            "",
            "Git agrees with the checkout and index"
        );
        Ok(())
    }

    #[test]
    fn attached_worktrees_ignore_reference_namespaces_like_git() -> gix_testtools::TestResult {
        for (namespace, from_config) in [("foo", false), ("foo/bar", true)] {
            let (mut source, _fixture) = crate::basic_rw_repo()?;
            let work_dir = source.workdir().expect("source checkout").to_owned();
            let commit_id = source.head_id()?.detach();
            let namespaced_commit_id = source
                .head_commit()?
                .parent_ids()
                .next()
                .expect("the fixture has two commits with different trees")
                .detach();
            let topic = branch("topic");
            source.reference(topic.clone(), commit_id, PreviousValue::MustNotExist, "create topic")?;
            source.set_namespace(namespace)?;
            for name in ["main", "topic", "only-namespaced"] {
                source.reference(
                    branch(name),
                    namespaced_commit_id,
                    PreviousValue::MustNotExist,
                    "create namespaced branch",
                )?;
            }
            if from_config {
                let mut config = source.config_file_mut(source.common_dir().join("config"))?;
                config.set_raw_value("gitoxide.core.refsNamespace", namespace)?;
                config.commit()?;
                source.reload()?;
            }

            gix_testtools::git(
                &work_dir,
                &format!("--namespace={namespace} worktree add git-created topic"),
            )?;
            let git_work_dir = work_dir.join("git-created");
            let git_contents = std::fs::read(git_work_dir.join("this"))?;
            assert_eq!(
                gix_testtools::git(&git_work_dir, "rev-parse HEAD")?.trim(),
                commit_id.to_string(),
                "Git selects the default namespace even when the namespaced branch has another commit"
            );
            gix_testtools::git(&work_dir, "worktree remove git-created")?;

            let (created, _) = source.add_worktree(
                work_dir.join("gix-created"),
                gix::worktree::add::Head::Attached(topic.clone()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )?;
            assert_eq!(
                std::fs::read(created.workdir().expect("new checkout").join("this"))?,
                git_contents,
                "the checkout uses the same default-namespace tree as Git"
            );
            assert!(
                created.namespace().is_none(),
                "the returned repository uses its physical HEAD"
            );
            assert_eq!(created.head_id()?, commit_id, "HEAD agrees with the checked-out tree");
            assert_eq!(
                created.head_name()?,
                Some(topic.clone()),
                "HEAD stays on the default branch"
            );
            assert_eq!(
                gix_testtools::git(created.workdir().expect("new checkout"), "status --porcelain")?,
                "",
                "the new worktree has no staged or unstaged changes"
            );
            assert_eq!(
                source.find_reference(topic.as_ref())?.id(),
                namespaced_commit_id,
                "the source repository retains its namespace"
            );

            for name in ["main", "topic", "only-namespaced"] {
                gix_testtools::git(
                    &work_dir,
                    &format!("--namespace={namespace} worktree add rejected {name}"),
                )
                .expect_err("Git rejects occupied branches and branches that only exist in a namespace");
                let err = source
                    .add_worktree(
                        work_dir.join("rejected"),
                        gix::worktree::add::Head::Attached(branch(name)),
                        gix::progress::Discard,
                        &AtomicBool::default(),
                    )
                    .expect_err("validation uses the default namespace like Git");
                assert!(
                    match name {
                        "only-namespaced" => err.to_string().contains("The local branch could not be found"),
                        _ => matches!(
                            err.downcast_any_ref::<gix::worktree::add::Error>(),
                            Some(gix::worktree::add::Error::CheckedOut { .. })
                        ),
                    },
                    "{name}: the default namespace determines whether a branch is missing or occupied: {err:?}"
                );
                assert!(
                    !work_dir.join("rejected").exists(),
                    "validation leaves the destination absent"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn separate_git_dir_preserves_main_worktree_and_its_occupied_branch() -> gix_testtools::TestResult {
        let fixture = gix_testtools::scripted_fixture_writable_with_args(
            "make_worktree_add_repos.sh",
            ["separate-git-dir"],
            gix_testtools::Creation::Execute,
        )?;
        let work_dir = gix_path::realpath(fixture.path().join("main"))?;
        let mut repo = gix::open_opts(&work_dir, crate::restricted())?;
        let commit_id = repo.head_id()?.detach();
        let name = branch("main");
        let destination = fixture.path().join("rejected");

        for command in ["worktree add ../rejected main", "branch -D main"] {
            gix_testtools::git(&work_dir, command)
                .expect_err("Git protects the main worktree's branch with a separate Git directory");
        }
        let err = repo
            .add_worktree(
                &destination,
                gix::worktree::add::Head::Attached(name.clone()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )
            .expect_err("the main worktree's branch cannot be checked out twice");
        let expected_dirs = vec![work_dir.clone()];
        assert!(
            matches!(err.downcast_any_ref::<gix::worktree::add::Error>(), Some(gix::worktree::add::Error::CheckedOut { name: actual, worktree_dirs })
                if actual == &name && worktree_dirs == &expected_dirs),
            "the occupied branch reports its known main worktree: {err:?}"
        );
        let err = repo
            .delete_local_branches([name.clone()])
            .expect_err("the main worktree's checked-out branch cannot be deleted");
        assert!(
            matches!(err.downcast_any_ref::<gix::repository::branch::delete::CheckedOutError>(), Some(gix::repository::branch::delete::CheckedOutError { name: actual, worktree_dirs })
                if actual == &name && worktree_dirs == &expected_dirs),
            "branch deletion protects the same main worktree: {err:?}"
        );
        assert_eq!(repo.head_id()?, commit_id, "the checked-out branch is unchanged");
        assert!(!destination.exists(), "rejection leaves the destination absent");

        let repositories = repo.worktrees_including_main()?.collect::<gix::Result<Vec<_>>>()?;
        assert_eq!(repositories.len(), 1, "only the main repository is registered");
        assert_eq!(
            repositories[0].workdir(),
            Some(work_dir.as_path()),
            "enumeration preserves the worktree known from its .git file"
        );
        assert_eq!(
            repo.main_repo()?.workdir(),
            Some(work_dir.as_path()),
            "requesting the already-open main repository preserves its worktree"
        );
        Ok(())
    }

    #[test]
    fn adds_a_worktree_from_a_bare_parent() -> gix_testtools::TestResult {
        let Some(fixture) = gix_testtools::scripted_fixture_writable_with_args_with_git_version(
            "make_worktree_repo.sh",
            ["bare"],
            gix_testtools::Creation::CopyFromReadOnly,
            |version| version >= (2, 31, 0),
        )?
        else {
            return Ok(());
        };
        let mut repo = gix::open_opts(fixture.path().join("repo.git"), crate::restricted())?;
        assert!(repo.is_bare(), "the parent repository has no main worktree");
        let mut config = repo.config_file_mut(repo.common_dir().join("config"))?;
        config.set_raw_value("extensions.worktreeConfig", "true")?;
        // With worktree config enabled, the bare setting belongs only to the main repository's config.worktree.
        config.raw_values_mut("core.bare")?.delete_all();
        config.commit()?;
        let mut config = repo.config_file_mut(repo.git_dir().join("config.worktree"))?;
        config.set_raw_value("core.bare", "true")?;
        config.set_raw_value("core.autocrlf", "true")?;
        config.commit()?;
        repo.reload()?;
        gix_testtools::git(repo.git_dir(), "worktree add --detach ../git-added-from-bare HEAD")?;
        let destination = fixture.path().join("added-from-bare");
        let main = branch("main");

        let (worktree, _) = repo.add_worktree(
            &destination,
            gix::worktree::add::Head::Attached(main.clone()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;

        assert_eq!(worktree.head_name()?, Some(main));
        assert_eq!(worktree.workdir(), Some(gix_path::realpath(&destination)?.as_path()));
        let copied = gix_config::File::from_path_no_includes(
            worktree.git_dir().join("config.worktree"),
            gix_config::Source::Worktree,
        )?;
        assert_eq!(copied.boolean("core.bare")?, None, "a true bare setting is excluded");
        let git_contents = std::fs::read(fixture.path().join("git-added-from-bare/a"))?;
        assert_eq!(
            git_contents, b"hello\r\n",
            "Git inherits the bare source's line-ending settings"
        );
        assert_eq!(
            std::fs::read(destination.join("a"))?,
            git_contents,
            "checkout from a bare parent applies the same inherited settings as Git"
        );
        assert_eq!(gix_testtools::git(&destination, "status --porcelain")?, "");
        Ok(())
    }

    #[test]
    fn validation_failures_leave_the_destination_absent() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let destination = destinations.path().join("rejected");
        let main = branch("main");

        let err = repo
            .add_worktree(
                &destination,
                gix::worktree::add::Head::Attached(main.clone()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )
            .expect_err("the main branch is already checked out");
        assert!(
            matches!(err.downcast_any_ref::<gix::worktree::add::Error>(), Some(gix::worktree::add::Error::CheckedOut { name, .. }) if name == &main),
            "the checked-out branch is identified"
        );
        assert!(
            err.is_conflict(),
            "branch occupancy requires reconciling worktree state"
        );
        assert!(!destination.exists(), "validation happens before creating files");

        let interrupted = AtomicBool::new(true);
        let err = repo
            .add_worktree(
                &destination,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                gix::progress::Discard,
                &interrupted,
            )
            .expect_err("an already-interrupted operation does no work");
        assert!(matches!(
            err.downcast_any_ref::<gix::worktree::add::Error>(),
            Some(gix::worktree::add::Error::Interrupted)
        ));
        assert!(err.is_cancelled(), "observed cancellation tells callers to stop");
        assert!(!destination.exists(), "interruption leaves no destination behind");

        std::fs::create_dir(&destination)?;
        std::fs::write(destination.join("keep"), b"user data")?;
        let err = repo
            .add_worktree(
                &destination,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )
            .expect_err("a non-empty destination is rejected");
        assert!(
            err.to_string().contains("Could not prepare the linked worktree"),
            "non-empty destinations fail during worktree preparation"
        );
        assert_eq!(
            std::fs::read(destination.join("keep"))?,
            b"user data",
            "a failed addition preserves existing destination contents"
        );
        Ok(())
    }

    #[test]
    fn registered_destinations_are_rejected_even_when_missing_or_empty() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let destination = destinations.path().join("registered");
        let commit_id = repo.head_id()?.detach();
        repo.add_worktree(
            &destination,
            gix::worktree::add::Head::Detached(commit_id),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;

        std::fs::remove_dir_all(&destination)?;
        for exists in [false, true] {
            if exists {
                std::fs::create_dir(&destination)?;
            }
            let err = repo
                .add_worktree(
                    &destination,
                    gix::worktree::add::Head::Detached(commit_id),
                    gix::progress::Discard,
                    &AtomicBool::default(),
                )
                .expect_err("registered destinations cannot be reused");
            assert!(
                matches!(err.downcast_any_ref::<gix::worktree::add::Error>(), Some(gix::worktree::add::Error::DestinationRegistered { destination: actual }) if actual == &destination),
                "the registered destination is identified"
            );
        }

        let case_variant = destination.with_file_name("REGISTERED");
        if case_variant.exists() {
            let err = repo
                .add_worktree(
                    &case_variant,
                    gix::worktree::add::Head::Detached(commit_id),
                    gix::progress::Discard,
                    &AtomicBool::default(),
                )
                .expect_err("filesystem-equivalent casing cannot bypass registration");
            assert!(
                matches!(err.downcast_any_ref::<gix::worktree::add::Error>(), Some(gix::worktree::add::Error::DestinationRegistered { destination }) if destination == &case_variant),
                "the caller's case variant is identified"
            );
        }
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn registered_destinations_are_matched_through_symlinked_parents() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let actual_parent = destinations.path().join("actual");
        let linked_parent = destinations.path().join("linked");
        std::fs::create_dir(&actual_parent)?;
        std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;
        let destination = actual_parent.join("registered");
        let commit_id = repo.head_id()?.detach();
        repo.add_worktree(
            &destination,
            gix::worktree::add::Head::Detached(commit_id),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        std::fs::remove_dir_all(&destination)?;

        let alias = linked_parent.join("registered");
        let err = repo
            .add_worktree(
                &alias,
                gix::worktree::add::Head::Detached(commit_id),
                gix::progress::Discard,
                &AtomicBool::default(),
            )
            .expect_err("registered destinations are compared by their real paths");
        assert!(
            matches!(err.downcast_any_ref::<gix::worktree::add::Error>(), Some(gix::worktree::add::Error::DestinationRegistered { destination }) if destination == &alias),
            "the alias supplied by the caller is identified"
        );
        Ok(())
    }

    fn branch(name: &str) -> FullName {
        format!("refs/heads/{name}").try_into().expect("valid test branch name")
    }
}

#[cfg(feature = "worktree-mutation")]
mod remove {
    /// Writes once after each removal root has been scanned, through its existing progress API.
    struct LateWriter {
        name: String,
        roots: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, std::path::PathBuf>>>,
        discard: gix::progress::Discard,
    }
    impl gix::progress::Count for LateWriter {
        fn set(&self, step: usize) {
            self.discard.set(step);
        }
        fn step(&self) -> usize {
            self.discard.step()
        }
        fn inc_by(&self, step: usize) {
            self.discard.inc_by(step);
        }
        fn counter(&self) -> gix::progress::StepShared {
            self.discard.counter()
        }
    }
    impl gix::progress::Progress for LateWriter {
        fn init(&mut self, _: Option<usize>, _: Option<gix::progress::Unit>) {
            if let Some(root) = self.roots.lock().expect("unpoisoned writer state").remove(&self.name) {
                std::fs::write(root.join("late-write"), b"written after scan").expect("root is present after scan");
            }
        }
        fn set_name(&mut self, name: String) {
            self.name = name;
        }
        fn name(&self) -> Option<String> {
            Some(self.name.clone())
        }
        fn id(&self) -> gix::progress::Id {
            gix::progress::UNKNOWN
        }
        fn message(&self, _: gix::progress::MessageLevel, _: String) {}
    }
    impl gix::progress::NestedProgress for LateWriter {
        type SubProgress = Self;
        fn add_child(&mut self, name: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                roots: self.roots.clone(),
                discard: gix::progress::Discard,
            }
        }
        fn add_child_with_id(&mut self, name: impl Into<String>, _: gix::progress::Id) -> Self {
            self.add_child(name)
        }
    }

    #[test]
    fn late_writes_to_both_roots_require_and_succeed_with_default_retries() -> gix_testtools::TestResult {
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        for max_retries in [0, gix::worktree::remove::Options::default().max_retries] {
            let (repo, _fixture) = crate::basic_rw_repo()?;
            let destinations = gix_testtools::tempfile::TempDir::new()?;
            let path = destinations.path().join("late-writes");
            let (linked, _) = repo.add_worktree(
                &path,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )?;
            let git_dir = linked.git_dir().to_owned();
            let roots = Arc::new(Mutex::new(std::collections::HashMap::from([
                ("remove worktree".to_owned(), path.clone()),
                ("remove administration".to_owned(), git_dir.clone()),
            ])));
            let result = repo
                .prepare_remove_worktree(&path)?
                .options(gix::worktree::remove::Options {
                    max_retries,
                    ..Default::default()
                })
                .remove(
                    gix::worktree::remove::Force::DiscardChanges,
                    LateWriter {
                        name: String::new(),
                        roots: roots.clone(),
                        discard: gix::progress::Discard,
                    },
                );
            assert!(
                roots.lock().expect("unpoisoned state").is_empty(),
                "both roots received a late write"
            );
            if max_retries == 0 {
                assert!(
                    result.is_err(),
                    "one-pass removal cannot delete files absent from its scan"
                );
                assert!(
                    path.join("late-write").exists() && git_dir.join("late-write").exists(),
                    "both roots retain late writes without retries"
                );
                std::fs::remove_dir_all(&git_dir)?;
            } else {
                result?;
                assert!(
                    !path.exists() && !git_dir.exists(),
                    "default retries remove both roots after bounded writes"
                );
            }
        }
        Ok(())
    }

    use std::sync::atomic::AtomicBool;

    use gix::{
        refs::{FullName, transaction::PreviousValue},
        worktree::remove::Force,
    };

    #[test]
    fn rejects_empty_targets() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let err = repo
            .remove_worktree("", Force::Never, gix::progress::Discard)
            .expect_err("an empty target cannot select a worktree");
        assert!(
            matches!(
                err.downcast_any_ref::<gix::worktree::remove::Error>(),
                Some(gix::worktree::remove::Error::EmptyTarget)
            ),
            "an empty target retains its typed error"
        );
        assert!(err.is_validation(), "an empty target is invalid input");
        Ok(())
    }

    #[test]
    fn removes_worktrees_from_a_repository_opened_with_a_relative_path() -> gix_testtools::TestResult {
        if gix_testtools::run_in_isolated_process()? {
            return Ok(());
        }
        let (source, _fixture) = crate::basic_rw_repo()?;
        let elsewhere = gix_testtools::tempfile::TempDir::new()?;
        let _cwd = gix_testtools::set_current_dir(source.workdir().expect("non-bare fixture"))?;
        let mut repo = gix::open_opts(".", crate::restricted())?;
        let destination = repo.current_dir().join("linked");
        for relative_links in [false, true] {
            repo.config_snapshot_mut().set_raw_value(
                "worktree.useRelativePaths",
                if relative_links { "true" } else { "false" },
            )?;
            let (linked, _) = repo.add_worktree(
                &destination,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )?;
            let private_git_dir = linked.git_dir().to_owned();
            drop(linked);
            let proxy = repo.worktrees()?.pop().expect("one linked worktree was registered");
            assert!(
                repo.git_dir().is_relative(),
                "the parent repository's private Git directory needs an absolute base"
            );
            assert!(
                proxy.git_dir().is_absolute() && proxy.base()?.is_absolute(),
                "proxies anchor relative paths to the repository CWD"
            );
            std::fs::write(private_git_dir.join("locked"), b"keep this worktree\n")?;
            let _moved_cwd = gix_testtools::set_current_dir(elsewhere.path())?;

            assert_eq!(
                repo.worktree_proxy_by_id(proxy.id()?)?
                    .expect("registration lookup uses the repository CWD")
                    .git_dir(),
                proxy.git_dir(),
                "looking up a proxy before or after changing CWD locates the same registration"
            );
            let err = proxy
                .clone()
                .remove(Force::DiscardChanges, gix::progress::Discard)
                .expect_err("changing CWD cannot bypass a worktree lock");
            assert!(
                matches!(err.downcast_any_ref::<gix::worktree::remove::Error>(), Some(gix::worktree::remove::Error::Locked { reason: Some(reason), .. }) if reason == "keep this worktree"),
                "the lock and its reason are read relative to the repository CWD: {err:?}"
            );
            std::fs::remove_file(private_git_dir.join("locked"))?;

            proxy.remove(Force::Never, gix::progress::Discard)?;

            assert!(!destination.exists(), "the checkout was removed");
            assert!(!private_git_dir.exists(), "the private Git directory was removed");
        }
        Ok(())
    }

    #[test]
    fn resolves_registered_paths_after_changing_current_directory() -> gix_testtools::TestResult {
        if gix_testtools::run_in_isolated_process()? {
            return Ok(());
        }
        let (source, _fixture) = crate::basic_rw_repo()?;
        let _cwd = gix_testtools::set_current_dir(source.workdir().expect("non-bare fixture"))?;
        let mut repo = gix::open_opts(".", crate::restricted())?;
        repo.config_snapshot_mut()
            .set_raw_value("worktree.useRelativePaths", "true")?;
        let first_path = repo.current_dir().join("one/shared");
        let second_path = repo.current_dir().join("two/shared");
        for destination in [&first_path, &second_path] {
            repo.add_worktree(
                destination,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )?;
        }
        let linked = gix::open_opts("one/shared", crate::restricted())?;
        let _moved_cwd = gix_testtools::set_current_dir(first_path.parent().expect("nested checkout"))?;

        for repo in [&repo, &linked] {
            for target in [first_path.as_path(), std::path::Path::new("shared")] {
                let target = repo.prepare_remove_worktree(target)?;
                assert_eq!(
                    gix_path::realpath(target.base())?,
                    first_path,
                    "absolute paths and ambiguous suffixes select the registered checkout after changing CWD"
                );
                assert_eq!(
                    target.repository()?.head_id()?,
                    source.head_id()?,
                    "the selected repository remains inspectable after changing CWD"
                );
            }
            let err = repo
                .remove_worktree(repo.current_dir(), Force::OverrideLock, gix::progress::Discard)
                .expect_err("the main worktree is still recognized after changing CWD");
            assert!(
                matches!(
                    err.downcast_any_ref::<gix::worktree::remove::Error>(),
                    Some(gix::worktree::remove::Error::MainWorktree { .. })
                ),
                "the main worktree path is resolved against the repository CWD: {err:?}"
            );
        }

        repo.remove_worktree("shared", Force::Never, gix::progress::Discard)?;
        assert!(!first_path.exists(), "the exact match in the process CWD was removed");
        assert!(second_path.exists(), "the other suffix match remains");
        Ok(())
    }

    #[test]
    fn removes_a_clean_worktree_by_suffix_without_deleting_its_branch() -> gix_testtools::TestResult {
        let (mut repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let destination = destinations.path().join("nested/topic-checkout");
        let topic = branch("remove-topic");
        repo.reference(
            topic.clone(),
            repo.head_id()?.detach(),
            PreviousValue::MustNotExist,
            "create worktree removal test branch",
        )?;
        let (linked, _) = repo.add_worktree(
            &destination,
            gix::worktree::add::Head::Attached(topic.clone()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let private_git_dir = linked.git_dir().to_owned();
        drop(linked);
        let malformed = repo.common_dir().join("worktrees/malformed");
        std::fs::create_dir(&malformed)?;
        std::fs::write(malformed.join("gitdir"), b"not a gitdir\n")?;
        repo.config_snapshot_mut()
            .set_value(&gix::config::tree::Core::IGNORE_CASE, "true")?;

        let target = repo.prepare_remove_worktree("TOPIC-CHECKOUT")?;
        assert_eq!(target.base(), gix_path::realpath(&destination)?);
        assert_eq!(
            target.repository()?.head_name()?,
            Some(topic.clone()),
            "the resolved worktree can be inspected before removal"
        );
        target.remove(Force::Never, gix::progress::Discard)?;

        assert!(!destination.exists(), "the checkout is removed");
        assert!(!private_git_dir.exists(), "the registration is removed");
        assert!(
            repo.try_find_reference(topic.as_ref())?.is_some(),
            "core worktree removal leaves the attached branch untouched"
        );
        Ok(())
    }

    #[test]
    fn permits_removing_the_current_linked_worktree_but_not_the_main_worktree() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let destination = destinations.path().join("current");
        let (linked, _) = repo.add_worktree(
            &destination,
            gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let private_git_dir = linked.git_dir().to_owned();

        linked.remove_worktree(&destination, Force::Never, gix::progress::Discard)?;
        assert!(!destination.exists(), "the current linked checkout is removed");
        assert!(!private_git_dir.exists(), "its registration is removed");

        let main_path = repo.workdir().expect("non-bare fixture").to_owned();
        let err = repo
            .remove_worktree(&main_path, Force::OverrideLock, gix::progress::Discard)
            .expect_err("the main worktree is never removable");
        assert!(err.is_validation(), "the main worktree is an invalid removal target");
        assert!(
            matches!(err.downcast_any_ref::<gix::worktree::remove::Error>(), Some(gix::worktree::remove::Error::MainWorktree { path }) if path == &main_path)
        );
        Ok(())
    }

    #[test]
    fn dirty_and_locked_worktrees_require_the_corresponding_force_level() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let dirty_path = gix_path::realpath(destinations.path().join("dirty"))?;
        let (dirty, _) = repo.add_worktree(
            &dirty_path,
            gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        std::fs::write(dirty_path.join("untracked"), b"changes")?;
        drop(dirty);
        let proxy = repo.worktrees()?.pop().expect("the dirty worktree was registered");
        let err = proxy
            .clone()
            .remove(Force::Never, gix::progress::Discard)
            .expect_err("an untracked file makes the worktree dirty");
        assert!(err.is_conflict(), "unforced removal requires reconciling local changes");
        assert!(
            matches!(err.downcast_any_ref::<gix::worktree::remove::Error>(), Some(gix::worktree::remove::Error::Dirty { path }) if path == &dirty_path),
            "an untracked file is rejected as dirty, got {err:?}"
        );
        proxy.remove(Force::DiscardChanges, gix::progress::Discard)?;

        let locked_path = gix_path::realpath(destinations.path().join("locked"))?;
        let (locked, _) = repo.add_worktree(
            &locked_path,
            gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let private_git_dir = locked.git_dir().to_owned();
        std::fs::write(private_git_dir.join("locked"), b"on external storage\n")?;
        drop(locked);
        let err = repo
            .remove_worktree(&locked_path, Force::DiscardChanges, gix::progress::Discard)
            .expect_err("one force does not override a lock");
        assert!(err.is_conflict(), "a lock requires an explicit override");
        assert!(
            matches!(err.downcast_any_ref::<gix::worktree::remove::Error>(), Some(gix::worktree::remove::Error::Locked { path, reason: Some(reason) })
                if path == &locked_path && reason == "on external storage"),
            "the lock and its reason are reported"
        );
        std::fs::write(locked_path.join("untracked"), b"changes")?;
        repo.remove_worktree(&locked_path, Force::OverrideLock, gix::progress::Discard)?;
        assert!(
            !locked_path.exists(),
            "overriding a lock still permits discarding changes"
        );
        assert!(!private_git_dir.exists(), "forced removal unregisters the worktree");
        Ok(())
    }

    #[test]
    fn untracked_files_hidden_by_status_configuration_require_force() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let destination = gix_path::realpath(destinations.path().join("hidden-untracked"))?;
        let (linked, _) = repo.add_worktree(
            &destination,
            gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let private_git_dir = linked.git_dir().to_owned();
        let untracked_path = destination.join("untracked");
        std::fs::write(&untracked_path, b"local changes")?;
        drop(linked);

        let main_work_dir = repo.workdir().expect("non-bare fixture");
        gix_testtools::git(main_work_dir, "config --local status.showUntrackedFiles no")?;
        assert_eq!(
            gix_testtools::git(&destination, "status --porcelain")?,
            "",
            "the status display configuration hides the untracked file"
        );
        // Git also honors the display setting during removal, so explicitly enable scanning for this reference check.
        let git_err = gix_testtools::git(
            main_work_dir,
            "-c status.showUntrackedFiles=all worktree remove hidden-untracked",
        )
        .expect_err("Git refuses to remove the checkout when untracked-file scanning is enabled");
        assert!(
            git_err.to_string().contains("modified or untracked files"),
            "Git rejects removal because of the hidden untracked file: {git_err}"
        );

        let err = repo
            .remove_worktree(&destination, Force::Never, gix::progress::Discard)
            .expect_err("status display configuration must not bypass removal safety checks");
        assert!(err.is_conflict(), "hidden untracked files still prevent removal");
        assert!(
            matches!(err.downcast_any_ref::<gix::worktree::remove::Error>(), Some(gix::worktree::remove::Error::Dirty { path }) if path == &destination),
            "the hidden untracked file is rejected as dirty, got {err:?}"
        );
        assert_eq!(
            std::fs::read(&untracked_path)?,
            b"local changes",
            "rejected removal preserves the untracked contents"
        );
        assert!(private_git_dir.is_dir(), "rejected removal preserves the registration");

        repo.remove_worktree(&destination, Force::DiscardChanges, gix::progress::Discard)?;
        assert!(!destination.exists(), "force permits discarding hidden untracked files");
        assert!(!private_git_dir.exists(), "forced removal unregisters the worktree");
        Ok(())
    }

    #[test]
    fn initialized_submodules_require_force() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let destination = destinations.path().join("submodules");
        let (linked, _) = repo.add_worktree(
            &destination,
            gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let private_git_dir = linked.git_dir().to_owned();
        std::fs::create_dir(private_git_dir.join("modules"))?;
        drop(linked);

        let err = repo
            .remove_worktree(&destination, Force::Never, gix::progress::Discard)
            .expect_err("initialized submodules prevent an unforced removal");
        assert!(err.is_conflict(), "initialized submodules require forced removal");
        assert!(matches!(
            err.downcast_any_ref::<gix::worktree::remove::Error>(),
            Some(gix::worktree::remove::Error::ContainsSubmodule { .. })
        ));

        repo.remove_worktree(&destination, Force::DiscardChanges, gix::progress::Discard)?;
        Ok(())
    }

    #[test]
    fn backlink_validation_is_never_forced_and_missing_checkouts_are_unregistered() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let invalid_path = destinations.path().join("invalid-backlink");
        let (invalid, _) = repo.add_worktree(
            &invalid_path,
            gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let private_git_dir = invalid.git_dir().to_owned();
        std::fs::write(invalid_path.join(".git"), "gitdir: ../elsewhere\n")?;
        drop(invalid);
        let err = repo
            .remove_worktree(&invalid_path, Force::OverrideLock, gix::progress::Discard)
            .expect_err("force cannot bypass backlink validation");
        assert!(err.is_corrupted(), "a mismatched backlink is inconsistent metadata");
        assert!(matches!(
            err.downcast_any_ref::<gix::worktree::remove::Error>(),
            Some(gix::worktree::remove::Error::BacklinkMismatch { .. })
        ));
        assert!(invalid_path.exists(), "an invalid checkout is retained");
        assert!(private_git_dir.exists(), "an invalid registration is retained");

        let missing_path = destinations.path().join("missing");
        let (missing, _) = repo.add_worktree(
            &missing_path,
            gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let private_git_dir = missing.git_dir().to_owned();
        drop(missing);
        std::fs::remove_dir_all(&missing_path)?;
        let target = repo.prepare_remove_worktree(&missing_path)?;
        assert_eq!(
            target.repository()?.head_id()?.detach(),
            repo.head_id()?.detach(),
            "private metadata remains inspectable without the checkout"
        );
        target.remove(Force::Never, gix::progress::Discard)?;
        assert!(!private_git_dir.exists(), "a missing checkout is unregistered");

        let blocked_parent = destinations.path().join("non-directory");
        let blocked_path = blocked_parent.join("missing");
        let (blocked, _) = repo.add_worktree(
            &blocked_path,
            gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
            gix::progress::Discard,
            &AtomicBool::default(),
        )?;
        let private_git_dir = blocked.git_dir().to_owned();
        drop(blocked);
        std::fs::remove_dir_all(&blocked_parent)?;
        std::fs::write(&blocked_parent, b"not a directory")?;
        repo.remove_worktree(&blocked_path, Force::Never, gix::progress::Discard)?;
        assert!(
            !private_git_dir.exists(),
            "a checkout hidden behind a non-directory ancestor is unregistered"
        );
        Ok(())
    }

    #[test]
    fn ambiguous_suffixes_can_be_disambiguated_with_an_exact_path() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let first_path = destinations.path().join("one/shared");
        let second_path = destinations.path().join("two/shared");
        for destination in [&first_path, &second_path] {
            repo.add_worktree(
                destination,
                gix::worktree::add::Head::Detached(repo.head_id()?.detach()),
                gix::progress::Discard,
                &AtomicBool::default(),
            )?;
        }

        let err = repo
            .remove_worktree("shared", Force::Never, gix::progress::Discard)
            .expect_err("a non-unique suffix is ambiguous");
        assert!(err.is_validation(), "an ambiguous suffix cannot select a worktree");
        assert!(
            matches!(err.downcast_any_ref::<gix::worktree::remove::Error>(), Some(gix::worktree::remove::Error::Ambiguous { candidates, .. }) if candidates.len() == 2),
            "all suffix matches are reported"
        );
        repo.remove_worktree(&first_path, Force::Never, gix::progress::Discard)?;
        assert!(!first_path.exists(), "the exact match was removed");
        assert!(second_path.exists(), "the other suffix match remains");

        let missing = destinations.path().join("does-not-exist");
        let err = repo
            .remove_worktree(&missing, Force::Never, gix::progress::Discard)
            .expect_err("an unknown path is reported as such");
        assert!(err.is_not_found(), "an unknown target has no registered worktree");
        assert!(
            matches!(err.downcast_any_ref::<gix::worktree::remove::Error>(), Some(gix::worktree::remove::Error::NotFound { target }) if target == &missing)
        );
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn canonical_paths_resolve_git_registered_worktrees() -> gix_testtools::TestResult {
        let (repo, _fixture) = crate::basic_rw_repo()?;
        let destinations = gix_testtools::tempfile::TempDir::new()?;
        let destination = destinations.path().join("topic");
        let output = gix_testtools::git_command(repo.workdir().expect("non-bare fixture"))
            .args(["worktree", "add", "--detach"])
            .arg(&destination)
            .output()?;
        assert!(
            output.status.success(),
            "Git registers the disposable worktree: {output:?}"
        );
        let canonical = destination.canonicalize()?;
        let target = repo.prepare_remove_worktree(&canonical)?;
        assert_eq!(
            target.base().canonicalize()?,
            canonical,
            "canonical, verbatim, and short Windows names identify the same registered worktree"
        );
        target.remove(Force::Never, gix::progress::Discard)?;
        assert!(
            !destination.exists(),
            "the equivalent path removes the intended worktree"
        );
        Ok(())
    }

    fn branch(name: &str) -> FullName {
        format!("refs/heads/{name}").try_into().expect("valid test branch name")
    }
}

#[cfg(feature = "worktree-stream")]
fn expected_buffer_length(repo: &gix::Repository) -> usize {
    /// The buffer length for SHA1 archives.
    #[cfg(target_pointer_width = "64")]
    #[cfg(feature = "worktree-stream")]
    const EXPECTED_BUFFER_LENGTH: usize = 102;
    /// The buffer length for SHA1 archives on 32bit machines.
    #[cfg(target_pointer_width = "32")]
    #[cfg(feature = "worktree-stream")]
    const EXPECTED_BUFFER_LENGTH: usize = 86;

    EXPECTED_BUFFER_LENGTH + repo.object_hash().len_in_hex() - gix::hash::Kind::Sha1.len_in_hex()
}

#[test]
#[cfg(feature = "worktree-stream")]
fn stream() -> TestResult {
    let repo = crate::named_repo("make_packed_and_loose.sh")?;
    let mut stream = repo.worktree_stream(repo.head_commit()?.tree_id()?)?.0.into_read();
    assert_eq!(
        std::io::copy(&mut stream, &mut std::io::sink())?,
        expected_buffer_length(&repo) as u64,
        "there is some content in the stream, it works"
    );
    Ok(())
}

#[test]
#[cfg(feature = "worktree-archive")]
fn archive() -> TestResult {
    let repo = crate::named_repo("make_packed_and_loose.sh")?;
    let (stream, _index) = repo.worktree_stream(repo.head_commit()?.tree_id()?)?;
    let mut buf = Vec::<u8>::new();

    repo.worktree_archive(
        stream,
        std::io::Cursor::new(&mut buf),
        gix_utils::progress::Discard,
        &std::sync::atomic::AtomicBool::default(),
        Default::default(),
    )?;
    assert_eq!(buf.len(), expected_buffer_length(&repo), "default format is internal");
    Ok(())
}

mod with_core_worktree_config {
    use gix_testtools::TestResult;
    use std::io::BufRead;

    #[test]
    #[cfg(feature = "index")]
    fn relative() -> TestResult {
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
    fn bare_relative() -> TestResult {
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

    #[test]
    #[cfg(unix)] // symlinks are used here, let's not try our luck on Windows.
    fn relative_through_symlinked_ancestor_keeps_callers_path_namespace() -> TestResult {
        let link = gix_testtools::scripted_fixture_read_only("make_core_worktree_repo.sh")?.join("symlinked-ancestor");

        let repo = gix::open_opts(link.join("relative-worktree"), crate::restricted())?;
        assert_eq!(
            repo.workdir(),
            Some(link.join("worktree").as_path()),
            "if a symlink in an ancestor changes nothing about how the relative worktree resolves, \
             the caller's path namespace is kept instead of jumping to the canonicalized one"
        );
        Ok(())
    }

    #[test]
    #[cfg(unix)] // symlinks are used here, let's not try our luck on Windows.
    fn relative_from_symlinked_git_dir() -> TestResult {
        let fixture = gix_testtools::scripted_fixture_read_only("make_core_worktree_repo.sh")?;
        let root = fixture.join("linked-git-dir-detached-worktree");
        let repo = gix::open_opts(root.join("home"), crate::restricted())?;
        let git_worktree = std::fs::read_to_string(root.join("worktree.baseline"))?;

        assert_eq!(
            gix_path::realpath(repo.workdir().expect("core.worktree is configured"))?,
            gix_path::realpath(git_worktree.trim_end())?,
            "relative core.worktree values from repository config are resolved against the real git dir"
        );
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
            .map_while(std::result::Result::ok)
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
            let root = gix_path::from_bstr(Cow::Borrowed(fields(self.lines.next()?).1))
                .expect("Git baseline paths can be represented natively")
                .into_owned();
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
fn worktree_listing_errors_preserve_io_errors() -> TestResult {
    let (repo, _fixture) = crate::basic_rw_repo()?;
    std::fs::write(repo.common_dir().join("worktrees"), b"not a directory")?;

    let listings: [gix::Result<()>; 2] = [
        repo.worktrees().map(|_| ()),
        repo.worktrees_including_main().map(|_| ()),
    ];
    for listing in listings {
        let err = listing.expect_err("a file cannot be enumerated as a worktrees directory");
        let io_err = err
            .downcast_any_ref::<std::io::Error>()
            .expect("the public error retains its underlying I/O error");
        assert_eq!(
            io_err.kind(),
            std::io::ErrorKind::NotADirectory,
            "both listing APIs preserve the directory traversal failure"
        );
    }
    Ok(())
}

#[test]
fn from_bare_parent_repo() -> TestResult {
    let Some(dir) = gix_testtools::scripted_fixture_read_only_with_args_with_git_version(
        "make_worktree_repo.sh",
        ["bare"],
        |version| version >= (2, 31, 0),
    )?
    else {
        return Ok(());
    };
    let repo = gix::open_opts(dir.join("repo.git"), crate::restricted())?;

    Ok(run_assertions(repo, true /* bare */)?)
}

#[test]
fn from_nonbare_parent_repo() -> TestResult {
    let Some(dir) = gix_testtools::scripted_fixture_read_only_with_git_version("make_worktree_repo.sh", |version| {
        version >= (2, 31, 0)
    })?
    else {
        return Ok(());
    };
    let repo = gix::open_opts(dir.join("repo"), crate::restricted())?;

    Ok(run_assertions(repo, false /* bare */)?)
}

#[test]
fn linked_worktree_proxy_base_with_relative_linking_files() -> TestResult {
    let fixture = gix_testtools::scripted_fixture_read_only_needs_archive("make_worktree_relative_linking.sh")?;
    let main = fixture.join("main");
    let linked = fixture.join("linked");
    let private_git_dir = main.join(".git/worktrees/linked");
    let repo = gix::open_opts(&main, crate::restricted())?;
    let worktrees = repo.worktrees()?;
    assert_eq!(worktrees.len(), 1, "the relative-path fixture has one linked worktree");
    let proxy = worktrees.into_iter().next().expect("one worktree");

    assert_eq!(
        gix_path::realpath(proxy.base()?)?,
        gix_path::realpath(&linked)?,
        "proxy bases resolve relative worktrees/<id>/gitdir paths against the private git dir"
    );
    let linked_repo = proxy.into_repo()?;
    assert_eq!(
        linked_repo.workdir().map(gix_path::realpath).transpose()?,
        Some(gix_path::realpath(&linked)?)
    );
    assert_eq!(
        linked_repo.git_dir(),
        repo.current_dir().join(private_git_dir),
        "the private Git directory stays anchored when the parent repository was opened with a relative path"
    );

    Ok(())
}

#[test]
#[cfg(unix)]
fn linked_worktree_proxy_base_with_symlinked_main_repo() -> TestResult {
    let fixture = gix_testtools::scripted_fixture_read_only_needs_archive("make_worktree_relative_linking.sh")?;
    let linked = fixture.join("actual/linked");
    let main_symlink = fixture.join("main-symlink");

    let repo = gix::open_opts(&main_symlink, crate::restricted())?;
    let worktrees = repo.worktrees()?;
    assert_eq!(worktrees.len(), 1, "the relative-path fixture has one linked worktree");
    let proxy = worktrees.into_iter().next().expect("one worktree");

    assert_eq!(
        gix_path::realpath(proxy.base()?)?,
        gix_path::realpath(&linked)?,
        "proxy bases preserve symlink semantics when resolving relative worktrees/<id>/gitdir paths"
    );
    let repo = proxy.into_repo()?;
    assert_eq!(
        repo.workdir().map(gix_path::realpath).transpose()?,
        Some(gix_path::realpath(&linked)?)
    );

    Ok(())
}

#[test]
fn from_nonbare_parent_repo_set_workdir() -> gix_testtools::TestResult {
    let Some(dir) = gix_testtools::scripted_fixture_read_only_with_git_version("make_worktree_repo.sh", |version| {
        version >= (2, 31, 0)
    })?
    else {
        return Ok(());
    };
    let mut repo = gix::open_opts(dir.join("repo"), crate::restricted())?;

    assert!(repo.worktree().is_some_and(|wt| wt.is_main()), "we have main worktree");

    let worktrees = repo.worktrees()?;
    assert_eq!(worktrees.len(), 6);

    let linked_wt_dir = worktrees.first().unwrap().base()?;
    repo.set_workdir(linked_wt_dir)?;

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

fn run_assertions(main_repo: gix::Repository, should_be_bare: bool) -> Result {
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
            main_repo.head_name().unwrap().expect("no detached head"),
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

    let linked_repo = actual
        .first()
        .expect("the fixture has linked worktrees")
        .clone()
        .into_repo()
        .expect("the first linked checkout exists");
    for repo in [&main_repo, &linked_repo] {
        let repositories = repo
            .worktrees_including_main()
            .expect("the fixture's worktrees can be listed")
            .collect::<gix::Result<Vec<_>>>()
            .expect("repositories open even for removed checkouts");
        let (main, linked) = repositories.split_first().expect("the main repository is included");
        assert_eq!(main, &main_repo, "the main repository comes first, even when bare");
        assert_eq!(
            linked.iter().map(gix::Repository::git_dir).collect::<Vec<_>>(),
            actual.iter().map(gix::worktree::Proxy::git_dir).collect::<Vec<_>>(),
            "each linked worktree is listed once in private Git directory order, from either caller"
        );
    }

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
        assert_eq!(
            actual.is_prunable(),
            expected.prunable.is_some(),
            "prunability matches `git worktree list --porcelain`"
        );
        // TODO: check id of expected worktree, but need access to .gitdir from worktree base
        let proxy_id = actual.id()?.to_owned();
        assert_eq!(
            base.is_dir(),
            expected.prunable.is_none(),
            "in our case prunable repos have no worktree base"
        );

        assert_eq!(
            main_repo.worktree_proxy_by_id(actual.id()?)?.expect("exists").git_dir(),
            actual.git_dir(),
            "we can basically get the same proxy by its ID explicitly"
        );

        let repo = if base.is_dir() {
            let repo = actual.clone().into_repo().unwrap();
            assert_eq!(
                &gix::open_opts(base, crate::restricted()).expect("linked worktree repository opens"),
                &repo,
                "repos are considered the same no matter if opened from worktree or from git dir"
            );
            repo
        } else {
            let err = actual.clone().into_repo().expect_err("the worktree base is missing");
            insta::allow_duplicates! {
                insta::assert_debug_snapshot!(gix_testtools::redact_debug_snapshot(&err, &[(&base.to_string_lossy(), "<worktree>")]), "opening a worktree reports its inaccessible base", @r#"Worktree at "<worktree>" is inaccessible"#);
            }
            actual.clone().into_repo_with_possibly_inaccessible_worktree().unwrap()
        };
        let worktree = repo.worktree().expect("linked worktrees have at least a base path");
        assert!(!worktree.is_main());
        assert_eq!(worktree.lock_reason(), proxy_lock_reason);
        assert_eq!(worktree.is_locked(), proxy_is_locked);
        assert_eq!(worktree.id()?, Some(proxy_id.as_ref()));
        assert_eq!(
            repo.main_repo().unwrap(),
            main_repo,
            "main repo from worktree repo is the actual main repo"
        );

        let proxy_by_id = repo
            .worktree_proxy_by_id(actual.id()?)?
            .expect("can get the proxy from a linked repo as well");
        assert_eq!(
            proxy_by_id.git_dir(),
            actual.git_dir(),
            "The git directories are the same"
        );
        assert_eq!(
            gix_path::realpath(proxy_by_id.git_dir()).ok(),
            gix_path::realpath(actual.git_dir()).ok(),
            "the git directories are effectively the same"
        );
    }
    Ok(())
}

/// Tests for worktree add and remove operations
#[cfg(feature = "worktree-mutation")]
mod mutation {
    use std::process::Command;

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

    fn add_unbacklinked_worktree_entry(
        repo: &gix::Repository,
        id: &str,
        base: &std::path::Path,
    ) -> std::io::Result<std::path::PathBuf> {
        let admin_dir = repo.common_dir().join("worktrees").join(id);
        std::fs::create_dir_all(&admin_dir)?;
        std::fs::write(admin_dir.join("gitdir"), format!("{}\n", base.join(".git").display()))?;
        Ok(admin_dir)
    }

    fn git(dir: &std::path::Path, args: &[&str]) -> crate::Result {
        let output = Command::new("git").args(args).current_dir(dir).output()?;
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    fn add_committed_submodule(worktree_path: &std::path::Path) -> crate::Result {
        let parent = worktree_path.parent().expect("worktree has parent directory");
        let submodule_origin = parent.join("submodule-origin");
        std::fs::create_dir(&submodule_origin)?;
        git(&submodule_origin, &["init", "-q"])?;
        git(&submodule_origin, &["config", "user.name", "Test User"])?;
        git(&submodule_origin, &["config", "user.email", "test@example.com"])?;
        std::fs::write(submodule_origin.join("file"), "submodule\n")?;
        git(&submodule_origin, &["add", "file"])?;
        git(&submodule_origin, &["commit", "-q", "-m", "initial submodule"])?;

        let submodule_url = submodule_origin.to_string_lossy();
        git(
            worktree_path,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                submodule_url.as_ref(),
                "nested",
            ],
        )?;
        git(worktree_path, &["commit", "-q", "-m", "add submodule"])?;
        Ok(())
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
        fn default_creates_branch_from_path_basename() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let head_id = repo.head_id()?.detach();
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt.default-branch");

            let proxy = repo.worktree_add(&worktree_path, Default::default())?;
            let wt_repo = proxy.into_repo()?;

            assert_eq!(
                wt_repo.head_name()?.expect("symbolic HEAD").as_bstr(),
                "refs/heads/wt.default-branch"
            );
            assert_eq!(wt_repo.head_id()?.detach(), head_id);
            assert!(worktree_path.join("a").is_file(), "files should be checked out");
            Ok(())
        }

        #[test]
        fn default_on_unborn_repo_creates_unborn_branch_from_path_basename() -> crate::Result {
            let tmp = gix_testtools::tempfile::tempdir()?;
            let repo_path = tmp.path().join("repo");
            let repo = gix::ThreadSafeRepository::init_opts(
                &repo_path,
                gix::create::Kind::WithWorktree,
                Default::default(),
                crate::restricted(),
            )?
            .to_thread_local();
            let worktree_path = tmp.path().join("wt-unborn");

            let proxy = repo.worktree_add(&worktree_path, Default::default())?;
            let wt_repo = proxy.into_repo()?;

            assert_eq!(
                wt_repo.head_name()?.expect("symbolic HEAD").as_bstr(),
                "refs/heads/wt-unborn"
            );
            let mut head = wt_repo.head()?;
            assert!(head.try_peel_to_id()?.is_none(), "unborn branch has no object id");
            assert!(
                !repo.git_dir().join("refs").join("heads").join("wt-unborn").exists(),
                "unborn branches are not materialized as refs"
            );
            Ok(())
        }

        #[test]
        fn detach_on_unborn_repo_without_start_point_fails() -> crate::Result {
            let tmp = gix_testtools::tempfile::tempdir()?;
            let repo_path = tmp.path().join("repo");
            let repo = gix::ThreadSafeRepository::init_opts(
                &repo_path,
                gix::create::Kind::WithWorktree,
                Default::default(),
                crate::restricted(),
            )?
            .to_thread_local();
            let worktree_path = tmp.path().join("wt-detach-unborn");

            let result = repo.worktree_add(
                &worktree_path,
                gix::worktree::add::Options {
                    detach: true,
                    ..Default::default()
                },
            );

            assert!(
                result.as_ref().is_err_and(gix::Error::is_validation),
                "detaching an unborn HEAD without a start point must fail, got {result:?}"
            );
            assert!(!worktree_path.exists(), "failure happens before creating the worktree");
            Ok(())
        }

        #[test]
        fn checkout_respects_configured_ntfs_protection() -> crate::Result {
            let (mut repo, _keep) = repo_rw()?;
            {
                let mut config = repo.config_edit()?;
                config.set_value(&gix::config::tree::Core::PROTECT_NTFS, "false")?;
                config.commit()?;
            }

            let blob_id = repo.write_blob("content\n")?.detach();
            let file_tree_id = repo
                .write_object(gix::objs::TreeRef {
                    entries: vec![gix::objs::tree::EntryRef {
                        mode: gix::objs::tree::EntryKind::Blob.into(),
                        filename: b"file".as_bstr(),
                        oid: blob_id.as_ref(),
                    }],
                })?
                .detach();
            let root_tree_id = repo
                .write_object(gix::objs::TreeRef {
                    entries: vec![gix::objs::tree::EntryRef {
                        mode: gix::objs::tree::EntryKind::Tree.into(),
                        filename: b"git~1".as_bstr(),
                        oid: file_tree_id.as_ref(),
                    }],
                })?
                .detach();
            let parent = repo.head_id()?.detach();
            let commit_id = repo.commit("HEAD", "add ntfs opt-out path", root_tree_id, [parent])?;

            let worktree_path = repo
                .workdir()
                .unwrap()
                .parent()
                .unwrap()
                .join("wt-protect-ntfs-disabled");
            let proxy = repo.worktree_add(
                &worktree_path,
                gix::worktree::add::Options {
                    detach: true,
                    ..Default::default()
                },
            )?;

            assert_eq!(proxy.into_repo()?.head_id()?.detach(), commit_id.detach());
            assert!(
                worktree_path.join("git~1").join("file").is_file(),
                "worktree add should honor core.protectNTFS=false when creating the checkout index"
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
        fn start_point_local_branch_checks_out_branch() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-start-branch");

            let proxy = repo.worktree_add(
                &worktree_path,
                gix::worktree::add::Options {
                    start_point: Some(b"feature-1".as_bstr()),
                    ..Default::default()
                },
            )?;
            let wt_repo = proxy.into_repo()?;

            assert_eq!(
                wt_repo.head_name()?.expect("symbolic HEAD").as_bstr(),
                "refs/heads/feature-1"
            );
            assert_eq!(
                wt_repo.head_id()?.detach(),
                repo.find_reference("refs/heads/feature-1")?.id().detach()
            );
            Ok(())
        }

        #[test]
        fn start_point_prefers_local_branch_over_ambiguous_tag() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let tag_target = repo.head_commit()?.parent_ids().next().expect("parent");
            repo.tag(
                "feature-1",
                tag_target,
                gix_object::Kind::Commit,
                Some(repo.committer().expect("present")?),
                "ambiguous with branch",
                gix_ref::transaction::PreviousValue::MustNotExist,
            )?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-ambiguous-start");

            let proxy = repo.worktree_add(
                &worktree_path,
                gix::worktree::add::Options {
                    start_point: Some(b"feature-1".as_bstr()),
                    ..Default::default()
                },
            )?;
            let wt_repo = proxy.into_repo()?;
            let branch_id = repo.find_reference("refs/heads/feature-1")?.id().detach();

            assert_eq!(
                wt_repo.head_name()?.expect("symbolic HEAD").as_bstr(),
                "refs/heads/feature-1"
            );
            assert_eq!(wt_repo.head_id()?.detach(), branch_id);
            assert_ne!(branch_id, tag_target, "fixture keeps the tag target distinct");
            Ok(())
        }

        #[test]
        fn start_point_checked_out_branch_fails_before_creating_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-main-again");

            let result = repo.worktree_add(
                &worktree_path,
                gix::worktree::add::Options {
                    start_point: Some(b"main".as_bstr()),
                    ..Default::default()
                },
            );

            assert!(
                matches!(
                    result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_any_ref::<gix::worktree::add::Error>()),
                    Some(gix::worktree::add::Error::CheckedOut { .. })
                ),
                "local branch start points must be subject to checked-out branch protection, got {result:?}"
            );
            assert!(!worktree_path.exists(), "failure should happen before creating files");
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
        fn with_reset_branch_resets_existing_branch_to_start_point() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-reset-branch");
            let old_feature_id = repo.find_reference("refs/heads/feature-1")?.id().detach();
            let target_id = repo
                .head_commit()?
                .parent_ids()
                .next()
                .expect("fixture has a parent commit")
                .detach();
            let target_spec = target_id.to_string();
            assert_ne!(
                old_feature_id, target_id,
                "fixture should keep feature-1 distinct from HEAD's parent"
            );

            let proxy = repo.worktree_add(
                &worktree_path,
                gix::worktree::add::Options {
                    reset_branch: Some(b"feature-1".as_bstr()),
                    start_point: Some(target_spec.as_bytes().as_bstr()),
                    ..Default::default()
                },
            )?;

            assert_eq!(repo.find_reference("refs/heads/feature-1")?.id().detach(), target_id);
            let wt_repo = proxy.into_repo()?;
            assert_eq!(
                wt_repo.head_name()?.expect("should have branch").as_bstr(),
                "refs/heads/feature-1"
            );
            assert_eq!(wt_repo.head_id()?.detach(), target_id);
            Ok(())
        }

        #[test]
        fn reset_branch_fails_when_branch_is_checked_out() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let first_path = repo.workdir().unwrap().parent().unwrap().join("wt-feature-reset");
            repo.worktree_add(
                &first_path,
                gix::worktree::add::Options {
                    branch: Some(b"feature-1".as_bstr()),
                    ..Default::default()
                },
            )?;

            let second_path = repo.workdir().unwrap().parent().unwrap().join("wt-feature-reset-2");
            let result = repo.worktree_add(
                &second_path,
                gix::worktree::add::Options {
                    reset_branch: Some(b"feature-1".as_bstr()),
                    start_point: Some(b"HEAD".as_bstr()),
                    ..Default::default()
                },
            );

            assert!(
                matches!(
                    result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_any_ref::<gix::worktree::add::Error>()),
                    Some(gix::worktree::add::Error::CheckedOut { .. })
                ),
                "resetting a checked-out branch must fail, got {result:?}"
            );
            assert!(
                !second_path.exists(),
                "failure should happen before creating a worktree"
            );
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
                result.as_ref().is_err_and(|err| err
                    .downcast_any_ref::<std::io::Error>()
                    .is_some_and(|source| source.kind() == std::io::ErrorKind::AlreadyExists)),
                "should fail with PathExists error, got {result:?}"
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
                matches!(
                    result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_any_ref::<gix::worktree::add::Error>()),
                    Some(gix::worktree::add::Error::CheckedOut { .. })
                ),
                "should fail because branch is already checked out, got {result:?}"
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
                result.as_ref().is_err_and(gix::Error::is_not_found),
                "should fail with BranchNotFound error, got {result:?}"
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
                result.as_ref().is_err_and(gix::Error::is_validation),
                "should fail with MutuallyExclusiveOptions error, got {result:?}"
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
            assert_ne!(proxy1.id()?, proxy2.id()?, "IDs should be different");
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
            assert_eq!(locked_content, format!("{lock_reason}\n"));
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
        fn explicit_start_point_does_not_read_a_missing_head_object() -> crate::Result {
            use gix::worktree::add::Options;
            let (repo, fixture) = repo_rw()?;
            let target = repo.head_id()?.detach();
            let hex = target.to_string();
            std::fs::write(repo.git_dir().join("HEAD"), format!("{}\n", "1".repeat(hex.len())))?;
            let start = Options {
                start_point: Some(hex.as_str().into()),
                ..Default::default()
            };
            for (name, options) in [
                (
                    "detached",
                    Options {
                        detach: true,
                        ..start.clone()
                    },
                ),
                (
                    "new",
                    Options {
                        new_branch: Some(b"from-explicit".as_bstr()),
                        ..start.clone()
                    },
                ),
                (
                    "reset",
                    Options {
                        reset_branch: Some(b"reset-explicit".as_bstr()),
                        ..start
                    },
                ),
            ] {
                let linked = repo.worktree_add(fixture.path().join(name), options)?.into_repo()?;
                assert_eq!(linked.head_id()?.detach(), target);
            }
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
            let worktree_id = proxy.id()?.to_owned();
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
            let worktree_id = proxy.id()?.to_owned();

            // Try to remove without force
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());

            assert!(
                matches!(
                    result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_any_ref::<gix::worktree::remove::Error>()),
                    Some(gix::worktree::remove::Error::Locked { .. })
                ),
                "should fail because worktree is locked, got {result:?}"
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
            let worktree_id = proxy.id()?.to_owned();

            // Remove with force level 2 (required for locked worktrees)
            let remove_options = gix::worktree::remove::Force::OverrideLock;
            repo.worktree_remove(worktree_id.as_bstr(), remove_options)?;

            assert!(
                !worktree_path.exists(),
                "locked worktree should be removed with force level 2"
            );
            Ok(())
        }

        #[test]
        fn refuses_to_remove_worktree_containing_submodules() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let worktree_path = repo.workdir().unwrap().parent().unwrap().join("wt-submodule-remove");

            let proxy = repo.worktree_add(
                &worktree_path,
                gix::worktree::add::Options {
                    detach: true,
                    ..Default::default()
                },
            )?;
            let worktree_id = proxy.id()?.to_owned();
            add_committed_submodule(&worktree_path)?;

            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());
            assert!(
                matches!(
                    result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_any_ref::<gix::worktree::remove::Error>()),
                    Some(gix::worktree::remove::Error::ContainsSubmodule { .. })
                ),
                "worktree remove must refuse worktrees containing submodules, got {result:?}"
            );
            assert!(
                worktree_path.exists(),
                "refused remove must leave the worktree in place"
            );
            Ok(())
        }

        #[test]
        fn fails_on_nonexistent_worktree() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            let result = repo.worktree_remove(b"nonexistent".as_bstr(), Default::default());

            assert!(
                result.as_ref().is_err_and(gix::Error::is_not_found),
                "should fail with NotFound error, got {result:?}"
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
            let worktree_id = proxy.id()?.to_owned();
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
            let worktree_id = proxy.id()?.to_owned();

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
            let wt1_id = proxy1.id()?.to_owned();
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
            let worktree_id = proxy.id()?.to_owned();

            // Manually delete the worktree directory (simulating prunable state)
            std::fs::remove_dir_all(&worktree_path)?;

            // Remove should fail because it's locked (even though the dir is missing)
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());
            assert!(
                matches!(
                    result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_any_ref::<gix::worktree::remove::Error>()),
                    Some(gix::worktree::remove::Error::Locked { .. })
                ),
                "should fail because worktree is locked, got {result:?}"
            );

            // But with force level 2 it should succeed
            repo.worktree_remove(worktree_id.as_bstr(), gix::worktree::remove::Force::OverrideLock)?;
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
            let worktree_id = proxy.id()?.to_owned();

            // Try to remove without force
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());

            // Verify the error contains the lock reason
            match result
                .as_ref()
                .err()
                .and_then(|err| err.downcast_any_ref::<gix::worktree::remove::Error>())
            {
                Some(gix::worktree::remove::Error::Locked { path, reason }) => {
                    assert_eq!(path, &worktree_path);
                    assert_eq!(
                        reason.as_ref().map(|s| s.as_bstr()),
                        Some(lock_reason.as_bytes().as_bstr()),
                        "lock reason should be in error"
                    );
                }
                other => panic!("expected Locked error, got {other:?}"),
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
            let worktree_id = proxy.id()?.to_owned();

            // Make the worktree dirty by modifying a tracked file
            std::fs::write(worktree_path.join("a"), "modified content")?;

            // Try to remove without force - should fail
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());
            assert!(
                matches!(
                    result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_any_ref::<gix::worktree::remove::Error>()),
                    Some(gix::worktree::remove::Error::Dirty { .. })
                ),
                "should fail because worktree is dirty, got {result:?}"
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
            let worktree_id = proxy.id()?.to_owned();

            // Make the worktree dirty
            std::fs::write(worktree_path.join("a"), "modified content")?;

            // Remove with force level 1 - should succeed
            let remove_options = gix::worktree::remove::Force::DiscardChanges;
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
            let worktree_id = proxy.id()?.to_owned();

            // Create an untracked file (not added to git)
            std::fs::write(worktree_path.join("untracked-file.txt"), "untracked content")?;

            // Try to remove without force - should fail
            let result = repo.worktree_remove(worktree_id.as_bstr(), Default::default());
            assert!(
                matches!(
                    result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_any_ref::<gix::worktree::remove::Error>()),
                    Some(gix::worktree::remove::Error::Dirty { .. })
                ),
                "should fail because worktree has untracked files, got {result:?}"
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
            let worktree_id = proxy.id()?.to_owned();

            // Create an untracked file
            std::fs::write(worktree_path.join("untracked-file.txt"), "untracked content")?;

            // Remove with force level 1 - should succeed
            let remove_options = gix::worktree::remove::Force::DiscardChanges;
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
            let worktree_id = proxy.id()?.to_owned();

            // Remove with force level 1 - should still fail because locked
            let remove_options = gix::worktree::remove::Force::DiscardChanges;
            let result = repo.worktree_remove(worktree_id.as_bstr(), remove_options);
            assert!(
                matches!(
                    result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_any_ref::<gix::worktree::remove::Error>()),
                    Some(gix::worktree::remove::Error::Locked { .. })
                ),
                "force level 1 should still fail on locked worktrees, got {result:?}"
            );
            Ok(())
        }

        #[test]
        fn refuses_to_remove_directory_without_gitfile_backlink() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let victim = repo.workdir().unwrap().parent().unwrap().join("not-a-worktree");
            std::fs::create_dir_all(&victim)?;
            std::fs::write(victim.join("important"), "keep me")?;
            let admin_dir = add_unbacklinked_worktree_entry(&repo, "unbacklinked-remove", &victim)?;

            let result = repo.worktree_remove(
                b"unbacklinked-remove".as_bstr(),
                gix::worktree::remove::Force::OverrideLock,
            );

            assert!(
                result.as_ref().is_err_and(gix::Error::is_not_found),
                "should refuse to remove a directory without a matching .git backlink, got {result:?}"
            );
            assert!(victim.join("important").exists(), "unrelated directory must remain");
            assert!(admin_dir.exists(), "admin dir remains for repair or explicit cleanup");
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
                "main worktree should not be lockable (not found in worktrees/), got {result:?}"
            );
            Ok(())
        }

        #[test]
        fn unlock_main_worktree_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            let result = repo.worktree_unlock(b"main".as_bstr());
            assert!(
                matches!(result, Err(gix::worktree::lock::UnlockError::NotFound { .. })),
                "main worktree should not be unlockable (not found in worktrees/), got {result:?}"
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
            let worktree_id = proxy.id()?.to_owned();

            // Verify not locked initially
            assert!(!proxy.is_locked());

            // Lock it
            repo.worktree_lock(worktree_id.as_bstr(), Default::default())?;

            // Verify locked via proxy
            let proxy = repo
                .worktree_proxy_by_id(worktree_id.as_bstr())?
                .expect("registered worktree");
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
            let worktree_id = proxy.id()?.to_owned();

            let lock_reason = "external drive";
            repo.worktree_lock(
                worktree_id.as_bstr(),
                gix::worktree::lock::Options {
                    reason: Some(lock_reason.into()),
                },
            )?;

            let proxy = repo
                .worktree_proxy_by_id(worktree_id.as_bstr())?
                .expect("registered worktree");
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
            let worktree_id = proxy.id()?.to_owned();

            // Try to lock again
            let result = repo.worktree_lock(worktree_id.as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::lock::Error::AlreadyLocked { .. })),
                "should fail because already locked, got {result:?}"
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
            let worktree_id = proxy.id()?.to_owned();

            assert!(proxy.is_locked());

            // Unlock it
            repo.worktree_unlock(worktree_id.as_bstr())?;

            // Verify unlocked
            let proxy = repo
                .worktree_proxy_by_id(worktree_id.as_bstr())?
                .expect("registered worktree");
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
            let worktree_id = proxy.id()?.to_owned();

            // Try to unlock when not locked
            let result = repo.worktree_unlock(worktree_id.as_bstr());
            assert!(
                matches!(result, Err(gix::worktree::lock::UnlockError::NotLocked { .. })),
                "should fail because not locked, got {result:?}"
            );
            Ok(())
        }

        #[test]
        fn lock_nonexistent_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            let result = repo.worktree_lock(b"nonexistent".as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::lock::Error::NotFound { .. })),
                "should fail with NotFound, got {result:?}"
            );
            Ok(())
        }

        #[test]
        fn unlock_nonexistent_fails() -> crate::Result {
            let (repo, _keep) = repo_rw()?;

            let result = repo.worktree_unlock(b"nonexistent".as_bstr());
            assert!(
                matches!(result, Err(gix::worktree::lock::UnlockError::NotFound { .. })),
                "should fail with NotFound, got {result:?}"
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
            let wt1_id = proxy1.id()?.to_owned();
            let proxy2 = repo.worktree_add(&wt2_path, options)?;
            let wt2_repo = proxy2.into_repo()?;

            // Lock wt1 from wt2
            wt2_repo.worktree_lock(wt1_id.as_bstr(), Default::default())?;

            // Verify it's locked
            let proxy1 = repo
                .worktree_proxy_by_id(wt1_id.as_bstr())?
                .expect("registered worktree");
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
            let wt_id = proxy.id()?.to_owned();
            let wt_repo = proxy.into_repo()?;

            // Try to lock it again from within itself
            let result = wt_repo.worktree_lock(wt_id.as_bstr(), Default::default());
            assert!(
                matches!(result, Err(gix::worktree::lock::Error::AlreadyLocked { .. })),
                "should fail because already locked, got {result:?}"
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
            let worktree_id = proxy.id()?.to_owned();
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
            let worktree_id = proxy.id()?.to_owned();
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
            let worktree_id = proxy.id()?.to_owned();

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
            let worktree_id = proxy.id()?.to_owned();
            let gitdir_path = proxy.git_dir().join("gitdir");

            // Move it
            repo.worktree_move(worktree_id.as_bstr(), &new_path)?;

            // Verify gitdir file points to new location
            let gitdir_content = std::fs::read_to_string(&gitdir_path)?;
            assert!(
                gitdir_content.contains("wt-move-gitdir-new"),
                "gitdir should point to new location, got: {gitdir_content}"
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
            let worktree_id = proxy.id()?.to_owned();

            // Try to move without force
            let result = repo.worktree_move(worktree_id.as_bstr(), &new_path);
            assert!(
                matches!(result, Err(gix::worktree::r#move::Error::Locked { .. })),
                "should fail because worktree is locked, got {result:?}"
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
            let worktree_id = proxy.id()?.to_owned();

            // Move with force level 2
            let move_options = gix::worktree::r#move::Options { force: 2 };
            repo.worktree_move_opts(worktree_id.as_bstr(), &new_path, move_options)?;

            assert!(!original_path.exists(), "original should be gone");
            assert!(new_path.exists(), "new path should exist");
            Ok(())
        }

        #[test]
        fn refuses_to_move_worktree_containing_submodules() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let original_path = repo.workdir().unwrap().parent().unwrap().join("wt-submodule-move");
            let new_path = repo.workdir().unwrap().parent().unwrap().join("wt-submodule-moved");

            let proxy = repo.worktree_add(
                &original_path,
                gix::worktree::add::Options {
                    detach: true,
                    ..Default::default()
                },
            )?;
            let worktree_id = proxy.id()?.to_owned();
            add_committed_submodule(&original_path)?;

            let result = repo.worktree_move(worktree_id.as_bstr(), &new_path);
            assert!(
                matches!(result, Err(gix::worktree::r#move::Error::ContainsSubmodules { .. })),
                "worktree move must refuse worktrees containing submodules, got {result:?}"
            );
            assert!(
                original_path.exists(),
                "refused move must leave the source worktree in place"
            );
            assert!(!new_path.exists(), "refused move must not create the destination");
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
            let worktree_id = proxy.id()?.to_owned();

            // Try to move to existing path
            let result = repo.worktree_move(worktree_id.as_bstr(), &existing_path);
            assert!(
                matches!(result, Err(gix::worktree::r#move::Error::DestinationExists { .. })),
                "should fail because destination exists, got {result:?}"
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
                "should fail with NotFound, got {result:?}"
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
            let worktree_id = proxy.id()?.to_owned();

            // Move to nested path (parent directories don't exist)
            repo.worktree_move(worktree_id.as_bstr(), &new_path)?;

            assert!(new_path.exists(), "worktree should be moved to nested path");
            Ok(())
        }

        #[test]
        fn refuses_to_move_directory_without_gitfile_backlink() -> crate::Result {
            let (repo, _keep) = repo_rw()?;
            let victim = repo.workdir().unwrap().parent().unwrap().join("not-a-worktree-move");
            let destination = repo.workdir().unwrap().parent().unwrap().join("moved-victim");
            std::fs::create_dir_all(&victim)?;
            std::fs::write(victim.join("important"), "keep me")?;
            let admin_dir = add_unbacklinked_worktree_entry(&repo, "unbacklinked-move", &victim)?;

            let result = repo.worktree_move(b"unbacklinked-move".as_bstr(), &destination);

            assert!(
                matches!(result, Err(gix::worktree::r#move::Error::InvalidWorktreeLink { .. })),
                "should refuse to move a directory without a matching .git backlink, got {result:?}"
            );
            assert!(victim.join("important").exists(), "unrelated directory must remain");
            assert!(
                !destination.exists(),
                "destination must not be created by a failed move"
            );
            assert!(admin_dir.exists(), "admin dir remains for repair or explicit cleanup");
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
            let repairs = repo.worktree_repair(std::slice::from_ref(&worktree_path))?;

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
            let repairs = repo.worktree_repair(std::slice::from_ref(&new_path))?;

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
                "should fail with InvalidPath, got {result:?}"
            );
            Ok(())
        }
    }
}
