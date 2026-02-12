//! Utilities for setting up the separate git directory layout used by modern submodules.
//!
//! Modern git stores submodule git directories at `$GIT_DIR/modules/<name>/` and places
//! a `.git` *file* (not directory) in the submodule worktree containing `gitdir: <path>`.
//! The submodule config also has `core.worktree` pointing back to the worktree.

use std::path::{Component, Path, PathBuf};

/// Errors that can occur when setting up the separate git directory layout.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("Failed to create modules directory at '{}'", path.display())]
    CreateModulesDir { path: PathBuf, source: std::io::Error },
    #[error("Failed to move git directory from '{}' to '{}'", from.display(), to.display())]
    MoveGitDir {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to write .git file in submodule worktree at '{}'", path.display())]
    WriteGitFile { path: PathBuf, source: std::io::Error },
    #[error("Failed to read or write submodule config at '{}'", path.display())]
    Config {
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Connect a worktree and a separate git directory, as used by modern submodules.
///
/// This performs three operations:
/// 1. Moves the `.git/` directory from `worktree` to `target_git_dir`
/// 2. Writes a `.git` file in `worktree` with `gitdir: <relative_path_to_git_dir>`
/// 3. Sets `core.worktree` in the git dir's config to point back at the worktree
///
/// After this, `gix-discover` will correctly identify the layout as `LinkedWorkTree { git_dir, work_dir }`.
pub(crate) fn connect_work_tree_and_git_dir(worktree: &Path, target_git_dir: &Path) -> Result<(), Error> {
    let clone_git_dir = worktree.join(".git");

    // 1. Create parent directories for the target git dir.
    if let Some(parent) = target_git_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::CreateModulesDir {
            path: parent.to_owned(),
            source: e,
        })?;
    }

    // 2. Move .git directory to target location.
    // Try rename first (fast, same filesystem), fall back to recursive copy + delete.
    if let Err(rename_err) = std::fs::rename(&clone_git_dir, target_git_dir) {
        // Rename failed (likely cross-filesystem). Copy recursively then remove.
        copy_dir_recursive(&clone_git_dir, target_git_dir).map_err(|e| Error::MoveGitDir {
            from: clone_git_dir.clone(),
            to: target_git_dir.to_owned(),
            source: e,
        })?;
        std::fs::remove_dir_all(&clone_git_dir).map_err(|e| Error::MoveGitDir {
            from: clone_git_dir.clone(),
            to: target_git_dir.to_owned(),
            source: e,
        })?;
        // If even the copy failed, report the original rename error context
        if !target_git_dir.is_dir() {
            return Err(Error::MoveGitDir {
                from: clone_git_dir,
                to: target_git_dir.to_owned(),
                source: rename_err,
            });
        }
    }

    // 3. Write the .git file in the worktree.
    let rel_to_git_dir = compute_relative_path(worktree, target_git_dir);
    let git_file_path = worktree.join(".git");
    std::fs::write(&git_file_path, format!("gitdir: {}\n", rel_to_git_dir.display())).map_err(|e| {
        Error::WriteGitFile {
            path: git_file_path,
            source: e,
        }
    })?;

    // 4. Set core.worktree in the submodule's config.
    set_core_worktree(target_git_dir, worktree)?;

    Ok(())
}

/// Ensure `core.worktree` is set in the submodule's config, pointing from `git_dir` to `worktree`.
///
/// This is a safety net for submodule repos that already exist but may be missing the config entry.
pub(crate) fn ensure_core_worktree(git_dir: &Path, worktree: &Path) -> Result<(), Error> {
    let config_path = git_dir.join("config");
    if !config_path.exists() {
        return Ok(());
    }

    let config =
        gix_config::File::from_path_no_includes(config_path.clone(), gix_config::Source::Local).map_err(|e| {
            Error::Config {
                path: config_path.clone(),
                source: Box::new(e),
            }
        })?;

    // Check if core.worktree is already set
    if config.raw_value("core.worktree").is_ok() {
        return Ok(());
    }

    // Not set — add it
    set_core_worktree(git_dir, worktree)
}

/// Write `core.worktree = <relative_path>` into the config at `git_dir/config`.
fn set_core_worktree(git_dir: &Path, worktree: &Path) -> Result<(), Error> {
    let config_path = git_dir.join("config");
    let mut config =
        gix_config::File::from_path_no_includes(config_path.clone(), gix_config::Source::Local).map_err(|e| {
            Error::Config {
                path: config_path.clone(),
                source: Box::new(e),
            }
        })?;

    let rel_worktree = compute_relative_path(git_dir, worktree);
    let rel_worktree_bstr = gix_path::into_bstr(rel_worktree);

    let mut section = config
        .section_mut_or_create_new("core", None)
        .expect("'core' is a valid section name");
    // Remove any existing worktree value first
    while section.remove("worktree").is_some() {}
    section.push(
        "worktree".try_into().expect("valid key"),
        Some(crate::bstr::BStr::new(rel_worktree_bstr.as_ref())),
    );

    let mut out = Vec::new();
    config.write_to(&mut out).map_err(|e| Error::Config {
        path: config_path.clone(),
        source: Box::new(e),
    })?;
    std::fs::write(&config_path, out).map_err(|e| Error::WriteGitFile {
        path: config_path,
        source: e,
    })?;

    Ok(())
}

/// Compute a relative path from `base` to `target`.
///
/// Both paths should be absolute or both relative for correct results.
/// This is equivalent to `pathdiff::diff_paths(target, base)` but without the extra dependency.
fn compute_relative_path(base: &Path, target: &Path) -> PathBuf {
    let base_components: Vec<Component<'_>> = base.components().collect();
    let target_components: Vec<Component<'_>> = target.components().collect();

    // Find the common prefix length
    let common_len = base_components
        .iter()
        .zip(target_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = PathBuf::new();

    // Go up from base to the common ancestor
    for _ in common_len..base_components.len() {
        result.push("..");
    }

    // Go down from the common ancestor to target
    for component in &target_components[common_len..] {
        result.push(component);
    }

    if result.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        result
    }
}

/// Recursively copy a directory and its contents.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_sibling() {
        let base = Path::new("/a/b/c");
        let target = Path::new("/a/b/d");
        assert_eq!(compute_relative_path(base, target), PathBuf::from("../d"));
    }

    #[test]
    fn relative_path_nested() {
        let base = Path::new("/a/b");
        let target = Path::new("/a/b/c/d");
        assert_eq!(compute_relative_path(base, target), PathBuf::from("c/d"));
    }

    #[test]
    fn relative_path_parent() {
        let base = Path::new("/a/b/c/d");
        let target = Path::new("/a/b");
        assert_eq!(compute_relative_path(base, target), PathBuf::from("../.."));
    }

    #[test]
    fn relative_path_same() {
        let base = Path::new("/a/b");
        let target = Path::new("/a/b");
        assert_eq!(compute_relative_path(base, target), PathBuf::from("."));
    }

    #[test]
    fn relative_path_worktree_to_modules() {
        // Typical submodule layout: worktree at /repo/sub, git dir at /repo/.git/modules/sub
        let worktree = Path::new("/repo/sub");
        let git_dir = Path::new("/repo/.git/modules/sub");
        assert_eq!(
            compute_relative_path(worktree, git_dir),
            PathBuf::from("../.git/modules/sub")
        );
    }

    #[test]
    fn relative_path_modules_to_worktree() {
        let git_dir = Path::new("/repo/.git/modules/sub");
        let worktree = Path::new("/repo/sub");
        assert_eq!(compute_relative_path(git_dir, worktree), PathBuf::from("../../../sub"));
    }
}
