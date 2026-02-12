use std::path::PathBuf;

use crate::bstr::BString;

/// Options for the [`Repository::worktree_prune()`](crate::Repository::worktree_prune()) method.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// If `true`, only report what would be pruned without actually deleting anything.
    pub dry_run: bool,
}

/// The reason why a worktree is considered stale and can be pruned.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PruneReason {
    /// The `.git/worktrees/<id>/gitdir` file is missing.
    MissingGitdir,
    /// The gitdir file exists but cannot be read or contains invalid content.
    InvalidGitdir,
    /// The worktree path referenced by the gitdir file does not exist.
    MissingWorktreePath,
}

impl std::fmt::Display for PruneReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PruneReason::MissingGitdir => write!(f, "gitdir file missing"),
            PruneReason::InvalidGitdir => write!(f, "gitdir file invalid"),
            PruneReason::MissingWorktreePath => write!(f, "worktree path missing"),
        }
    }
}

/// Information about a worktree that was or would be pruned.
#[derive(Debug, Clone)]
pub struct PrunedWorktree {
    /// The worktree identifier (directory name under `.git/worktrees/`).
    pub id: BString,
    /// The path to the worktree's administrative git directory that was removed.
    pub git_dir: PathBuf,
    /// The reason this worktree was prunable.
    pub reason: PruneReason,
}

/// The error returned by [`Repository::worktree_prune()`](crate::Repository::worktree_prune()).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum Error {
    #[error("Failed to read worktrees directory")]
    ReadWorktreesDir(#[source] std::io::Error),
    #[error("Failed to remove stale worktree '{id}'")]
    RemoveStale {
        id: BString,
        #[source]
        source: std::io::Error,
    },
}

impl crate::Repository {
    /// Prune (remove) stale worktree entries from `.git/worktrees/`.
    ///
    /// A worktree is considered stale if:
    /// - The `gitdir` file is missing or unreadable
    /// - The worktree path referenced in `gitdir` does not exist
    ///
    /// Locked worktrees are never pruned, even if they are stale.
    ///
    /// Returns a list of worktrees that were pruned (or would be pruned if `dry_run` is true).
    pub fn worktree_prune(&self, options: Options) -> Result<Vec<PrunedWorktree>, Error> {
        let worktrees_dir = self.common_dir().join("worktrees");
        let mut pruned = Vec::new();

        // If worktrees directory doesn't exist, nothing to prune
        let entries = match std::fs::read_dir(&worktrees_dir) {
            Ok(iter) => iter,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(pruned),
            Err(err) => return Err(Error::ReadWorktreesDir(err)),
        };

        for entry in entries {
            let entry = entry.map_err(Error::ReadWorktreesDir)?;
            let worktree_git_dir = entry.path();

            // Skip if not a directory
            if !worktree_git_dir.is_dir() {
                continue;
            }

            let id = BString::from(
                worktree_git_dir
                    .file_name()
                    .expect("directory has a name")
                    .to_string_lossy()
                    .as_bytes()
                    .to_vec(),
            );

            // Skip locked worktrees - they are never pruned
            if worktree_git_dir.join("locked").exists() {
                continue;
            }

            // Check if this worktree is prunable
            if let Some(reason) = self.check_prunable(&worktree_git_dir) {
                if !options.dry_run {
                    // Actually remove the stale worktree entry
                    std::fs::remove_dir_all(&worktree_git_dir)
                        .map_err(|source| Error::RemoveStale { id: id.clone(), source })?;
                }

                pruned.push(PrunedWorktree {
                    id,
                    git_dir: worktree_git_dir,
                    reason,
                });
            }
        }

        // Clean up empty worktrees directory if not a dry run
        if !options.dry_run && pruned.len() > 0 {
            let _ = std::fs::remove_dir(&worktrees_dir);
        }

        Ok(pruned)
    }

    /// Check if a worktree entry is prunable, returning the reason if so.
    fn check_prunable(&self, worktree_git_dir: &std::path::Path) -> Option<PruneReason> {
        let gitdir_file = worktree_git_dir.join("gitdir");

        // Check if gitdir file exists
        if !gitdir_file.exists() {
            return Some(PruneReason::MissingGitdir);
        }

        // Try to read and parse the gitdir file
        let gitdir_content = match std::fs::read_to_string(&gitdir_file) {
            Ok(content) => content,
            Err(_) => return Some(PruneReason::InvalidGitdir),
        };

        // The gitdir file contains the path to the .git file in the worktree
        let worktree_dot_git = gitdir_content.trim();
        if worktree_dot_git.is_empty() {
            return Some(PruneReason::InvalidGitdir);
        }

        // The worktree path is the parent of the .git file
        let worktree_dot_git_path = std::path::Path::new(worktree_dot_git);
        let worktree_path = worktree_dot_git_path.parent();

        // Check if worktree path exists
        match worktree_path {
            Some(path) if path.exists() => None, // Worktree exists, not prunable
            _ => Some(PruneReason::MissingWorktreePath),
        }
    }
}
