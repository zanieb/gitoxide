use anyhow::Context;
use gix::{ObjectId, objs::Kind};

pub fn function(mut repo: gix::Repository, spec: Option<String>, mut out: impl std::io::Write) -> anyhow::Result<()> {
    let spec = spec.unwrap_or("HEAD".into());

    repo.object_cache_size_if_unset(4 * 1024 * 1024);
    // We expect to be finding a bunch of non-existent objects here - never refresh the ODB
    repo.objects.refresh_never();

    let id = repo
        .rev_parse_single(spec.as_str())
        .context("Only single revisions are supported")?;
    let commits: gix::revision::Walk<'_> = id
        .object()?
        .peel_to_kind(gix::object::Kind::Commit)
        .context("Need committish as starting point")?
        .id()
        .ancestors()
        .all()?;

    let on_missing = |oid: &ObjectId, kind: Kind| {
        writeln!(out, "{oid}: {kind}").expect("failed to write output");
    };

    let shallow_commits = repo
        .shallow_commits()?
        .map(|commits| commits.iter().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    let is_shallow_commit = |id: &ObjectId| shallow_commits.binary_search(id).is_ok();

    let mut check = gix_fsck::Connectivity::new(&repo.objects, on_missing);
    let options = gix_fsck::Options {
        verify_hashes: true,
        skip_commit_parents: (!shallow_commits.is_empty()).then_some(&is_shallow_commit as &dyn Fn(&ObjectId) -> bool),
        ..Default::default()
    };
    // Walk all commits, checking each one for connectivity
    for commit in commits {
        let commit = commit?;
        check.check_commit_with_options(&commit.id, options)?;
        // The checker validates parent links itself. The outer walk preserves the
        // command's previous traversal shape, while the seen set keeps repeated
        // roots cheap and `skip_commit_parents` handles shallow boundaries.
    }
    Ok(())
}
