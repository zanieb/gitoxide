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

fn git_bundle_create_main(dir: &std::path::Path, bundle_path: &std::path::Path) {
    let output = Command::new("git")
        .args(["bundle", "create"])
        .arg(bundle_path)
        .arg("refs/heads/main")
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

#[test]
fn unbundle_rejects_ref_targets_missing_from_pack() -> crate::Result {
    let tmp = gix_testtools::tempfile::tempdir()?;
    let source = tmp.path().join("source");
    std::fs::create_dir(&source)?;

    git_output(&source, &["init", "-q", "-b", "main"]);
    git_output(&source, &["config", "user.name", "Bundle Test"]);
    git_output(&source, &["config", "user.email", "bundle@example.com"]);
    std::fs::write(source.join("file"), "content\n")?;
    git_output(&source, &["add", "file"]);
    git_output(&source, &["commit", "-q", "-m", "initial"]);
    let real_head = git_output(&source, &["rev-parse", "HEAD"]);

    let bundle_path = tmp.path().join("tampered.bundle");
    git_bundle_create_main(&source, &bundle_path);

    let missing_head = "1111111111111111111111111111111111111111";
    assert_ne!(missing_head, real_head, "test object id must differ from HEAD");

    let mut bundle = std::fs::read(&bundle_path)?;
    let header_end = bundle
        .windows(2)
        .position(|window| window == b"\n\n")
        .expect("bundle has a header terminator");
    let oid_pos = bundle[..header_end]
        .windows(real_head.len())
        .position(|window| window == real_head.as_bytes())
        .expect("bundle header contains HEAD id");
    bundle[oid_pos..oid_pos + real_head.len()].copy_from_slice(missing_head.as_bytes());
    std::fs::write(&bundle_path, bundle)?;

    let target_path = tmp.path().join("target.git");
    let target = gix::init_bare(&target_path)?;
    let err = target
        .bundle_unbundle(&bundle_path)
        .expect_err("tampered bundle refs must not be imported");

    match err {
        gix::bundle::Error::MissingRefObject { name, id } => {
            assert_eq!(name.as_bstr(), "refs/heads/main");
            assert_eq!(id.to_string(), missing_head);
        }
        other => panic!("expected MissingRefObject, got {other:?}"),
    }
    assert!(
        target.try_find_reference("refs/heads/main")?.is_none(),
        "unbundle failure must not leave a ref pointing at the missing object"
    );
    assert!(
        std::fs::read_dir(target.git_dir().join("objects/pack"))?
            .next()
            .is_none(),
        "unbundle failure must remove the imported pack when advertised refs are missing"
    );

    Ok(())
}

#[test]
fn unbundle_rejects_invalid_ref_names_before_importing_pack() -> crate::Result {
    let tmp = gix_testtools::tempfile::tempdir()?;
    let source = tmp.path().join("source");
    std::fs::create_dir(&source)?;

    git_output(&source, &["init", "-q", "-b", "main"]);
    git_output(&source, &["config", "user.name", "Bundle Test"]);
    git_output(&source, &["config", "user.email", "bundle@example.com"]);
    std::fs::write(source.join("file"), "content\n")?;
    git_output(&source, &["add", "file"]);
    git_output(&source, &["commit", "-q", "-m", "initial"]);

    let bundle_path = tmp.path().join("invalid-ref.bundle");
    git_bundle_create_main(&source, &bundle_path);

    let mut bundle = std::fs::read(&bundle_path)?;
    let header_end = bundle
        .windows(2)
        .position(|window| window == b"\n\n")
        .expect("bundle has a header terminator");
    let valid_ref = b"refs/heads/main";
    let invalid_ref = b"refs/heads/.bad";
    assert_eq!(valid_ref.len(), invalid_ref.len());
    let ref_pos = bundle[..header_end]
        .windows(valid_ref.len())
        .position(|window| window == valid_ref)
        .expect("bundle header contains main ref");
    bundle[ref_pos..ref_pos + valid_ref.len()].copy_from_slice(invalid_ref);
    std::fs::write(&bundle_path, bundle)?;

    let target_path = tmp.path().join("target.git");
    let target = gix::init_bare(&target_path)?;
    let err = target
        .bundle_unbundle(&bundle_path)
        .expect_err("invalid bundle refs must be rejected");

    match err {
        gix::bundle::Error::InvalidRefName { name, .. } => {
            assert_eq!(name.as_bstr(), "refs/heads/.bad");
        }
        other => panic!("expected InvalidRefName, got {other:?}"),
    }
    assert!(
        std::fs::read_dir(target.git_dir().join("objects/pack"))?
            .next()
            .is_none(),
        "invalid ref names must be rejected before importing the pack"
    );

    Ok(())
}
