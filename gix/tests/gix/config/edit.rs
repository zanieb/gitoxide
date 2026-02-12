fn repo_rw_config() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
    let dir = gix_testtools::scripted_fixture_writable("make_config_edit_repo.sh")?;
    let repo_path = dir.path().join("repo");
    let repo = gix::open_opts(&repo_path, crate::restricted())?;
    Ok((repo, dir))
}

fn reopen(dir: &std::path::Path) -> crate::Result<gix::Repository> {
    let repo_path = dir.join("repo");
    Ok(gix::open_opts(&repo_path, crate::restricted())?)
}

#[test]
fn set_value_then_commit_persists_to_disk() -> crate::Result {
    let (mut repo, dir) = repo_rw_config()?;

    {
        let mut guard = repo.config_edit()?;
        guard.set_raw_value_by("core", None, "abbrev", "7").expect("set value");
        guard.commit()?;
    }

    let repo2 = reopen(dir.path())?;
    let snap = repo2.config_snapshot();
    let value = snap.string("core.abbrev").expect("abbrev should exist");
    assert_eq!(value.as_ref(), "7", "committed value should persist on disk");
    Ok(())
}

#[test]
fn forget_does_not_persist() -> crate::Result {
    let (mut repo, dir) = repo_rw_config()?;

    {
        let mut guard = repo.config_edit()?;
        guard.set_raw_value_by("core", None, "abbrev", "3").expect("set value");
        guard.forget();
    }

    let repo2 = reopen(dir.path())?;
    let snap = repo2.config_snapshot();
    let value = snap.string("core.abbrev").expect("abbrev should still exist");
    assert_ne!(value.as_ref(), "3", "forgotten value should NOT persist on disk");
    Ok(())
}

#[test]
fn drop_commits_automatically() -> crate::Result {
    let (mut repo, dir) = repo_rw_config()?;

    {
        let mut guard = repo.config_edit()?;
        guard.set_raw_value_by("core", None, "abbrev", "5").expect("set value");
        // guard is dropped here without explicit commit — should auto-commit
    }

    let repo2 = reopen(dir.path())?;
    let snap = repo2.config_snapshot();
    let value = snap.string("core.abbrev").expect("abbrev should exist");
    assert_eq!(value.as_ref(), "5", "drop should auto-commit the value");
    Ok(())
}

#[test]
fn set_subsection_value_persists() -> crate::Result {
    let (mut repo, dir) = repo_rw_config()?;

    {
        let mut guard = repo.config_edit()?;
        guard
            .set_raw_value_by("remote", Some("origin".into()), "url", "https://example.com/repo.git")
            .expect("set subsection value");
        guard.commit()?;
    }

    let repo2 = reopen(dir.path())?;
    let snap = repo2.config_snapshot();
    let value = snap
        .string("remote.origin.url")
        .expect("remote.origin.url should exist");
    assert_eq!(
        value.as_ref(),
        "https://example.com/repo.git",
        "subsection value should persist"
    );
    Ok(())
}

#[test]
fn roundtrip_preserves_existing_values() -> crate::Result {
    let (mut repo, dir) = repo_rw_config()?;

    // Read the original user.email value before editing
    let original_email = {
        let snap = repo.config_snapshot();
        snap.string("user.email").map(|v| v.to_string())
    };

    {
        let mut guard = repo.config_edit()?;
        guard
            .set_raw_value_by("custom", None, "key", "value")
            .expect("set new value");
        guard.commit()?;
    }

    let repo2 = reopen(dir.path())?;
    let snap = repo2.config_snapshot();

    // The new value should exist
    let custom = snap.string("custom.key").expect("custom.key should exist");
    assert_eq!(custom.as_ref(), "value");

    // Pre-existing values should be unchanged
    let email = snap.string("user.email").map(|v| v.to_string());
    assert_eq!(email, original_email, "pre-existing user.email should be preserved");
    Ok(())
}

#[test]
fn concurrent_lock_fails() -> crate::Result {
    let (mut repo, dir) = repo_rw_config()?;

    // Acquire first lock
    let _guard = repo.config_edit()?;

    // Try to acquire second lock via a separate repo handle to the same path
    let mut repo2 = reopen(dir.path())?;
    let result = repo2.config_edit();

    assert!(
        result.is_err(),
        "second config_edit() should fail while first lock is held"
    );
    Ok(())
}

#[test]
fn multiple_values_in_single_session() -> crate::Result {
    let (mut repo, dir) = repo_rw_config()?;

    {
        let mut guard = repo.config_edit()?;
        guard.set_raw_value_by("core", None, "abbrev", "9").expect("set abbrev");
        guard
            .set_raw_value_by("core", None, "autocrlf", "false")
            .expect("set autocrlf");
        guard
            .set_raw_value_by("custom", None, "key", "hello")
            .expect("set custom key");
        guard.commit()?;
    }

    let repo2 = reopen(dir.path())?;
    let snap = repo2.config_snapshot();
    assert_eq!(snap.string("core.abbrev").unwrap().as_ref(), "9");
    assert_eq!(snap.string("core.autocrlf").unwrap().as_ref(), "false");
    assert_eq!(snap.string("custom.key").unwrap().as_ref(), "hello");
    Ok(())
}
