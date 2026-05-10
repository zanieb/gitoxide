use std::process::Command;

use gix::bstr::ByteSlice;

fn git_output(dir: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be on PATH for bundle interop tests");
    assert!(
        output.status.success(),
        "git {} failed in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_bundle_create(dir: &std::path::Path, bundle_path: &std::path::Path) {
    let output = Command::new("git")
        .args(["bundle", "create"])
        .arg(bundle_path)
        .args(["HEAD", "refs/heads/main", "refs/tags/v1"])
        .current_dir(dir)
        .output()
        .expect("git must be on PATH for bundle interop tests");
    assert!(
        output.status.success(),
        "git bundle create failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unbundle_imports_c_git_bundle_pack_and_refs() -> crate::Result {
    let tmp = gix_testtools::tempfile::tempdir()?;
    let source = tmp.path().join("source");
    std::fs::create_dir(&source)?;

    git_output(&source, &["init", "-q", "-b", "main"]);
    git_output(&source, &["config", "user.name", "Bundle Test"]);
    git_output(&source, &["config", "user.email", "bundle@example.com"]);
    std::fs::write(source.join("file"), "content\n")?;
    git_output(&source, &["add", "file"]);
    git_output(&source, &["commit", "-q", "-m", "initial"]);
    git_output(&source, &["tag", "v1"]);

    let bundle_path = tmp.path().join("repo.bundle");
    git_bundle_create(&source, &bundle_path);

    let target_path = tmp.path().join("target.git");
    let target = gix::init_bare(&target_path)?;
    let outcome = target.bundle_unbundle(&bundle_path)?;

    assert!(
        outcome.write_pack.index.num_objects > 0,
        "a complete bundle should contain objects"
    );
    assert!(
        outcome.write_pack.keep_path.is_none(),
        ".keep is removed after refs make the pack reachable"
    );
    assert!(
        !outcome.reference_edits.is_empty(),
        "bundle refs should be imported into the target repository"
    );

    let main = outcome
        .header
        .refs
        .iter()
        .find(|r| r.name.as_bstr() == "refs/heads/main")
        .expect("bundle contains main branch");
    assert_eq!(target.find_reference("refs/heads/main")?.id().detach(), main.id);
    target.find_object(main.id)?;

    let tag = outcome
        .header
        .refs
        .iter()
        .find(|r| r.name.as_bstr() == "refs/tags/v1")
        .expect("bundle contains tag");
    assert_eq!(target.find_reference("refs/tags/v1")?.id().detach(), tag.id);

    let head = target.find_reference("HEAD")?;
    assert_eq!(
        head.target()
            .try_name()
            .expect("unbundling must leave HEAD symbolic")
            .as_bstr(),
        "refs/heads/main"
    );
    if outcome.header.refs.iter().any(|r| r.name.as_bstr() == "HEAD") {
        assert!(
            outcome.skipped_refs.iter().any(|r| r.name.as_bstr() == "HEAD"),
            "pseudo refs advertised by the bundle should not be written"
        );
    }

    Ok(())
}
