#![allow(clippy::result_large_err)]
use std::path::{Path, PathBuf};

use crate::{
    Repository, ThreadSafeRepository,
    bstr::{BStr, BString, ByteSlice},
    worktree::Proxy,
};

#[allow(missing_docs)]
pub mod into_repo {
    use std::path::PathBuf;

    /// The error returned by [`Proxy::into_repo()`][super::Proxy::into_repo()].
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        Open(#[from] crate::open::Error),
        #[error("Worktree at '{}' is inaccessible", .base.display())]
        MissingWorktree { base: PathBuf },
        #[error(transparent)]
        MissingGitDirFile(#[from] std::io::Error),
    }
}

impl<'repo> Proxy<'repo> {
    pub(crate) fn new(parent: &'repo Repository, git_dir: impl Into<PathBuf>) -> Self {
        Proxy {
            parent,
            git_dir: git_dir.into(),
        }
    }

    pub(crate) fn new_if_gitdir_file_exists(parent: &'repo Repository, git_dir: impl Into<PathBuf>) -> Option<Self> {
        let git_dir = git_dir.into();
        if git_dir.join("gitdir").is_file() {
            Some(Proxy::new(parent, git_dir))
        } else {
            None
        }
    }
}

impl Proxy<'_> {
    /// Read the location of the checkout, the base of the work tree.
    /// Note that the location might not exist.
    pub fn base(&self) -> std::io::Result<PathBuf> {
        let git_dir = self.git_dir.join("gitdir");
        let base_dot_git = gix_discover::path::from_plain_file(&git_dir).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Required file '{}' does not exist", git_dir.display()),
            )
        })??;

        Ok(gix_discover::path::without_dot_git_dir(base_dot_git))
    }

    pub(crate) fn validate_gitfile_backlink(&self, base: &Path) -> std::io::Result<()> {
        let dot_git = base.join(gix_discover::DOT_GIT_DIR);
        let backlink = gix_discover::path::from_gitdir_file(&dot_git).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "worktree .git file '{}' does not point to an administrative git directory: {err}",
                    dot_git.display()
                ),
            )
        })?;

        let backlink = backlink.canonicalize().map_err(|err| {
            std::io::Error::new(
                err.kind(),
                format!(
                    "worktree .git file '{}' points to inaccessible git directory '{}': {err}",
                    dot_git.display(),
                    backlink.display()
                ),
            )
        })?;
        let expected = self.git_dir.canonicalize().map_err(|err| {
            std::io::Error::new(
                err.kind(),
                format!(
                    "worktree administrative git directory '{}' is inaccessible: {err}",
                    self.git_dir.display()
                ),
            )
        })?;

        if backlink != expected {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "worktree .git file '{}' points to '{}', expected '{}'",
                    dot_git.display(),
                    backlink.display(),
                    expected.display()
                ),
            ));
        }

        Ok(())
    }

    /// The git directory for the work tree, typically contained within the parent git dir.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// The name of the worktree, which is derived from its folder within the `worktrees` directory within the parent `.git` folder.
    pub fn id(&self) -> &BStr {
        gix_path::os_str_into_bstr(self.git_dir.file_name().expect("worktrees/ parent dir"))
            .expect("no illformed UTF-8")
    }

    /// Return true if the worktree cannot be pruned, moved or deleted, which is useful if it is located on an external storage device.
    pub fn is_locked(&self) -> bool {
        self.git_dir.join("locked").is_file()
    }

    /// Provide a reason for the locking of this worktree, if it is locked at all.
    ///
    /// Note that we squelch errors in case the file cannot be read in which case the
    /// reason is an empty string.
    pub fn lock_reason(&self) -> Option<BString> {
        std::fs::read(self.git_dir.join("locked"))
            .ok()
            .map(|contents| contents.trim().into())
    }

    /// Transform this proxy into a [`Repository`] while ignoring issues reading `base()` and ignoring that it might not exist.
    ///
    /// Most importantly, the `Repository` might be initialized with a non-existing work tree directory as the checkout
    /// was removed or moved in the mean time or is unavailable for other reasons.
    /// The caller will encounter io errors if it's used like the work tree is guaranteed to be present, but can still access
    /// a lot of information if work tree access is avoided.
    pub fn into_repo_with_possibly_inaccessible_worktree(self) -> Result<Repository, crate::open::Error> {
        let base = self.base().ok();
        let repo = ThreadSafeRepository::open_from_paths(self.git_dir, base, self.parent.options.clone())?;
        Ok(repo.into())
    }

    /// Like `into_repo_with_possibly_inaccessible_worktree()` but will fail if the `base()` cannot be read or
    /// if the worktree doesn't exist.
    ///
    /// Note that it won't fail if the worktree doesn't exist.
    pub fn into_repo(self) -> Result<Repository, into_repo::Error> {
        let base = self.base()?;
        if !base.is_dir() {
            return Err(into_repo::Error::MissingWorktree { base });
        }
        let repo = ThreadSafeRepository::open_from_paths(self.git_dir, base.into(), self.parent.options.clone())?;
        Ok(repo.into())
    }
}
