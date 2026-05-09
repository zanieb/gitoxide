use gix_glob::pattern::Case;

mod create_directory;

mod attributes;
mod ignore;

fn probe_case() -> crate::Result<Case> {
    let (git_dir, worktree_dir) = gix_discover::upwards(".".as_ref())?
        .0
        .into_repository_and_work_tree_directories();
    let capabilities = match worktree_dir {
        Some(worktree_dir) => gix_fs::Capabilities::probe_dir(&worktree_dir),
        None => gix_fs::Capabilities::probe(&git_dir),
    };
    Ok(if capabilities.ignore_case {
        Case::Fold
    } else {
        Case::Sensitive
    })
}
