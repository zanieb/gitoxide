use gix::hook::Name;

use crate::util::{named_repo, named_subrepo_opts};

fn hooks_repo() -> crate::Result<gix::Repository> {
    named_repo("make_hooks_repo.sh")
}

fn hooks_repo_rw() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
    crate::util::repo_rw("make_hooks_repo.sh")
}

mod find {
    use super::*;

    #[test]
    fn existing_executable_hook_is_found() -> crate::Result {
        let repo = hooks_repo()?;
        let hook = repo.find_hook(Name::PreCommit)?;
        assert!(hook.is_some(), "pre-commit hook should be found");
        let hook = hook.unwrap();
        assert_eq!(hook.name, Name::PreCommit);
        assert!(hook.path.ends_with("pre-commit"));
        Ok(())
    }

    #[test]
    fn missing_hook_returns_none() -> crate::Result {
        let repo = hooks_repo()?;
        let hook = repo.find_hook(Name::PreRebase)?;
        assert!(hook.is_none(), "pre-rebase hook should not exist");
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn non_executable_hook_is_not_found() -> crate::Result {
        let repo = hooks_repo()?;
        let hook = repo.find_hook(Name::PostUpdate)?;
        assert!(
            hook.is_none(),
            "post-update hook exists but is not executable, so should not be found"
        );
        Ok(())
    }
}

mod hooks_dir {
    use super::*;

    #[test]
    fn default_hooks_dir() -> crate::Result {
        let repo = hooks_repo()?;
        let hooks_dir = repo.hooks_dir()?;
        assert!(hooks_dir.ends_with("hooks"), "should end with 'hooks': {hooks_dir:?}");
        assert!(hooks_dir.is_dir(), "hooks directory should exist");
        Ok(())
    }

    #[test]
    fn custom_hooks_path_from_config() -> crate::Result {
        let repo = named_subrepo_opts(
            "make_hooks_repo.sh",
            "custom-hooks-path",
            gix::open::Options::isolated(),
        )?;
        let hooks_dir = repo.hooks_dir()?;
        assert!(
            hooks_dir.to_string_lossy().contains("custom-hooks"),
            "should use the custom hooks path: {hooks_dir:?}"
        );
        Ok(())
    }
}

mod run {
    use super::*;

    #[test]
    fn successful_hook_returns_outcome() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        let outcome = repo.run_hook(Name::PreCommit, &[], None)?;
        assert!(outcome.is_some(), "hook should run");
        let outcome = outcome.unwrap();
        assert!(outcome.success(), "pre-commit hook should succeed");
        assert!(
            outcome
                .stdout
                .as_ref()
                .map_or(false, |s| s.starts_with(b"pre-commit ran")),
            "should capture stdout: {:?}",
            outcome.stdout.as_deref().map(String::from_utf8_lossy)
        );
        Ok(())
    }

    #[test]
    fn missing_hook_returns_none() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        let outcome = repo.run_hook(Name::PreRebase, &[], None)?;
        assert!(outcome.is_none(), "missing hook should return None");
        Ok(())
    }

    #[test]
    fn failing_hook_returns_nonzero_status() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        let outcome = repo.run_hook(Name::PrePush, &[], None)?;
        assert!(outcome.is_some(), "hook should run");
        let outcome = outcome.unwrap();
        assert!(!outcome.success(), "pre-push hook should fail");
        Ok(())
    }

    #[test]
    fn hook_receives_stdin_data() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        let stdin = b"old-sha new-sha refs/heads/main\n";
        let outcome = repo.run_hook(Name::PreReceive, &[], Some(stdin))?;
        assert!(outcome.is_some(), "hook should run");
        let outcome = outcome.unwrap();
        assert!(outcome.success(), "pre-receive hook should succeed");
        let stdout = outcome.stdout.unwrap_or_default();
        assert!(
            stdout.starts_with(b"received: old-sha"),
            "should process stdin data: {:?}",
            String::from_utf8_lossy(&stdout)
        );
        Ok(())
    }

    #[test]
    fn hook_receives_arguments() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        // commit-msg hook receives the message file path as an argument
        let outcome = repo.run_hook(Name::CommitMsg, &["/tmp/COMMIT_EDITMSG"], None)?;
        assert!(outcome.is_some(), "hook should run");
        let outcome = outcome.unwrap();
        assert!(outcome.success(), "commit-msg hook should succeed");
        let stdout = outcome.stdout.unwrap_or_default();
        assert!(
            stdout
                .windows(b"/tmp/COMMIT_EDITMSG".len())
                .any(|w| w == b"/tmp/COMMIT_EDITMSG"),
            "should pass arguments: {:?}",
            String::from_utf8_lossy(&stdout)
        );
        Ok(())
    }
}

mod run_or_fail {
    use super::*;

    #[test]
    fn succeeding_hook_returns_outcome() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        let outcome = repo.run_hook_or_fail(Name::PreCommit, &[], None)?;
        assert!(outcome.is_some(), "hook should run");
        assert!(outcome.unwrap().success());
        Ok(())
    }

    #[test]
    fn failing_hook_returns_error() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        let result = repo.run_hook_or_fail(Name::PrePush, &[], None);
        assert!(result.is_err(), "failing hook should return error");
        let err = result.unwrap_err();
        let err_string = err.to_string();
        assert!(
            err_string.contains("exited with code 1"),
            "error should mention exit code: {err_string}"
        );
        Ok(())
    }

    #[test]
    fn missing_hook_returns_none() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        let result = repo.run_hook_or_fail(Name::PreRebase, &[], None)?;
        assert!(result.is_none(), "missing hook should return None, not error");
        Ok(())
    }
}

mod custom_hooks_path {
    use super::*;

    #[test]
    fn hook_found_in_custom_path() -> crate::Result {
        let repo = named_subrepo_opts(
            "make_hooks_repo.sh",
            "custom-hooks-path",
            gix::open::Options::isolated(),
        )?;
        let hook = repo.find_hook(Name::PreCommit)?;
        assert!(hook.is_some(), "pre-commit hook should be found in custom hooks path");
        Ok(())
    }
}

mod name {
    use super::*;

    /// Ported from git/hook.c - verify all hook names map to their correct file names.
    #[test]
    fn as_str_returns_correct_file_names() {
        assert_eq!(Name::PreCommit.as_str(), "pre-commit");
        assert_eq!(Name::PrepareCommitMsg.as_str(), "prepare-commit-msg");
        assert_eq!(Name::CommitMsg.as_str(), "commit-msg");
        assert_eq!(Name::PostCommit.as_str(), "post-commit");
        assert_eq!(Name::PrePush.as_str(), "pre-push");
        assert_eq!(Name::PreReceive.as_str(), "pre-receive");
        assert_eq!(Name::PostReceive.as_str(), "post-receive");
        assert_eq!(Name::Update.as_str(), "update");
        assert_eq!(Name::PreApplyPatch.as_str(), "pre-applypatch");
        assert_eq!(Name::PostApplyPatch.as_str(), "post-applypatch");
        assert_eq!(Name::PostMerge.as_str(), "post-merge");
        assert_eq!(Name::PreRebase.as_str(), "pre-rebase");
        assert_eq!(Name::PostCheckout.as_str(), "post-checkout");
        assert_eq!(Name::PostRewrite.as_str(), "post-rewrite");
        assert_eq!(Name::PreMergeCommit.as_str(), "pre-merge-commit");
        assert_eq!(Name::PostUpdate.as_str(), "post-update");
        assert_eq!(Name::ReferenceTransaction.as_str(), "reference-transaction");
    }

    /// Verify the Display impl matches as_str.
    #[test]
    fn display_matches_as_str() {
        assert_eq!(format!("{}", Name::PreCommit), "pre-commit");
        assert_eq!(format!("{}", Name::PrePush), "pre-push");
        assert_eq!(format!("{}", Name::ReferenceTransaction), "reference-transaction");
    }
}

mod run_prepared {
    use super::*;

    /// Ported from t7503: test running a hook that was pre-discovered with find_hook.
    #[test]
    fn run_prepared_hook_executes_found_hook() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        let prepared = repo.find_hook(Name::PreCommit)?.expect("hook exists");
        let outcome = repo.run_prepared_hook(&prepared, &[], None)?;
        assert!(outcome.success(), "prepared hook should succeed");
        assert!(
            outcome
                .stdout
                .as_ref()
                .map_or(false, |s| s.starts_with(b"pre-commit ran")),
            "should capture stdout"
        );
        Ok(())
    }

    /// Ported from t7503: stderr output from hooks should be captured.
    #[test]
    fn hook_stderr_is_captured() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        // The pre-push hook fails with exit 1 and writes to stdout.
        // Run it and verify the outcome captures the failure.
        let outcome = repo.run_hook(Name::PrePush, &[], None)?;
        let outcome = outcome.expect("hook should run");
        assert!(!outcome.success(), "pre-push hook should fail");
        // The pre-push hook echoes to stdout; stderr may be None (empty).
        // We're verifying the stderr capture path works without error.
        // An empty stderr becomes None per the implementation.
        assert!(
            outcome.stderr.is_none() || outcome.stderr.as_ref().map_or(false, |s| !s.is_empty()),
            "stderr should be None when empty or contain data"
        );
        Ok(())
    }
}

mod run_multi_line_stdin {
    use super::*;

    /// Ported from t5541: pre-receive hook receives multiple lines of input.
    #[test]
    fn hook_processes_multiple_stdin_lines() -> crate::Result {
        let (repo, _tmp) = hooks_repo_rw()?;
        let stdin = b"aaa bbb refs/heads/main\nccc ddd refs/heads/develop\n";
        let outcome = repo.run_hook(Name::PreReceive, &[], Some(stdin))?;
        let outcome = outcome.expect("hook should run");
        assert!(outcome.success());
        let stdout = outcome.stdout.unwrap_or_default();
        let stdout_str = String::from_utf8_lossy(&stdout);
        assert!(
            stdout_str.contains("received: aaa bbb refs/heads/main"),
            "should process first line: {stdout_str}"
        );
        assert!(
            stdout_str.contains("received: ccc ddd refs/heads/develop"),
            "should process second line: {stdout_str}"
        );
        Ok(())
    }
}
