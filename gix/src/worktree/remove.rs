use std::path::PathBuf;

use crate::bstr::{BStr, BString, ByteSlice};

/// Options for the [`Repository::worktree_remove()`](crate::Repository::worktree_remove()) method.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// The force level for removing worktrees:
    ///
    /// - `0` (default): Fail if the worktree is dirty or locked
    /// - `1` (equivalent to `git worktree remove -f`): Allow removing dirty worktrees, but still fail if locked
    /// - `2` (equivalent to `git worktree remove -f -f`): Allow removing both dirty and locked worktrees
    ///
    /// This matches git's behavior where each `-f` flag increases the force level.
    pub force: u8,
}

/// The error returned by [`Repository::worktree_remove()`](crate::Repository::worktree_remove()).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum Error {
    #[error("Worktree '{id}' was not found")]
    NotFound { id: BString },
    #[error("Cannot remove the main worktree")]
    MainWorktree,
    #[error("Cannot remove a locked worktree '{id}'{}", format_lock_reason(.reason))]
    Locked { id: BString, reason: Option<BString> },
    #[error("Worktree '{id}' contains modified tracked files, use force to delete")]
    DirtyWorktree { id: BString },
    #[error("Worktree '{id}' contains untracked files, use force to delete")]
    UntrackedFiles { id: BString },
    #[error("Failed to check worktree status")]
    CheckStatus(#[source] crate::status::is_dirty::Error),
    #[error("Failed to create status platform")]
    StatusPlatform(#[source] crate::status::Error),
    #[error("Failed to check for untracked files")]
    CheckUntracked(#[source] crate::status::into_iter::Error),
    #[error("Failed to open worktree as repository")]
    OpenWorktree(#[source] crate::worktree::proxy::into_repo::Error),
    #[error("Failed to read worktree base path from gitdir file")]
    ReadBase(#[from] std::io::Error),
    #[error("Failed to remove worktree directory at '{path}'")]
    RemoveWorktreeDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to remove worktree git directory at '{path}'")]
    RemoveGitDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn format_lock_reason(reason: &Option<BString>) -> String {
    reason
        .as_ref()
        .map(|r| format!(", lock reason: {}", r.to_str_lossy()))
        .unwrap_or_default()
}

impl crate::Repository {
    /// Remove the worktree identified by `id`.
    ///
    /// This removes both the worktree checkout directory (if it exists) and the
    /// `.git/worktrees/<id>` administrative directory.
    ///
    /// The behavior is controlled by [`Options::force`]:
    /// - `0` (default): Fail if the worktree is dirty (has uncommitted changes) or locked
    /// - `1`: Allow removing dirty worktrees, but still fail if locked
    /// - `2`: Allow removing both dirty and locked worktrees
    ///
    /// This matches git's behavior where each `-f` flag increases the force level.
    ///
    /// The main worktree cannot be removed.
    pub fn worktree_remove<'a>(&self, id: impl Into<&'a BStr>, options: Options) -> Result<(), Error> {
        let id = id.into();

        // Find the worktree by id
        let proxy = self.worktree_proxy_by_id(id).ok_or_else(|| Error::NotFound {
            id: id.to_owned(),
        })?;

        // The main worktree has no id in the worktrees directory, so if we found a proxy,
        // it's a linked worktree. But let's double-check by verifying the git_dir is
        // under worktrees/
        let git_dir = proxy.git_dir();
        if git_dir.parent().and_then(|p| p.file_name()) != Some(std::ffi::OsStr::new("worktrees")) {
            return Err(Error::MainWorktree);
        }

        // Check if the worktree is locked (force >= 2 overrides)
        if options.force < 2 && proxy.is_locked() {
            return Err(Error::Locked {
                id: id.to_owned(),
                reason: proxy.lock_reason(),
            });
        }

        // Check if the worktree is dirty (force >= 1 overrides)
        // Only check if the worktree base path exists
        if options.force < 1 {
            if let Ok(base_path) = proxy.base() {
                if base_path.is_dir() {
                    // Try to open the worktree as a repository and check if it's dirty
                    let worktree_repo = proxy
                        .clone()
                        .into_repo()
                        .map_err(Error::OpenWorktree)?;

                    // Check for modified tracked files
                    let is_dirty = worktree_repo.is_dirty().map_err(Error::CheckStatus)?;
                    if is_dirty {
                        return Err(Error::DirtyWorktree { id: id.to_owned() });
                    }

                    // Check for untracked files using status iterator
                    let has_untracked = worktree_repo
                        .status(gix_features::progress::Discard)
                        .map_err(Error::StatusPlatform)?
                        .index_worktree_options_mut(|opts| {
                            // Disable rewrites for faster checking
                            opts.rewrites = None;
                        })
                        .into_index_worktree_iter(Vec::new())
                        .map_err(Error::CheckUntracked)?
                        .filter_map(Result::ok)
                        .any(|item| {
                            use crate::status::index_worktree::Item;
                            matches!(
                                item,
                                Item::DirectoryContents {
                                    entry,
                                    ..
                                } if entry.status == gix_dir::entry::Status::Untracked
                            )
                        });

                    if has_untracked {
                        return Err(Error::UntrackedFiles { id: id.to_owned() });
                    }
                }
            }
        }

        // Get the worktree base path and remove it if it exists
        // We don't fail if the base path cannot be read or doesn't exist
        if let Ok(base_path) = proxy.base() {
            if base_path.is_dir() {
                std::fs::remove_dir_all(&base_path).map_err(|source| Error::RemoveWorktreeDir {
                    path: base_path,
                    source,
                })?;
            }
        }

        // Remove the .git/worktrees/<id> directory
        let worktree_git_dir = git_dir.to_owned();
        std::fs::remove_dir_all(&worktree_git_dir).map_err(|source| Error::RemoveGitDir {
            path: worktree_git_dir,
            source,
        })?;

        // Try to remove the empty worktrees directory (ignore errors)
        let worktrees_dir = self.common_dir().join("worktrees");
        let _ = std::fs::remove_dir(&worktrees_dir);

        Ok(())
    }
}
