#[cfg(feature = "blocking-network-client")]
mod blocking_io {
    use std::process::Command;
    use std::sync::atomic::AtomicBool;

    use gix::remote::Direction;
    use gix_testtools::tempfile;

    /// Helper: run a C `git` command in the given directory and return stdout as a trimmed string.
    fn git_in(dir: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("failed to execute git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("valid utf8").trim().to_string()
    }

    /// Helper: run a C `git` command and check whether it succeeds (true) or fails (false).
    fn git_in_ok(dir: &std::path::Path, args: &[&str]) -> bool {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("failed to execute git")
            .status
            .success()
    }

    /// Set up writable push test repos.
    /// Returns (working_repo, bare_repo_path, _tempdir).
    fn setup_push_repos() -> gix_testtools::Result<(gix::Repository, std::path::PathBuf, tempfile::TempDir)> {
        let dir = gix_testtools::scripted_fixture_writable("make_push_repos.sh")?;
        let working_path = dir.path().join("working");
        let bare_path = dir.path().join("remote.git");
        let repo = gix::open_opts(&working_path, crate::restricted())?;
        Ok((repo, bare_path, dir))
    }

    /// Push using gix, then verify with C `git` that the remote ref matches the expected oid.
    fn verify_push_result(bare_path: &std::path::Path, ref_name: &str, expected_oid: &str) {
        let actual = git_in(bare_path, &["rev-parse", "--verify", ref_name]);
        assert_eq!(
            actual, expected_oid,
            "C git rev-parse of {ref_name} in bare repo does not match expected oid"
        );
        // Also verify fsck passes on the bare repo
        assert!(
            git_in_ok(bare_path, &["fsck", "--full"]),
            "git fsck failed on bare repo after push"
        );
    }

    /// Verify that a ref does NOT exist in the bare repo.
    fn verify_ref_absent(bare_path: &std::path::Path, ref_name: &str) {
        assert!(
            !git_in_ok(bare_path, &["rev-parse", "--verify", ref_name]),
            "Expected ref {ref_name} to be absent in bare repo, but it exists"
        );
    }

    /// Perform a push operation using gix with given refspecs.
    fn do_push(
        repo: &gix::Repository,
        bare_url: &str,
        refspecs: &[&str],
    ) -> gix_testtools::Result<gix::remote::push::Outcome> {
        let remote = repo
            .remote_at(bare_url)?
            .with_refspecs(refspecs.iter().copied(), Direction::Push)?;
        let outcome = remote
            .connect(Direction::Push)?
            .prepare_push(gix::progress::Discard, Default::default())?
            .send(gix::progress::Discard, &AtomicBool::default())?;
        Ok(outcome)
    }

    // -----------------------------------------------------------------------
    // Tests ported from t5516-fetch-push.sh
    // -----------------------------------------------------------------------

    #[test]
    fn push_single_branch_to_bare_repo() -> gix_testtools::Result {
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());
        let head_oid = repo.head_id()?.to_string();

        do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/main"])?;

        verify_push_result(&bare_path, "refs/heads/main", &head_oid);
        Ok(())
    }

    #[test]
    fn push_with_explicit_refspec() -> gix_testtools::Result {
        // Mirrors: 'push without wildcard'
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());
        let head_oid = repo.head_id()?.to_string();

        do_push(&repo, &bare_url, &["refs/heads/main:refs/remotes/origin/main"])?;

        verify_push_result(&bare_path, "refs/remotes/origin/main", &head_oid);
        // Also verify that C git can read the repo properly
        let log_output = git_in(&bare_path, &["log", "--oneline", "refs/remotes/origin/main"]);
        assert!(
            !log_output.is_empty(),
            "C git should be able to read pushed ref history"
        );
        Ok(())
    }

    #[test]
    fn push_feature_branch() -> gix_testtools::Result {
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());

        let feature_ref = repo.find_reference("refs/heads/feature")?;
        let feature_oid = feature_ref.id().to_string();

        do_push(&repo, &bare_url, &["refs/heads/feature:refs/heads/feature"])?;

        verify_push_result(&bare_path, "refs/heads/feature", &feature_oid);
        Ok(())
    }

    #[test]
    fn push_creates_new_remote_ref() -> gix_testtools::Result {
        // Mirrors: 'push head with non-existent, incomplete dest'
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());
        let head_oid = repo.head_id()?.to_string();

        // Push main to a brand new ref that doesn't exist yet on the remote
        do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/new-branch"])?;

        verify_push_result(&bare_path, "refs/heads/new-branch", &head_oid);
        Ok(())
    }

    #[test]
    fn push_tag() -> gix_testtools::Result {
        // Mirrors: 'push tag with non-existent, incomplete dest'
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());

        let tag_ref = repo.find_reference("refs/tags/v1.0")?;
        let tag_oid = tag_ref.id().to_string();

        do_push(&repo, &bare_url, &["refs/tags/v1.0:refs/tags/v1.0"])?;

        verify_push_result(&bare_path, "refs/tags/v1.0", &tag_oid);
        Ok(())
    }

    #[test]
    fn push_multiple_refs_at_once() -> gix_testtools::Result {
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());

        let main_oid = repo.head_id()?.to_string();
        let feature_ref = repo.find_reference("refs/heads/feature")?;
        let feature_oid = feature_ref.id().to_string();
        let tag_ref = repo.find_reference("refs/tags/v1.0")?;
        let tag_oid = tag_ref.id().to_string();

        do_push(
            &repo,
            &bare_url,
            &[
                "refs/heads/main:refs/heads/main",
                "refs/heads/feature:refs/heads/feature",
                "refs/tags/v1.0:refs/tags/v1.0",
            ],
        )?;

        verify_push_result(&bare_path, "refs/heads/main", &main_oid);
        verify_push_result(&bare_path, "refs/heads/feature", &feature_oid);
        verify_push_result(&bare_path, "refs/tags/v1.0", &tag_oid);
        Ok(())
    }

    #[test]
    fn push_delete_remote_ref() -> gix_testtools::Result {
        // Mirrors: 'allow deleting an invalid remote ref'
        // First push a branch, then delete it.
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());

        // Push feature branch first
        do_push(&repo, &bare_url, &["refs/heads/feature:refs/heads/feature"])?;
        verify_push_result(
            &bare_path,
            "refs/heads/feature",
            &repo.find_reference("refs/heads/feature")?.id().to_string(),
        );

        // Now delete it by pushing an empty source (null oid -> remote ref).
        // The refspec for deletion is ":refs/heads/feature".
        let remote = repo
            .remote_at(bare_url.as_str())?
            .with_refspecs([":refs/heads/feature"].iter().copied(), Direction::Push)?;
        let outcome = remote
            .connect(Direction::Push)?
            .prepare_push(gix::progress::Discard, Default::default())?
            .send(gix::progress::Discard, &AtomicBool::default())?;

        assert!(outcome.unpack_ok, "unpack should succeed for deletion");

        verify_ref_absent(&bare_path, "refs/heads/feature");
        Ok(())
    }

    #[test]
    fn push_force_non_fast_forward() -> gix_testtools::Result {
        // Mirrors: 'push --force with matching heads'
        // Push main, then create a divergent commit and force-push it.
        let (repo, bare_path, dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());
        let original_oid = repo.head_id()?.to_string();

        // First, normal push
        do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/main"])?;
        verify_push_result(&bare_path, "refs/heads/main", &original_oid);

        // Create a divergent commit in the working repo using C git
        // (amend the last commit to create a non-fast-forward situation)
        let working_path = dir.path().join("working");
        git_in(
            &working_path,
            &["commit", "--amend", "-m", "amended commit", "--allow-empty"],
        );
        // Re-open repo to pick up changes
        let repo = gix::open_opts(&working_path, crate::restricted())?;
        let amended_oid = repo.head_id()?.to_string();
        assert_ne!(original_oid, amended_oid, "amended commit should have different oid");

        // Force push using + prefix in refspec
        do_push(&repo, &bare_url, &["+refs/heads/main:refs/heads/main"])?;

        verify_push_result(&bare_path, "refs/heads/main", &amended_oid);
        Ok(())
    }

    #[test]
    fn push_reject_non_fast_forward_without_force() -> gix_testtools::Result {
        // Mirrors: 'failed (non-fast-forward) push with matching heads'
        let (repo, bare_path, dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());
        let original_oid = repo.head_id()?.to_string();

        // First, normal push
        do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/main"])?;
        verify_push_result(&bare_path, "refs/heads/main", &original_oid);

        // Configure bare repo to reject non-fast-forward updates, matching the
        // behavior of hosted Git services (GitHub, GitLab) and C Git's
        // receive.denyNonFastForwards setting.
        git_in(&bare_path, &["config", "receive.denyNonFastForwards", "true"]);

        // Amend to create divergence
        let working_path = dir.path().join("working");
        git_in(
            &working_path,
            &["commit", "--amend", "-m", "amended commit", "--allow-empty"],
        );
        let repo = gix::open_opts(&working_path, crate::restricted())?;

        // Non-force push should fail because the remote denies non-fast-forward updates.
        let result = do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/main"]);

        match result {
            Ok(outcome) => {
                // If the push "succeeded" at the protocol level, check that the
                // server rejected the update.
                let rejected = outcome
                    .updates
                    .iter()
                    .any(|s| matches!(s, gix::remote::push::RefUpdateStatus::Ng { .. }));
                assert!(
                    rejected,
                    "non-fast-forward push without force should be rejected by the remote"
                );
                // The remote should still have the original oid
                verify_push_result(&bare_path, "refs/heads/main", &original_oid);
            }
            Err(_) => {
                // Protocol-level error is also acceptable -- the push was rejected.
                // Verify the remote ref is unchanged.
                verify_push_result(&bare_path, "refs/heads/main", &original_oid);
            }
        }

        Ok(())
    }

    #[test]
    fn push_with_dry_run_does_not_modify_remote() -> gix_testtools::Result {
        // Mirrors: 'push with dry-run'
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());

        let remote = repo
            .remote_at(bare_url.as_str())?
            .with_refspecs(["refs/heads/main:refs/heads/main"].iter().copied(), Direction::Push)?;
        let _outcome = remote
            .connect(Direction::Push)?
            .prepare_push(gix::progress::Discard, Default::default())?
            .with_dry_run(true)
            .send(gix::progress::Discard, &AtomicBool::default())?;

        // After dry-run, the remote should NOT have the ref
        verify_ref_absent(&bare_path, "refs/heads/main");
        Ok(())
    }

    #[test]
    fn push_two_tags() -> gix_testtools::Result {
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());

        let v1_ref = repo.find_reference("refs/tags/v1.0")?;
        let v1_oid = v1_ref.id().to_string();
        let v2_ref = repo.find_reference("refs/tags/v2.0")?;
        let v2_oid = v2_ref.id().to_string();

        do_push(
            &repo,
            &bare_url,
            &["refs/tags/v1.0:refs/tags/v1.0", "refs/tags/v2.0:refs/tags/v2.0"],
        )?;

        verify_push_result(&bare_path, "refs/tags/v1.0", &v1_oid);
        verify_push_result(&bare_path, "refs/tags/v2.0", &v2_oid);

        // Verify C git can list the tags
        let tags = git_in(&bare_path, &["tag", "-l"]);
        assert!(tags.contains("v1.0"), "C git should list v1.0 tag");
        assert!(tags.contains("v2.0"), "C git should list v2.0 tag");
        Ok(())
    }

    #[test]
    fn push_then_c_git_can_clone() -> gix_testtools::Result {
        // End-to-end interop: push with gix, then clone with C git
        let (repo, bare_path, dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());
        let head_oid = repo.head_id()?.to_string();

        // Push main and a tag
        do_push(
            &repo,
            &bare_url,
            &["refs/heads/main:refs/heads/main", "refs/tags/v1.0:refs/tags/v1.0"],
        )?;

        // Clone with C git into a new directory
        let clone_path = dir.path().join("clone_test");
        let status = Command::new("git")
            .args(["clone", &bare_url, clone_path.to_str().expect("valid path")])
            .output()
            .expect("failed to execute git clone");
        assert!(
            status.status.success(),
            "C git clone of gix-pushed bare repo failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );

        // Verify the cloned repo has the correct HEAD
        let cloned_head = git_in(&clone_path, &["rev-parse", "HEAD"]);
        assert_eq!(cloned_head, head_oid, "cloned HEAD should match pushed HEAD");

        // Verify the tag is present in the clone
        let tag_oid = git_in(&clone_path, &["rev-parse", "v1.0"]);
        let expected_tag_oid = repo.find_reference("refs/tags/v1.0")?.id().to_string();
        assert_eq!(tag_oid, expected_tag_oid, "cloned tag v1.0 should match pushed tag");

        Ok(())
    }

    #[test]
    fn push_incremental_updates() -> gix_testtools::Result {
        // Push once, add more commits, push again -- verifies incremental pack generation
        let (repo, bare_path, dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());
        let first_oid = repo.head_id()?.to_string();

        // First push
        do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/main"])?;
        verify_push_result(&bare_path, "refs/heads/main", &first_oid);

        // Create additional commit using C git
        let working_path = dir.path().join("working");
        git_in(&working_path, &["checkout", "main"]);
        std::fs::write(working_path.join("file4.txt"), "fourth").expect("write file4");
        git_in(&working_path, &["add", "file4.txt"]);
        git_in(&working_path, &["commit", "-m", "fourth commit"]);
        let second_oid = git_in(&working_path, &["rev-parse", "HEAD"]);

        // Re-open and push again
        let repo = gix::open_opts(&working_path, crate::restricted())?;
        do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/main"])?;
        verify_push_result(&bare_path, "refs/heads/main", &second_oid);

        // Verify the full history is accessible via C git
        let log = git_in(&bare_path, &["log", "--oneline", "refs/heads/main"]);
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(
            lines.len(),
            4,
            "bare repo should have 4 commits after incremental push, got: {log}"
        );
        Ok(())
    }

    #[test]
    fn push_to_multiple_new_branches() -> gix_testtools::Result {
        // Push the same commit to two different new branches
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());
        let head_oid = repo.head_id()?.to_string();

        do_push(
            &repo,
            &bare_url,
            &[
                "refs/heads/main:refs/heads/branch-a",
                "refs/heads/main:refs/heads/branch-b",
            ],
        )?;

        verify_push_result(&bare_path, "refs/heads/branch-a", &head_oid);
        verify_push_result(&bare_path, "refs/heads/branch-b", &head_oid);
        Ok(())
    }

    #[test]
    fn push_to_empty_bare_repo() -> gix_testtools::Result {
        // Push to a bare repo that has zero refs (not even HEAD resolving)
        let dir = gix_testtools::scripted_fixture_writable("make_push_repos.sh")?;
        let working_path = dir.path().join("working");
        let repo = gix::open_opts(&working_path, crate::restricted())?;

        // Create a fresh empty bare repo
        let empty_bare = dir.path().join("empty.git");
        let status = Command::new("git")
            .args(["init", "--bare", empty_bare.to_str().unwrap()])
            .output()
            .expect("git init --bare");
        assert!(status.status.success());

        let bare_url = format!("file://{}", empty_bare.display());
        let head_oid = repo.head_id()?.to_string();

        do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/main"])?;

        verify_push_result(&empty_bare, "refs/heads/main", &head_oid);

        // Verify C git can clone from the previously-empty bare repo
        let clone_path = dir.path().join("clone_from_empty");
        assert!(
            Command::new("git")
                .args(["clone", &bare_url, clone_path.to_str().unwrap()])
                .output()
                .expect("git clone")
                .status
                .success(),
            "C git should be able to clone from the bare repo after push"
        );
        let cloned_head = git_in(&clone_path, &["rev-parse", "HEAD"]);
        assert_eq!(cloned_head, head_oid);
        Ok(())
    }

    #[test]
    fn push_binary_blob_pack_integrity() -> gix_testtools::Result {
        // Push a commit containing a binary blob and verify pack integrity
        let dir = gix_testtools::scripted_fixture_writable("make_push_repos.sh")?;
        let working_path = dir.path().join("working");
        let bare_path = dir.path().join("remote.git");

        // Create a binary file in the working repo
        let binary_data: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        std::fs::write(working_path.join("binary.bin"), &binary_data).expect("write binary file");
        git_in(&working_path, &["add", "binary.bin"]);
        git_in(&working_path, &["commit", "-m", "add binary blob"]);

        let repo = gix::open_opts(&working_path, crate::restricted())?;
        let head_oid = repo.head_id()?.to_string();
        let bare_url = format!("file://{}", bare_path.display());

        do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/main"])?;

        verify_push_result(&bare_path, "refs/heads/main", &head_oid);

        // Verify pack integrity with C git
        assert!(
            git_in_ok(&bare_path, &["fsck", "--full", "--strict"]),
            "git fsck --strict should pass after pushing binary blob"
        );

        // Verify the binary content is intact by cloning and checking
        let clone_path = dir.path().join("clone_binary");
        assert!(Command::new("git")
            .args(["clone", &bare_url, clone_path.to_str().unwrap()])
            .output()
            .expect("git clone")
            .status
            .success());
        let cloned_binary = std::fs::read(clone_path.join("binary.bin")).expect("read cloned binary");
        assert_eq!(
            cloned_binary, binary_data,
            "binary blob content should survive push+clone roundtrip"
        );
        Ok(())
    }

    #[test]
    fn push_with_symbolic_head_on_remote() -> gix_testtools::Result {
        // Push when the remote has a symbolic HEAD pointing to main
        let (repo, bare_path, _dir) = setup_push_repos()?;
        let bare_url = format!("file://{}", bare_path.display());

        // First push main so the bare repo has it
        let head_oid = repo.head_id()?.to_string();
        do_push(&repo, &bare_url, &["refs/heads/main:refs/heads/main"])?;

        // Set remote HEAD to point to main symbolically
        git_in(&bare_path, &["symbolic-ref", "HEAD", "refs/heads/main"]);

        // Now push feature branch — should work even with symbolic HEAD
        let feature_ref = repo.find_reference("refs/heads/feature")?;
        let feature_oid = feature_ref.id().to_string();
        do_push(&repo, &bare_url, &["refs/heads/feature:refs/heads/feature"])?;

        verify_push_result(&bare_path, "refs/heads/feature", &feature_oid);

        // HEAD should still resolve through the symref
        let remote_head = git_in(&bare_path, &["rev-parse", "HEAD"]);
        assert_eq!(remote_head, head_oid, "remote HEAD should still resolve to main");
        Ok(())
    }
}
