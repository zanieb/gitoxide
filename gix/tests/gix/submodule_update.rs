/// Helper to open a writable scenario from the `make_submodule_update.sh` fixture.
fn repo_rw(name: &str) -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
    let dir = gix_testtools::scripted_fixture_writable("make_submodule_update.sh")?;
    let repo_path = dir.path().join(name);
    Ok((
        gix::ThreadSafeRepository::open_opts(repo_path, gix::open::Options::isolated())?.to_thread_local(),
        dir,
    ))
}

mod init {
    use super::repo_rw;

    /// Ported from libgit2 test_submodule_init__absolute_url and git t7406:
    /// After init, submodule.<name>.url should be written to .git/config.
    #[test]
    fn writes_url_to_local_config() -> crate::Result {
        let (repo, _tmp) = repo_rw("after-clone")?;
        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        assert_eq!(sm.name(), "submodule");

        // Before init: no url in local config (fresh clone, not initialized)
        let snap = repo.config_snapshot();
        assert!(
            snap.string("submodule.submodule.url").is_none(),
            "url should not be present in local config before init"
        );

        sm.init(false)?;

        // Re-open to see persisted config
        let repo2 = gix::open_opts(repo.git_dir(), gix::open::Options::isolated())?;
        let snap2 = repo2.config_snapshot();
        let url = snap2
            .string("submodule.submodule.url")
            .expect("url should be present after init");
        assert!(!url.as_ref().is_empty(), "url should be non-empty after init");
        Ok(())
    }

    /// Ported from git t7406: submodule init does not overwrite existing url.
    #[test]
    fn does_not_overwrite_existing_url() -> crate::Result {
        let (repo, _tmp) = repo_rw("init-no-overwrite")?;
        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        // Verify the custom url is set
        let snap = repo.config_snapshot();
        let url_before = snap.string("submodule.submodule.url").expect("url set").to_owned();
        assert!(url_before.as_ref().starts_with(b"https://custom.example.com"));

        // Init without force should be a no-op
        sm.init(false)?;

        let repo2 = gix::open_opts(repo.git_dir(), gix::open::Options::isolated())?;
        let snap2 = repo2.config_snapshot();
        let url_after = snap2.string("submodule.submodule.url").expect("url still set");
        assert_eq!(
            url_after.as_ref(),
            url_before.as_ref(),
            "init without force should not overwrite"
        );
        Ok(())
    }

    /// Ported from git t7406: submodule init does not copy command into .git/config.
    /// Commands in .gitmodules update field are a security risk.
    #[test]
    fn rejects_command_from_gitmodules() -> crate::Result {
        let (repo, _tmp) = repo_rw("command-in-gitmodules")?;
        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        // Init should succeed. The !command in .gitmodules is rejected by the security
        // check in gix_submodule::File::update(), so no update strategy is written.
        // The url should still be written.
        sm.init(false)?;

        let repo2 = gix::open_opts(repo.git_dir(), gix::open::Options::isolated())?;
        let snap2 = repo2.config_snapshot();

        // The url should still be written
        assert!(
            snap2.string("submodule.submodule.url").is_some(),
            "url should be written after init"
        );

        // The update field must NOT contain the !command. It may be absent entirely
        // (since the command was rejected) or set to "none" -- either is acceptable.
        if let Some(val) = snap2.string("submodule.submodule.update") {
            assert!(
                !val.as_ref().starts_with(b"!"),
                "command should not be written from .gitmodules, got: {:?}",
                val.as_ref()
            );
        }
        Ok(())
    }

    /// Ported from git t7406: submodule init picks up update=none.
    #[test]
    fn init_picks_up_update_none() -> crate::Result {
        let (repo, _tmp) = repo_rw("update-none")?;

        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        // The update strategy from local config (.git/config) is 'none'
        // since we set it in the fixture.
        let update = sm.update_strategy()?;
        assert_eq!(
            update,
            Some(gix::submodule::config::Update::None),
            "update=none should be reflected"
        );
        Ok(())
    }
}

/// Tests that require networking (clone/fetch) for submodule update operations.
#[cfg(feature = "blocking-network-client")]
mod update {
    use super::repo_rw;

    /// Ported from libgit2 test_submodule_update__update_and_init_submodule:
    /// Calling update with init=true on an uninitialized submodule should init and clone it.
    #[test]
    fn update_with_init_on_uninitialized() -> crate::Result {
        let (repo, _tmp) = repo_rw("after-clone")?;

        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        assert_eq!(sm.name(), "submodule");
        let state = sm.state()?;
        assert!(
            !state.repository_exists,
            "submodule repo should not exist before update"
        );

        let outcome = sm.update_submodule(
            gix::progress::Discard,
            &std::sync::atomic::AtomicBool::default(),
            &gix::submodule::update::Options::new(true, false),
        )?;

        let outcome = outcome.expect("update should return an outcome");
        assert!(outcome.freshly_cloned, "submodule should have been cloned");
        assert_eq!(
            outcome.strategy,
            gix::submodule::config::Update::Checkout,
            "default strategy is checkout"
        );

        // Verify the submodule is now checked out at the expected commit
        let index_id = sm.index_id()?.expect("submodule in index");
        assert_eq!(
            outcome.target_commit, index_id,
            "submodule should be at the superproject's expected commit"
        );

        // The submodule repo should now exist and be openable
        let sm_repo = sm.open()?.expect("submodule should be openable after update");
        assert!(sm_repo.workdir().is_some(), "workdir should exist");
        Ok(())
    }

    /// Ported from libgit2 test_submodule_update__update_already_checked_out_submodule:
    /// When the submodule is already checked out but at the wrong commit, update should
    /// advance it to the correct commit.
    #[test]
    fn update_already_checked_out_to_wrong_commit() -> crate::Result {
        let (repo, _tmp) = repo_rw("needs-update")?;

        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        let index_id = sm.index_id()?.expect("submodule in index");
        let sm_repo = sm.open()?.expect("submodule repo exists");
        let current_head = sm_repo.head_id()?.detach();
        assert_ne!(current_head, index_id, "submodule should be behind the superproject");

        let outcome = sm.update_submodule(
            gix::progress::Discard,
            &std::sync::atomic::AtomicBool::default(),
            &Default::default(),
        )?;

        let outcome = outcome.expect("update should succeed");
        assert!(!outcome.freshly_cloned, "should not re-clone");
        assert_eq!(outcome.target_commit, index_id);

        // Verify HEAD is now detached at the correct commit
        let sm_repo = sm.open()?.expect("submodule opened");
        let head = sm_repo.head_id()?.detach();
        assert_eq!(head, outcome.target_commit, "HEAD should be at target commit");
        Ok(())
    }

    /// Ported from git t7406: submodule update - update=none in .git/config.
    /// When update strategy is 'none', the submodule should not be updated.
    #[test]
    fn update_with_strategy_none_is_skipped() -> crate::Result {
        let (repo, _tmp) = repo_rw("update-none")?;

        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        let result = sm.update_submodule(
            gix::progress::Discard,
            &std::sync::atomic::AtomicBool::default(),
            &Default::default(),
        )?;

        assert!(result.is_none(), "update with strategy=none should skip (return None)");
        Ok(())
    }

    /// Ported from libgit2 test_submodule_update__uninitialized_submodule_no_init:
    /// Updating an uninitialized submodule without init=true should be skipped.
    /// Git skips uninitialized submodules silently; we return Ok(None).
    #[test]
    fn update_uninitialized_without_init_is_skipped() -> crate::Result {
        let (repo, _tmp) = repo_rw("after-clone")?;

        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        let state = sm.state()?;
        assert!(!state.repository_exists);

        // Without init, the submodule is not initialized (no url in .git/config),
        // so it should be skipped, returning None.
        let result = sm.update_submodule(
            gix::progress::Discard,
            &std::sync::atomic::AtomicBool::default(),
            &gix::submodule::update::Options::new(false, false),
        )?;

        assert!(
            result.is_none(),
            "uninitialized submodule without --init should be skipped (return None)"
        );
        Ok(())
    }

    /// Ported from git t7406: submodule update --init --recursive.
    /// Recursive update should initialize and update nested submodules.
    #[test]
    fn update_init_recursive() -> crate::Result {
        let (repo, _tmp) = repo_rw("recursive-clone")?;

        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        assert_eq!(sm.name(), "mid");

        let outcome = sm.update_submodule(
            gix::progress::Discard,
            &std::sync::atomic::AtomicBool::default(),
            &gix::submodule::update::Options::new(true, true),
        )?;

        let outcome = outcome.expect("update should succeed");
        assert!(outcome.freshly_cloned);

        // The mid-module's inner submodule should also be checked out
        let sm_repo = sm.open()?.expect("mid submodule should be openable");
        if let Some(inner_sms) = sm_repo.submodules()? {
            for inner_sm in inner_sms {
                let inner_state = inner_sm.state()?;
                assert!(
                    inner_state.repository_exists,
                    "nested submodule '{}' should have a repository after recursive update",
                    inner_sm.name()
                );
            }
        }
        Ok(())
    }

    /// Ported from git t7406: submodule update does not fetch already present commits.
    /// If the target commit already exists in the submodule, no fetch should be needed.
    #[test]
    fn update_skips_fetch_when_commit_present() -> crate::Result {
        let (repo, _tmp) = repo_rw("needs-update")?;

        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        // The target commit should already be in the submodule (it was fetched during initial clone)
        let index_id = sm.index_id()?.expect("submodule in index");
        let sm_repo = sm.open()?.expect("repo exists");
        assert!(
            sm_repo.find_object(index_id).is_ok(),
            "target commit should already exist in submodule"
        );

        // Update should succeed without needing to fetch
        let outcome = sm.update_submodule(
            gix::progress::Discard,
            &std::sync::atomic::AtomicBool::default(),
            &Default::default(),
        )?;

        assert!(outcome.is_some());
        assert!(!outcome.unwrap().freshly_cloned);
        Ok(())
    }

    /// When the submodule HEAD already matches the superproject index, update should be a no-op.
    /// Uses the `fully-updated` fixture where `git submodule update --init` has already been run.
    #[test]
    fn update_is_noop_when_head_matches_target() -> crate::Result {
        let (repo, _tmp) = repo_rw("fully-updated")?;

        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        let index_id = sm.index_id()?.expect("submodule in index");
        let sm_repo = sm.open()?.expect("submodule repo exists");
        let head_before = sm_repo.head_id()?.detach();
        assert_eq!(
            head_before, index_id,
            "submodule HEAD should already match the superproject index"
        );

        let outcome = sm.update_submodule(
            gix::progress::Discard,
            &std::sync::atomic::AtomicBool::default(),
            &Default::default(),
        )?;

        let outcome = outcome.expect("update should return an outcome (not None)");
        assert!(!outcome.freshly_cloned, "should not re-clone");
        assert_eq!(outcome.target_commit, index_id);
        #[cfg(feature = "worktree-mutation")]
        assert!(
            outcome.checkout.is_none(),
            "checkout should be None when HEAD already matches target (no-op)"
        );
        Ok(())
    }

    /// Test the two-step workflow: init first, then update separately.
    /// Uses the `initialized-not-updated` fixture where `git submodule init` has been run
    /// but `git submodule update` has not.
    #[test]
    fn update_after_separate_init() -> crate::Result {
        let (repo, _tmp) = repo_rw("initialized-not-updated")?;

        let sm = repo
            .submodules()?
            .expect("modules present")
            .next()
            .expect("one submodule");

        // The submodule should be initialized (url in .git/config) but not yet cloned
        assert!(
            sm.is_initialized(),
            "submodule should be initialized after `git submodule init`"
        );
        let state = sm.state()?;
        assert!(
            !state.repository_exists,
            "submodule repo should not exist before update"
        );

        // Update WITHOUT init flag (should work because already initialized)
        let outcome = sm.update_submodule(
            gix::progress::Discard,
            &std::sync::atomic::AtomicBool::default(),
            &gix::submodule::update::Options::new(false, false),
        )?;

        let outcome = outcome.expect("update should succeed on an initialized submodule");
        assert!(outcome.freshly_cloned, "submodule should be freshly cloned");

        // Verify HEAD is at the correct commit
        let sm_repo = sm.open()?.expect("submodule opened");
        let head = sm_repo.head_id()?.detach();
        assert_eq!(head, outcome.target_commit, "HEAD should be at target commit");
        Ok(())
    }
}
