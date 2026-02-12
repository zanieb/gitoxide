use gix::config::tree::{gitoxide, Branch, Core, Key};

use crate::{named_repo, repo_rw};

#[cfg(feature = "credentials")]
mod credential_helpers;

#[test]
fn commit_auto_rollback() -> crate::Result {
    let mut repo: gix::Repository = named_repo("make_basic_repo.sh")?;
    assert_eq!(repo.head_id()?.shorten()?.to_string(), "3189cd3");

    {
        let mut config = repo.config_snapshot_mut();
        config.set_raw_value(&Core::ABBREV, "4")?;
        let repo = config.commit_auto_rollback()?;
        assert_eq!(repo.head_id()?.shorten()?.to_string(), "3189");
    }

    assert_eq!(repo.head_id()?.shorten()?.to_string(), "3189cd3");

    let repo = {
        let mut config = repo.config_snapshot_mut();
        config.set_raw_value(&Core::ABBREV, "4")?;
        let mut repo = config.commit_auto_rollback()?;
        assert_eq!(repo.head_id()?.shorten()?.to_string(), "3189");
        // access to the mutable repo underneath
        repo.object_cache_size_if_unset(16 * 1024);
        repo.rollback()?
    };
    assert_eq!(repo.head_id()?.shorten()?.to_string(), "3189cd3");

    Ok(())
}

mod trusted_path {
    use crate::util::named_repo;

    #[test]
    fn optional_is_respected() -> crate::Result {
        let mut repo: gix::Repository = named_repo("make_basic_repo.sh")?;
        repo.config_snapshot_mut().set_raw_value(&"my.path", "does-not-exist")?;

        let actual = repo
            .config_snapshot()
            .trusted_path("my.path")
            .transpose()?
            .expect("is set");
        assert_eq!(
            actual.as_ref(),
            "does-not-exist",
            "the path isn't evaluated by default, and may not exist"
        );

        repo.config_snapshot_mut()
            .set_raw_value(&"my.path", ":(optional)does-not-exist")?;
        let actual = repo.config_snapshot().trusted_path("my.path").transpose()?;
        assert_eq!(actual, None, "non-existing paths aren't returned to the caller");
        Ok(())
    }
}

#[test]
fn snapshot_mut_commit_and_forget() -> crate::Result {
    let mut repo: gix::Repository = named_repo("make_basic_repo.sh")?;
    let repo = {
        let mut repo = repo.config_snapshot_mut();
        repo.set_value(&Core::ABBREV, "4")?;
        repo.commit()?
    };
    assert_eq!(repo.config_snapshot().integer("core.abbrev").expect("set"), 4);
    {
        let mut repo = repo.config_snapshot_mut();
        repo.set_raw_value(&Core::ABBREV, "8")?;
        repo.forget();
    }
    assert_eq!(repo.config_snapshot().integer("core.abbrev"), Some(4));
    Ok(())
}

#[test]
fn values_are_set_in_memory_only() {
    let mut repo = named_repo("make_config_repo.sh").unwrap();
    let repo_clone = repo.clone();
    let key = "hallo.welt";
    let key_subsection = "branch.main.merge";
    assert_eq!(repo.config_snapshot().boolean(key), None, "no value there just yet");
    assert_eq!(repo.config_snapshot().string(key_subsection), None);

    {
        let mut config = repo.config_snapshot_mut();
        config.set_raw_value(&"hallo.welt", "true").unwrap();
        config
            .set_subsection_value(&Branch::MERGE, "main", "refs/heads/foo")
            .unwrap();
    }

    assert_eq!(
        repo.config_snapshot().boolean(key),
        Some(true),
        "value was set and applied"
    );
    assert_eq!(
        repo.config_snapshot().string(key_subsection).as_deref(),
        Some("refs/heads/foo".into())
    );

    assert_eq!(
        repo_clone.config_snapshot().boolean(key),
        None,
        "values are not written back automatically nor are they shared between clones"
    );
    assert_eq!(repo_clone.config_snapshot().string(key_subsection), None);
}

#[test]
fn set_value_in_subsection() {
    let mut repo = named_repo("make_config_repo.sh").unwrap();
    {
        let mut config = repo.config_snapshot_mut();
        config
            .set_value(&gitoxide::Credentials::TERMINAL_PROMPT, "yes")
            .unwrap();
        assert_eq!(
            config
                .string(&*gitoxide::Credentials::TERMINAL_PROMPT.logical_name())
                .expect("just set")
                .as_ref(),
            "yes"
        );
    }
}

mod config_edit {
    use gix::config::tree::Core;

    use crate::util::repo_rw;

    #[test]
    fn commit_persists_changes_to_disk() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();
        let original_contents = std::fs::read_to_string(&config_path)?;

        {
            let mut guard = repo.config_edit()?;
            guard.set_value(&Core::ABBREV, "4")?;
            guard.commit()?;
        }

        let new_contents = std::fs::read_to_string(&config_path)?;
        assert_ne!(original_contents, new_contents, "file on disk was changed");
        assert!(
            new_contents.contains("abbrev"),
            "the new value should be present in the file"
        );
        assert_eq!(
            repo.config_snapshot().integer("core.abbrev").expect("set"),
            4,
            "in-memory config was also updated"
        );
        Ok(())
    }

    #[test]
    fn drop_also_persists_changes() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();
        let original_contents = std::fs::read_to_string(&config_path)?;

        {
            let mut guard = repo.config_edit()?;
            guard.set_value(&Core::ABBREV, "5")?;
            // drop without explicit commit
        }

        let new_contents = std::fs::read_to_string(&config_path)?;
        assert_ne!(original_contents, new_contents, "file on disk was changed on drop");
        assert!(new_contents.contains("abbrev"));
        assert_eq!(repo.config_snapshot().integer("core.abbrev").expect("set"), 5);
        Ok(())
    }

    #[test]
    fn forget_does_not_persist_changes() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();
        let original_contents = std::fs::read_to_string(&config_path)?;

        {
            let mut guard = repo.config_edit()?;
            guard.set_value(&Core::ABBREV, "4")?;
            guard.forget();
        }

        let after_contents = std::fs::read_to_string(&config_path)?;
        assert_eq!(original_contents, after_contents, "file on disk was NOT changed");
        assert_ne!(
            repo.config_snapshot().integer("core.abbrev"),
            Some(4),
            "in-memory config was NOT updated either"
        );
        Ok(())
    }

    #[test]
    fn set_subsection_value_persists() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();

        {
            let mut guard = repo.config_edit()?;
            guard.set_subsection_value(&gix::config::tree::Branch::MERGE, "main", "refs/heads/other")?;
            guard.commit()?;
        }

        let new_contents = std::fs::read_to_string(&config_path)?;
        assert!(
            new_contents.contains("refs/heads/other"),
            "subsection value should be in the file"
        );
        assert_eq!(
            repo.config_snapshot().string("branch.main.merge").as_deref(),
            Some("refs/heads/other".into()),
            "in-memory config was updated"
        );
        Ok(())
    }

    #[test]
    fn overwrite_existing_value() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();

        // The fixture has core.autocrlf = true. Overwrite it.
        {
            let mut guard = repo.config_edit()?;
            guard.set_raw_value(&Core::AUTO_CRLF, "false")?;
            guard.commit()?;
        }

        let new_contents = std::fs::read_to_string(&config_path)?;
        assert!(
            new_contents.contains("autocrlf = false"),
            "value should be updated in the file, got: {new_contents}"
        );
        assert_eq!(
            repo.config_snapshot().boolean("core.autocrlf"),
            Some(false),
            "in-memory config reflects the overwrite"
        );
        Ok(())
    }

    #[test]
    fn multiple_values_in_one_guard() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();

        {
            let mut guard = repo.config_edit()?;
            guard.set_value(&Core::ABBREV, "6")?;
            guard.set_raw_value(&Core::AUTO_CRLF, "false")?;
            guard.commit()?;
        }

        let new_contents = std::fs::read_to_string(&config_path)?;
        assert!(new_contents.contains("abbrev"), "abbrev should be in file");
        assert!(new_contents.contains("autocrlf = false"), "autocrlf should be updated");
        assert_eq!(repo.config_snapshot().integer("core.abbrev").expect("set"), 6);
        assert_eq!(repo.config_snapshot().boolean("core.autocrlf"), Some(false));
        Ok(())
    }

    #[test]
    fn sequential_edits_accumulate() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;

        // First edit
        {
            let mut guard = repo.config_edit()?;
            guard.set_value(&Core::ABBREV, "4")?;
            guard.commit()?;
        }
        assert_eq!(repo.config_snapshot().integer("core.abbrev").expect("set"), 4);

        // Second edit should not lose the first
        {
            let mut guard = repo.config_edit()?;
            guard.set_raw_value(&Core::AUTO_CRLF, "false")?;
            guard.commit()?;
        }
        assert_eq!(
            repo.config_snapshot().integer("core.abbrev").expect("still set"),
            4,
            "previous edit is still present after second commit"
        );
        assert_eq!(repo.config_snapshot().boolean("core.autocrlf"), Some(false));
        Ok(())
    }

    #[test]
    fn new_section_is_created() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();

        // Set a value in a section that doesn't exist yet via raw value.
        {
            let mut guard = repo.config_edit()?;
            guard.set_raw_value(&"custom.key", "myvalue")?;
            guard.commit()?;
        }

        let new_contents = std::fs::read_to_string(&config_path)?;
        assert!(
            new_contents.contains("[custom]"),
            "new section should be created in file, got: {new_contents}"
        );
        assert!(new_contents.contains("key = myvalue"));
        assert_eq!(
            repo.config_snapshot().string("custom.key").as_deref(),
            Some("myvalue".into()),
            "in-memory config has the new value"
        );
        Ok(())
    }

    #[test]
    fn lock_prevents_concurrent_edit() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;

        // Hold the first guard (which holds the lock)
        let mut guard1 = repo.config_edit()?;
        guard1.set_value(&Core::ABBREV, "4")?;

        // A second config_edit on the same repo can't be done because repo is already mutably borrowed.
        // But we can test that the lock file exists on disk.
        let config_path = guard1
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();
        let lock_path = config_path.with_extension("lock");
        assert!(lock_path.exists(), "lock file should exist while guard is held");

        guard1.commit()?;
        assert!(!lock_path.exists(), "lock file should be removed after commit");
        Ok(())
    }

    #[test]
    fn lock_file_removed_on_forget() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();
        let lock_path = config_path.with_extension("lock");

        {
            let mut guard = repo.config_edit()?;
            guard.set_value(&Core::ABBREV, "4")?;
            assert!(lock_path.exists(), "lock file exists while guard is held");
            guard.forget();
        }

        assert!(!lock_path.exists(), "lock file should be removed after forget");
        Ok(())
    }

    #[test]
    fn values_with_special_characters() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();

        {
            let mut guard = repo.config_edit()?;
            // Set a value containing spaces and special characters.
            guard.set_raw_value(&"test.path-value", "/some/path with spaces/file.txt")?;
            guard.set_raw_value(&"test.url-value", "https://example.com/repo.git")?;
            guard.commit()?;
        }

        let new_contents = std::fs::read_to_string(&config_path)?;
        assert!(new_contents.contains("path-value"), "path value should be in the file");

        assert_eq!(
            repo.config_snapshot().string("test.path-value").as_deref(),
            Some("/some/path with spaces/file.txt".into()),
            "value with spaces is preserved"
        );
        assert_eq!(
            repo.config_snapshot().string("test.url-value").as_deref(),
            Some("https://example.com/repo.git".into()),
            "URL value is preserved"
        );
        Ok(())
    }

    #[test]
    fn read_after_write_consistency() -> crate::Result {
        let (mut repo, _tmpdir) = repo_rw("make_config_repo.sh")?;

        // Write a value, then read it back through a fresh snapshot.
        {
            let mut guard = repo.config_edit()?;
            guard.set_value(&Core::ABBREV, "8")?;
            guard.commit()?;
        }

        // Reopen the repo to verify the value is actually persisted.
        let config_path = repo
            .config_snapshot()
            .meta()
            .path
            .as_deref()
            .expect("local config has a path")
            .to_owned();
        let repo_dir = config_path.parent().expect("has parent");
        let repo2 = gix::open_opts(repo_dir, gix::open::Options::isolated())?;
        assert_eq!(
            repo2.config_snapshot().integer("core.abbrev").expect("set"),
            8,
            "reopened repo sees the persisted value"
        );
        Ok(())
    }
}

#[test]
fn apply_cli_overrides() -> crate::Result {
    let mut repo = named_repo("make_config_repo.sh").unwrap();
    repo.config_snapshot_mut().append_config(
        [
            "a.b=c",
            "remote.origin.url = url",
            "implicit.bool-true",
            "implicit.bool-false = ",
        ],
        gix_config::Source::Cli,
    )?;

    let config = repo.config_snapshot();
    assert_eq!(config.string("a.b").expect("present").as_ref(), "c");
    assert_eq!(config.string("remote.origin.url").expect("present").as_ref(), "url");
    assert_eq!(
        config.string("implicit.bool-true"),
        None,
        "no keysep is interpreted as 'not present' as we don't make up values"
    );
    assert_eq!(
        config.string("implicit.bool-false").expect("present").as_ref(),
        "",
        "empty values are fine"
    );
    assert_eq!(
        config.boolean("implicit.bool-false"),
        Some(false),
        "empty values are boolean true"
    );
    assert_eq!(
        config.boolean("implicit.bool-true"),
        Some(true),
        "values without key-sep are true"
    );

    Ok(())
}

#[test]
fn reload_reloads_on_disk_changes() -> crate::Result {
    use std::io::Write;

    let (mut repo, _tmp) = repo_rw("make_config_repo.sh")?;
    assert_eq!(repo.config_snapshot().integer("core.abbrev"), None);

    let config_path = repo.git_dir().join("config");
    let mut config = std::fs::OpenOptions::new().append(true).open(config_path)?;
    writeln!(config, "\n[core]\n\tabbrev = 4")?;

    assert_eq!(repo.config_snapshot().integer("core.abbrev"), None);
    repo.reload()?;
    assert_eq!(repo.config_snapshot().integer("core.abbrev"), Some(4));
    Ok(())
}

#[test]
fn reload_discards_in_memory_only_changes() -> crate::Result {
    let mut repo = named_repo("make_config_repo.sh")?;

    repo.config_snapshot_mut().set_raw_value(&Core::ABBREV, "4")?;
    assert_eq!(repo.config_snapshot().integer("core.abbrev"), Some(4));

    repo.reload()?;
    assert_eq!(repo.config_snapshot().integer("core.abbrev"), None);
    Ok(())
}
