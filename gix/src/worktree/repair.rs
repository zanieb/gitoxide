use std::path::{Path, PathBuf};

use crate::bstr::BString;

/// Information about a repair that was performed.
#[derive(Debug, Clone)]
pub struct Repair {
    /// The worktree identifier.
    pub id: BString,
    /// The path to the administrative git directory.
    pub git_dir: PathBuf,
    /// What was repaired.
    pub kind: RepairKind,
}

/// The kind of repair that was performed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RepairKind {
    /// The `.git` file in the worktree was missing or incorrect and was recreated.
    GitFile,
    /// The `gitdir` file in `.git/worktrees/<id>/` was missing or incorrect and was updated.
    Gitdir,
}

impl std::fmt::Display for RepairKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepairKind::GitFile => write!(f, ".git file"),
            RepairKind::Gitdir => write!(f, "gitdir file"),
        }
    }
}

/// The error returned by [`Repository::worktree_repair()`](crate::Repository::worktree_repair()).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum Error {
    #[error("Failed to read worktrees directory")]
    ReadWorktreesDir(#[source] std::io::Error),
    #[error("Path '{path}' is not a valid worktree path")]
    InvalidPath { path: PathBuf },
    #[error("Path '{path}' is not a directory")]
    NotADirectory { path: PathBuf },
    #[error("Path '{path}' has .git that is not a file (is it a regular repository?)")]
    DotGitNotAFile { path: PathBuf },
    #[error("Failed to write .git file at '{path}'")]
    WriteGitFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to write gitdir file at '{path}'")]
    WriteGitdir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Could not find repository for worktree at '{path}'")]
    RepositoryNotFound { path: PathBuf },
}

impl crate::Repository {
    /// Repair worktree links for all worktrees or for specific paths.
    ///
    /// If `paths` is empty, repairs all worktrees known to this repository.
    /// Otherwise, repairs only the worktrees at the specified paths.
    ///
    /// This fixes:
    /// - Missing or incorrect `.git` files in worktree directories
    /// - Missing or incorrect `gitdir` files in `.git/worktrees/<id>/`
    ///
    /// Returns a list of repairs that were performed.
    pub fn worktree_repair(&self, paths: &[PathBuf]) -> Result<Vec<Repair>, Error> {
        let mut repairs = Vec::new();

        if paths.is_empty() {
            // Repair all known worktrees
            self.repair_all_worktrees(&mut repairs)?;
        } else {
            // Repair specific paths
            for path in paths {
                self.repair_worktree_at_path(path, &mut repairs)?;
            }
        }

        Ok(repairs)
    }

    /// Repair all worktrees known to this repository.
    fn repair_all_worktrees(&self, repairs: &mut Vec<Repair>) -> Result<(), Error> {
        let worktrees_dir = self.common_dir().join("worktrees");

        let entries = match std::fs::read_dir(&worktrees_dir) {
            Ok(iter) => iter,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(Error::ReadWorktreesDir(err)),
        };

        for entry in entries {
            let entry = entry.map_err(Error::ReadWorktreesDir)?;
            let worktree_git_dir = entry.path();

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

            // Try to read the gitdir file to find the worktree path
            let gitdir_path = worktree_git_dir.join("gitdir");
            if let Ok(gitdir_content) = std::fs::read_to_string(&gitdir_path) {
                let worktree_dot_git = gitdir_content.trim();
                if !worktree_dot_git.is_empty() {
                    let worktree_dot_git_path = PathBuf::from(worktree_dot_git);
                    if let Some(worktree_path) = worktree_dot_git_path.parent() {
                        if worktree_path.is_dir() {
                            // Verify and potentially repair the .git file
                            if let Some(repair) = self.repair_dot_git_file(worktree_path, &worktree_git_dir, &id)? {
                                repairs.push(repair);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Repair a worktree at a specific path.
    fn repair_worktree_at_path(&self, path: &Path, repairs: &mut Vec<Repair>) -> Result<(), Error> {
        // Normalize the path
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|_| Error::InvalidPath { path: path.to_owned() })?
                .join(path)
        };

        // Verify path exists and is a directory
        if !path.exists() {
            return Err(Error::InvalidPath { path });
        }
        if !path.is_dir() {
            return Err(Error::NotADirectory { path });
        }

        let dot_git_path = path.join(".git");

        // Check if .git exists and what type it is
        if dot_git_path.exists() && !dot_git_path.is_file() {
            return Err(Error::DotGitNotAFile { path });
        }

        // Try to find the worktree admin directory
        // First, try to read existing .git file
        let worktree_git_dir = if dot_git_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&dot_git_path) {
                let content = content.trim();
                if let Some(gitdir) = content.strip_prefix("gitdir: ") {
                    let gitdir_path = if Path::new(gitdir).is_absolute() {
                        PathBuf::from(gitdir)
                    } else {
                        path.join(gitdir)
                    };
                    if gitdir_path.is_dir() {
                        Some(gitdir_path)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // If we couldn't find the admin dir from .git file, search worktrees/
        let worktree_git_dir = match worktree_git_dir {
            Some(dir) => dir,
            None => self.find_worktree_admin_dir_for_path(&path)?,
        };

        let id = BString::from(
            worktree_git_dir
                .file_name()
                .expect("directory has a name")
                .to_string_lossy()
                .as_bytes()
                .to_vec(),
        );

        // Repair the .git file if needed
        if let Some(repair) = self.repair_dot_git_file(&path, &worktree_git_dir, &id)? {
            repairs.push(repair);
        }

        // Repair the gitdir file if needed
        if let Some(repair) = self.repair_gitdir_file(&path, &worktree_git_dir, &id)? {
            repairs.push(repair);
        }

        Ok(())
    }

    /// Find the worktree admin directory for a given worktree path.
    fn find_worktree_admin_dir_for_path(&self, worktree_path: &Path) -> Result<PathBuf, Error> {
        let worktrees_dir = self.common_dir().join("worktrees");

        let entries = match std::fs::read_dir(&worktrees_dir) {
            Ok(iter) => iter,
            Err(_) => {
                return Err(Error::RepositoryNotFound {
                    path: worktree_path.to_owned(),
                })
            }
        };

        let worktree_dot_git = worktree_path.join(".git");

        for entry in entries.flatten() {
            let admin_dir = entry.path();
            if !admin_dir.is_dir() {
                continue;
            }

            let gitdir_path = admin_dir.join("gitdir");
            if let Ok(content) = std::fs::read_to_string(&gitdir_path) {
                let expected_dot_git = PathBuf::from(content.trim());
                // Check if this gitdir points to our worktree
                if expected_dot_git == worktree_dot_git
                    || expected_dot_git.canonicalize().ok().map_or(false, |p| {
                        worktree_dot_git.canonicalize().ok().map_or(false, |w| p == w)
                    })
                {
                    return Ok(admin_dir);
                }
            }
        }

        Err(Error::RepositoryNotFound {
            path: worktree_path.to_owned(),
        })
    }

    /// Repair the .git file in a worktree directory.
    fn repair_dot_git_file(
        &self,
        worktree_path: &Path,
        admin_dir: &Path,
        id: &BString,
    ) -> Result<Option<Repair>, Error> {
        let dot_git_path = worktree_path.join(".git");
        let expected_content = format!("gitdir: {}\n", admin_dir.display());

        // Check if repair is needed
        let needs_repair = if dot_git_path.is_file() {
            match std::fs::read_to_string(&dot_git_path) {
                Ok(content) => content.trim() != expected_content.trim(),
                Err(_) => true,
            }
        } else {
            true
        };

        if needs_repair {
            std::fs::write(&dot_git_path, &expected_content).map_err(|source| Error::WriteGitFile {
                path: dot_git_path.clone(),
                source,
            })?;

            Ok(Some(Repair {
                id: id.clone(),
                git_dir: admin_dir.to_owned(),
                kind: RepairKind::GitFile,
            }))
        } else {
            Ok(None)
        }
    }

    /// Repair the gitdir file in the worktree admin directory.
    fn repair_gitdir_file(
        &self,
        worktree_path: &Path,
        admin_dir: &Path,
        id: &BString,
    ) -> Result<Option<Repair>, Error> {
        let gitdir_path = admin_dir.join("gitdir");
        let worktree_dot_git = worktree_path.join(".git");
        let expected_content = format!("{}\n", worktree_dot_git.display());

        // Check if repair is needed
        let needs_repair = if gitdir_path.is_file() {
            match std::fs::read_to_string(&gitdir_path) {
                Ok(content) => content.trim() != expected_content.trim(),
                Err(_) => true,
            }
        } else {
            true
        };

        if needs_repair {
            std::fs::write(&gitdir_path, &expected_content).map_err(|source| Error::WriteGitdir {
                path: gitdir_path.clone(),
                source,
            })?;

            Ok(Some(Repair {
                id: id.clone(),
                git_dir: admin_dir.to_owned(),
                kind: RepairKind::Gitdir,
            }))
        } else {
            Ok(None)
        }
    }
}
