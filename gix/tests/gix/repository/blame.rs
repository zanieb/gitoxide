use gix::bstr::BString;
use std::{num::NonZero, path::Path, process::Command};

#[test]
fn simple() -> crate::Result {
    let repo = crate::named_repo("make_blame_repo.sh")?;

    let suspect = repo.head_id()?;
    let outcome = repo.blame_file("simple.txt".into(), suspect, Default::default())?;

    assert_eq!(outcome.entries.len(), 4);

    Ok(())
}

#[test]
fn with_options() -> crate::Result {
    let repo = crate::named_repo("make_blame_repo.sh")?;

    let options = gix::repository::blame_file::Options {
        ranges: gix::blame::BlameRanges::from_one_based_inclusive_range(1..=2)?,
        ..Default::default()
    };

    let suspect = repo.head_id()?;
    let outcome = repo.blame_file("simple.txt".into(), suspect, options)?;

    assert_eq!(outcome.entries.len(), 2);

    let entries_with_lines: Vec<_> = outcome.entries_with_lines().collect();

    assert!(matches!(
        entries_with_lines.as_slice(),
        &[
            (
                gix::blame::BlameEntry {
                    start_in_blamed_file: 0,
                    start_in_source_file: 0,
                    source_file_name: None,
                    ..
                },
                _,
            ),
            (
                gix::blame::BlameEntry {
                    start_in_blamed_file: 1,
                    start_in_source_file: 1,
                    source_file_name: None,
                    ..
                },
                _,
            )
        ]
    ));

    assert_eq!(entries_with_lines[0].0.len, NonZero::new(1).unwrap());
    assert_eq!(entries_with_lines[1].0.len, NonZero::new(1).unwrap());

    assert_eq!(entries_with_lines[0].1, vec![BString::new("line 1\n".into())]);
    assert_eq!(entries_with_lines[1].1, vec![BString::new("line 2\n".into())]);

    Ok(())
}

#[test]
fn shallow_boundary_is_reported_without_missing_parent_error() -> crate::Result {
    let shallow_base = gix_testtools::scripted_fixture_read_only("make_shallow_repo.sh")?;
    let repo = gix::open_opts(shallow_base.join("shallow"), crate::restricted())?;
    assert!(repo.is_shallow(), "fixture should be a shallow clone");

    let suspect = repo.head_id()?;
    let outcome = repo.blame_file("a".into(), suspect, Default::default())?;

    assert_eq!(
        outcome.entries.iter().map(|entry| entry.len.get()).sum::<u32>(),
        2,
        "the entire shallow HEAD file should be covered"
    );
    assert!(
        outcome
            .entries
            .iter()
            .all(|entry| entry.commit_id == suspect.detach() && entry.boundary),
        "missing parents behind a shallow boundary should attribute remaining lines to the boundary commit"
    );

    Ok(())
}

#[test]
fn shallow_file_boundary_is_honored_even_if_parent_object_exists() -> crate::Result {
    let dir = gix_testtools::tempfile::tempdir()?;
    let worktree = dir.path();
    run_git(worktree, &["init", "-q"])?;

    std::fs::write(worktree.join("a"), "one\n")?;
    run_git(worktree, &["add", "a"])?;
    run_git(
        worktree,
        &[
            "-c",
            "user.name=A",
            "-c",
            "user.email=a@example.com",
            "commit",
            "-qm",
            "base",
        ],
    )?;

    std::fs::write(worktree.join("a"), "one\ntwo\n")?;
    run_git(
        worktree,
        &[
            "-c",
            "user.name=A",
            "-c",
            "user.email=a@example.com",
            "commit",
            "-am",
            "second",
            "-q",
        ],
    )?;

    let head = run_git(worktree, &["rev-parse", "HEAD"])?;
    let parent = run_git(worktree, &["rev-parse", "HEAD^"])?;
    std::fs::write(worktree.join(".git").join("shallow"), &head)?;

    let c_git_blame = run_git(worktree, &["blame", "--line-porcelain", "a"])?;
    assert_eq!(
        c_git_blame.matches("boundary\n").count(),
        2,
        "C Git treats HEAD as a shallow boundary even though parent {} exists",
        parent.trim()
    );

    let repo = gix::open_opts(worktree, crate::restricted())?;
    assert!(repo.is_shallow(), "fixture should be explicitly shallow");

    let suspect = repo.head_id()?;
    let outcome = repo.blame_file("a".into(), suspect, Default::default())?;

    assert_eq!(
        outcome.entries.iter().map(|entry| entry.len.get()).sum::<u32>(),
        2,
        "the entire file should be covered"
    );
    assert!(
        outcome
            .entries
            .iter()
            .all(|entry| entry.commit_id == suspect.detach() && entry.boundary),
        "explicit shallow boundaries should prevent traversal to locally present parent objects"
    );

    Ok(())
}

fn run_git(dir: &Path, args: &[&str]) -> crate::Result<String> {
    let output = Command::new("git").args(args).current_dir(dir).output()?;
    assert!(
        output.status.success(),
        "git {} failed\nstdout: {}\nstderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}
