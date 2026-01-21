use std::path::{Path, PathBuf};

use crate::bstr::{BStr, BString, ByteSlice};

/// Options for the [`Repository::worktree_move()`](crate::Repository::worktree_move()) method.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// The force level for moving worktrees:
    ///
    /// - `0` (default): Fail if the worktree is locked
    /// - `1` (equivalent to `git worktree move -f`): Still fail if locked (reserved for future use)
    /// - `2` (equivalent to `git worktree move -f -f`): Allow moving locked worktrees
    ///
    /// This matches git's behavior where double `-f` is needed to move locked worktrees.
    pub force: u8,
}

/// The error returned by [`Repository::worktree_move()`](crate::Repository::worktree_move()).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum Error {
    #[error("Worktree '{id}' was not found")]
    NotFound { id: BString },
    #[error("Cannot move the main worktree")]
    MainWorktree,
    #[error("Cannot move a locked worktree '{id}'{}", format_lock_reason(.reason))]
    Locked { id: BString, reason: Option<BString> },
    #[error("Destination path '{path}' already exists")]
    DestinationExists { path: PathBuf },
    #[error("Failed to read worktree base path")]
    ReadBase(#[source] std::io::Error),
    #[error("Failed to move worktree directory from '{from}' to '{to}'")]
    MoveWorktreeDir {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to update gitdir file at '{path}'")]
    UpdateGitdir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to create parent directory for destination")]
    CreateParentDir(#[source] std::io::Error),
}

fn format_lock_reason(reason: &Option<BString>) -> String {
    reason
        .as_ref()
        .map(|r| format!(", lock reason: {}", r.to_str_lossy()))
        .unwrap_or_default()
}

impl crate::Repository {
    /// Move the worktree identified by `id` to a new path.
    ///
    /// This moves both the worktree checkout directory and updates the
    /// `.git/worktrees/<id>/gitdir` file to point to the new location.
    ///
    /// The behavior is controlled by [`Options::force`]:
    /// - `0` (default): Fail if the worktree is locked
    /// - `2`: Allow moving locked worktrees
    ///
    /// The main worktree cannot be moved.
    pub fn worktree_move<'a>(
        &self,
        id: impl Into<&'a BStr>,
        new_path: impl AsRef<Path>,
    ) -> Result<(), Error> {
        self.worktree_move_inner(id.into(), new_path.as_ref(), Options::default())
    }

    /// Move the worktree identified by `id` to a new path with options.
    ///
    /// See [`worktree_move`](Self::worktree_move) for more details.
    pub fn worktree_move_opts<'a>(
        &self,
        id: impl Into<&'a BStr>,
        new_path: impl AsRef<Path>,
        options: Options,
    ) -> Result<(), Error> {
        self.worktree_move_inner(id.into(), new_path.as_ref(), options)
    }

    fn worktree_move_inner(&self, id: &BStr, new_path: &Path, options: Options) -> Result<(), Error> {
        // Find the worktree by id
        let proxy = self.worktree_proxy_by_id(id).ok_or_else(|| Error::NotFound {
            id: id.to_owned(),
        })?;

        // Verify this is not the main worktree
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

        // Get the current worktree base path
        let current_path = proxy.base().map_err(Error::ReadBase)?;

        // Normalize the new path
        let new_path = if new_path.is_absolute() {
            new_path.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|e| Error::MoveWorktreeDir {
                    from: current_path.clone(),
                    to: new_path.to_owned(),
                    source: e,
                })?
                .join(new_path)
        };

        // Check if destination already exists
        if new_path.exists() {
            return Err(Error::DestinationExists { path: new_path });
        }

        // Create parent directory if needed
        if let Some(parent) = new_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(Error::CreateParentDir)?;
            }
        }

        // Move the worktree directory
        std::fs::rename(&current_path, &new_path).map_err(|source| Error::MoveWorktreeDir {
            from: current_path.clone(),
            to: new_path.clone(),
            source,
        })?;

        // Update the gitdir file to point to the new location
        let gitdir_path = git_dir.join("gitdir");
        let new_dot_git_path = new_path.join(".git");
        std::fs::write(&gitdir_path, format!("{}\n", new_dot_git_path.display()))
            .map_err(|source| Error::UpdateGitdir {
                path: gitdir_path,
                source,
            })?;

        Ok(())
    }
}
