use crate::bstr::{BStr, BString};

/// Options for the [`Repository::worktree_lock()`](crate::Repository::worktree_lock()) method.
#[derive(Debug, Default, Clone)]
pub struct Options<'a> {
    /// An optional reason for locking the worktree.
    pub reason: Option<&'a BStr>,
}

/// The error returned by [`Repository::worktree_lock()`](crate::Repository::worktree_lock()).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum Error {
    #[error("Worktree '{id}' was not found")]
    NotFound { id: BString },
    #[error("Cannot lock the main worktree")]
    MainWorktree,
    #[error("Worktree '{id}' is already locked")]
    AlreadyLocked { id: BString },
    #[error("Failed to write lock file")]
    WriteLock(#[source] std::io::Error),
}

/// The error returned by [`Repository::worktree_unlock()`](crate::Repository::worktree_unlock()).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum UnlockError {
    #[error("Worktree '{id}' was not found")]
    NotFound { id: BString },
    #[error("Cannot unlock the main worktree")]
    MainWorktree,
    #[error("Worktree '{id}' is not locked")]
    NotLocked { id: BString },
    #[error("Failed to remove lock file")]
    RemoveLock(#[source] std::io::Error),
}

impl crate::Repository {
    /// Lock the worktree identified by `id`.
    ///
    /// This creates a `.git/worktrees/<id>/locked` file, optionally containing
    /// a reason for the lock. A locked worktree cannot be pruned, moved, or deleted
    /// (unless force is used).
    ///
    /// The main worktree cannot be locked.
    pub fn worktree_lock<'a>(&self, id: impl Into<&'a BStr>, options: Options<'_>) -> Result<(), Error> {
        let id = id.into();

        // Find the worktree by id
        let proxy = self.worktree_proxy_by_id(id).ok_or_else(|| Error::NotFound {
            id: id.to_owned(),
        })?;

        // Verify this is not the main worktree (main worktree has no entry in worktrees/)
        let git_dir = proxy.git_dir();
        if git_dir.parent().and_then(|p| p.file_name()) != Some(std::ffi::OsStr::new("worktrees")) {
            return Err(Error::MainWorktree);
        }

        // Check if already locked
        if proxy.is_locked() {
            return Err(Error::AlreadyLocked { id: id.to_owned() });
        }

        // Write the lock file
        let lock_content = options
            .reason
            .map(|r| r.to_vec())
            .unwrap_or_default();
        std::fs::write(git_dir.join("locked"), lock_content).map_err(Error::WriteLock)?;

        Ok(())
    }

    /// Unlock the worktree identified by `id`.
    ///
    /// This removes the `.git/worktrees/<id>/locked` file, allowing the worktree
    /// to be pruned, moved, or deleted normally.
    ///
    /// The main worktree cannot be unlocked (as it cannot be locked).
    pub fn worktree_unlock<'a>(&self, id: impl Into<&'a BStr>) -> Result<(), UnlockError> {
        let id = id.into();

        // Find the worktree by id
        let proxy = self.worktree_proxy_by_id(id).ok_or_else(|| UnlockError::NotFound {
            id: id.to_owned(),
        })?;

        // Verify this is not the main worktree
        let git_dir = proxy.git_dir();
        if git_dir.parent().and_then(|p| p.file_name()) != Some(std::ffi::OsStr::new("worktrees")) {
            return Err(UnlockError::MainWorktree);
        }

        // Check if locked
        let lock_path = git_dir.join("locked");
        if !lock_path.exists() {
            return Err(UnlockError::NotLocked { id: id.to_owned() });
        }

        // Remove the lock file
        std::fs::remove_file(&lock_path).map_err(UnlockError::RemoveLock)?;

        Ok(())
    }
}
