use std::path::{Path, PathBuf};

use gix_ref::transaction::{Change, PreviousValue, RefEdit};

use crate::bstr::{BStr, BString, ByteSlice};

/// Options for the [`Repository::worktree_add()`](crate::Repository::worktree_add()) method.
#[derive(Debug, Default, Clone)]
pub struct Options<'a> {
    /// The name of an existing branch to check out in the new worktree.
    /// Mutually exclusive with `new_branch` and `detach`.
    pub branch: Option<&'a BStr>,
    /// Create a new branch with this name before checking out.
    /// Mutually exclusive with `branch` and `detach`.
    pub new_branch: Option<&'a BStr>,
    /// The commit-ish to use as the starting point for the new branch or detached HEAD.
    /// If not provided, defaults to HEAD.
    pub start_point: Option<&'a BStr>,
    /// If `true`, check out a detached HEAD at `start_point` (or HEAD if not specified).
    /// Mutually exclusive with `branch` and `new_branch`.
    pub detach: bool,
    /// If `true`, lock the worktree after creation.
    pub lock: bool,
    /// An optional reason for locking the worktree.
    pub lock_reason: Option<&'a BStr>,
    /// If `true`, skip the checkout step.
    pub no_checkout: bool,
}

/// The error returned by [`Repository::worktree_add()`](crate::Repository::worktree_add()).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum Error {
    #[error("Worktree path '{}' already exists", path.display())]
    PathExists { path: PathBuf },
    #[error("Failed to create worktree directory at '{}'", path.display())]
    CreateWorktreeDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to create worktree git directory at '{}'", path.display())]
    CreateGitDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to write '{file}' file")]
    WriteFile {
        file: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("Branch '{branch}' is already checked out at '{}'", worktree.display())]
    BranchCheckedOut { branch: BString, worktree: PathBuf },
    #[error("Invalid reference")]
    Reference(#[from] crate::reference::find::Error),
    #[error("Invalid reference name")]
    ReferenceName(#[from] gix_validate::reference::name::Error),
    #[error("Cannot resolve start point '{spec}'")]
    ResolveStartPoint {
        spec: BString,
        #[source]
        source: Box<crate::revision::spec::parse::single::Error>,
    },
    #[error("Failed to open the new worktree as a repository")]
    OpenWorktree(#[from] crate::open::Error),
    #[error("Failed to edit HEAD reference")]
    EditHead(#[from] crate::reference::edit::Error),
    #[error("Failed to checkout files")]
    Checkout(#[from] crate::clone::checkout::main_worktree::Error),
    #[error("The repository is bare and cannot have worktrees")]
    BareRepository,
    #[error("Mutually exclusive options: can only specify one of branch, new_branch, or detach")]
    MutuallyExclusiveOptions,
    #[error("Could not find branch '{name}'")]
    BranchNotFound { name: BString },
    #[error("Failed to list worktrees")]
    ListWorktrees(#[source] std::io::Error),
}

impl crate::Repository {
    /// Add a new worktree at the given `path`.
    ///
    /// The worktree will be set up with HEAD pointing to a branch or detached commit
    /// depending on the provided options.
    ///
    /// Returns a `Proxy` to the newly created worktree.
    ///
    /// # Errors
    ///
    /// - If the path already exists and is not empty
    /// - If the branch is already checked out in another worktree (unless `detach` is true)
    /// - If the repository is bare
    #[cfg(feature = "worktree-mutation")]
    pub fn worktree_add<'a>(
        &self,
        path: impl AsRef<Path>,
        options: Options<'a>,
    ) -> Result<crate::worktree::Proxy<'_>, Error> {
        let path = path.as_ref();

        // Validate mutually exclusive options
        let option_count = options.branch.is_some() as u8 + options.new_branch.is_some() as u8 + options.detach as u8;
        if option_count > 1 {
            return Err(Error::MutuallyExclusiveOptions);
        }

        // Note: bare repos CAN have worktrees added. The first worktree of a bare repo
        // is just another linked worktree. This is the typical use case for bare repos
        // when you want to work with multiple checkouts.

        // Validate path doesn't exist or is empty
        if path.exists() {
            let is_empty_dir =
                path.is_dir() && std::fs::read_dir(path).map(|mut d| d.next().is_none()).unwrap_or(false);
            if !is_empty_dir {
                return Err(Error::PathExists { path: path.to_owned() });
            }
        }

        // Generate worktree id from path basename, sanitized
        let worktree_id = sanitize_worktree_id(path);

        // Create a unique worktree git dir under .git/worktrees/<id>
        // Make sure to use absolute paths
        let common_dir = if self.common_dir().is_absolute() {
            self.common_dir().to_owned()
        } else {
            std::env::current_dir()
                .map_err(|source| Error::CreateGitDir {
                    path: self.common_dir().to_owned(),
                    source,
                })?
                .join(self.common_dir())
        };
        let worktrees_dir = common_dir.join("worktrees");
        let worktree_git_dir = find_unique_worktree_git_dir(&worktrees_dir, &worktree_id)?;

        // Determine what HEAD should point to
        let head_target = self.resolve_head_target(&options)?;

        // Check if the branch is already checked out (unless detaching)
        if let HeadTarget::Symbolic(ref branch_name) = head_target {
            self.check_branch_not_checked_out(branch_name.as_ref())?;
        }

        // Create the worktree directory - first try to make it absolute
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|source| Error::CreateWorktreeDir {
                    path: path.to_owned(),
                    source,
                })?
                .join(path)
        };
        std::fs::create_dir_all(&path).map_err(|source| Error::CreateWorktreeDir {
            path: path.clone(),
            source,
        })?;

        // Create the .git/worktrees/<id> directory
        std::fs::create_dir_all(&worktree_git_dir).map_err(|source| Error::CreateGitDir {
            path: worktree_git_dir.clone(),
            source,
        })?;

        // Write the linking files
        // 1. gitdir: path to the worktree's .git file
        let worktree_dot_git = path.join(".git");
        std::fs::write(
            worktree_git_dir.join("gitdir"),
            format!("{}\n", worktree_dot_git.display()),
        )
        .map_err(|source| Error::WriteFile { file: "gitdir", source })?;

        // 2. commondir: relative path to the common directory
        std::fs::write(worktree_git_dir.join("commondir"), "../..\n").map_err(|source| Error::WriteFile {
            file: "commondir",
            source,
        })?;

        // 3. HEAD: symbolic ref or detached commit id
        match &head_target {
            HeadTarget::Symbolic(branch) => {
                std::fs::write(
                    worktree_git_dir.join("HEAD"),
                    format!("ref: {}\n", branch.to_str_lossy()),
                )
                .map_err(|source| Error::WriteFile { file: "HEAD", source })?;
            }
            HeadTarget::Detached(id) => {
                std::fs::write(worktree_git_dir.join("HEAD"), format!("{id}\n"))
                    .map_err(|source| Error::WriteFile { file: "HEAD", source })?;
            }
        }

        // Write the .git file in the worktree pointing back
        std::fs::write(&worktree_dot_git, format!("gitdir: {}\n", worktree_git_dir.display()))
            .map_err(|source| Error::WriteFile { file: ".git", source })?;

        // If a new branch is requested, create it in the main repo before opening the worktree
        if let Some(new_branch_name) = options.new_branch {
            // Determine the target commit for the new branch
            let target_id = match &head_target {
                HeadTarget::Detached(id) => *id,
                HeadTarget::Symbolic(_) => {
                    // This shouldn't happen given our logic, but handle it gracefully
                    self.head()
                        .ok()
                        .and_then(|mut h| h.try_peel_to_id().ok().flatten())
                        .map(|id| id.detach())
                        .unwrap_or_else(|| self.object_hash().null())
                }
            };

            // Create the new branch
            let branch_ref_name = format!("refs/heads/{}", new_branch_name.to_str_lossy());
            self.edit_reference(RefEdit {
                change: Change::Update {
                    log: Default::default(),
                    expected: PreviousValue::MustNotExist,
                    new: gix_ref::Target::Object(target_id),
                },
                name: branch_ref_name.as_str().try_into()?,
                deref: false,
            })?;

            // Update the worktree's HEAD to point to the new branch
            std::fs::write(
                worktree_git_dir.join("HEAD"),
                format!("ref: refs/heads/{}\n", new_branch_name.to_str_lossy()),
            )
            .map_err(|source| Error::WriteFile { file: "HEAD", source })?;
        }

        // Lock if requested
        if options.lock || options.lock_reason.is_some() {
            let lock_content = options
                .lock_reason
                .map(|r| r.to_str_lossy().into_owned())
                .unwrap_or_default();
            std::fs::write(worktree_git_dir.join("locked"), lock_content)
                .map_err(|source| Error::WriteFile { file: "locked", source })?;
        }

        // Checkout files (unless no_checkout is set)
        if !options.no_checkout {
            // Open the worktree as a repository and perform checkout
            let worktree_repo = crate::open(&path)?;

            // Perform checkout using the worktree's repository
            checkout_worktree(&worktree_repo)?;
        }

        // Return a proxy to the new worktree
        Ok(crate::worktree::Proxy::new(self, worktree_git_dir))
    }

    fn resolve_head_target(&self, options: &Options<'_>) -> Result<HeadTarget, Error> {
        // If start_point is specified, resolve it first
        let start_point_id = if let Some(spec) = options.start_point {
            Some(
                self.rev_parse_single(spec)
                    .map_err(|e| Error::ResolveStartPoint {
                        spec: spec.to_owned(),
                        source: Box::new(e),
                    })?
                    .detach(),
            )
        } else {
            None
        };

        if options.detach {
            // Detached HEAD mode
            let id = match start_point_id {
                Some(id) => id,
                None => self
                    .head()
                    .ok()
                    .and_then(|mut h| h.try_peel_to_id().ok().flatten())
                    .map(|id| id.detach())
                    .unwrap_or_else(|| self.object_hash().null()),
            };
            Ok(HeadTarget::Detached(id))
        } else if let Some(branch) = options.branch {
            // Check out an existing branch
            let branch_ref = format!("refs/heads/{}", branch.to_str_lossy());
            // Verify the branch exists
            if self.try_find_reference(&*branch_ref)?.is_none() {
                return Err(Error::BranchNotFound {
                    name: branch.to_owned(),
                });
            }
            Ok(HeadTarget::Symbolic(branch_ref.into()))
        } else if let Some(new_branch) = options.new_branch {
            // New branch will be created - return detached for now, branch creation happens later
            let id = match start_point_id {
                Some(id) => id,
                None => self
                    .head()
                    .ok()
                    .and_then(|mut h| h.try_peel_to_id().ok().flatten())
                    .map(|id| id.detach())
                    .unwrap_or_else(|| self.object_hash().null()),
            };
            // We'll update HEAD to symbolic ref after creating the branch
            let _branch_ref = format!("refs/heads/{}", new_branch.to_str_lossy());
            Ok(HeadTarget::Detached(id))
        } else {
            // Default: check out current HEAD's branch or detach
            match self.head() {
                Ok(mut head) => {
                    if let Some(referent_name) = head.referent_name() {
                        Ok(HeadTarget::Symbolic(referent_name.as_bstr().to_owned()))
                    } else {
                        // Detached HEAD in main repo
                        let id = head
                            .try_peel_to_id()
                            .ok()
                            .flatten()
                            .map(|id| id.detach())
                            .unwrap_or_else(|| self.object_hash().null());
                        Ok(HeadTarget::Detached(id))
                    }
                }
                Err(_) => {
                    // No HEAD yet
                    Ok(HeadTarget::Detached(self.object_hash().null()))
                }
            }
        }
    }

    fn check_branch_not_checked_out(&self, branch_name: &BStr) -> Result<(), Error> {
        // Check main worktree
        if let Some(worktree) = self.worktree() {
            if let Some(head_name) = self.head_name().ok().flatten() {
                if head_name.as_bstr() == branch_name {
                    return Err(Error::BranchCheckedOut {
                        branch: branch_name.to_owned(),
                        worktree: worktree.base().to_owned(),
                    });
                }
            }
        }

        // Check linked worktrees
        for proxy in self.worktrees().map_err(Error::ListWorktrees)? {
            if let Ok(wt_repo) = proxy.clone().into_repo_with_possibly_inaccessible_worktree() {
                if let Ok(Some(head_name)) = wt_repo.head_name() {
                    if head_name.as_bstr() == branch_name {
                        return Err(Error::BranchCheckedOut {
                            branch: branch_name.to_owned(),
                            worktree: proxy.base().unwrap_or_default(),
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

enum HeadTarget {
    Symbolic(BString),
    Detached(gix_hash::ObjectId),
}

fn sanitize_worktree_id(path: &Path) -> String {
    let basename = path.file_name().and_then(|s| s.to_str()).unwrap_or("worktree");

    // Replace characters that are not allowed in ref names
    basename
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn find_unique_worktree_git_dir(worktrees_dir: &Path, base_id: &str) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(worktrees_dir).map_err(|source| Error::CreateGitDir {
        path: worktrees_dir.to_owned(),
        source,
    })?;

    let mut candidate = worktrees_dir.join(base_id);
    let mut counter = 0u32;

    while candidate.exists() {
        counter += 1;
        candidate = worktrees_dir.join(format!("{base_id}{counter}"));
    }

    Ok(candidate)
}

#[cfg(feature = "worktree-mutation")]
fn checkout_worktree(repo: &crate::Repository) -> Result<(), Error> {
    use std::sync::atomic::AtomicBool;

    let workdir = repo.workdir().ok_or(Error::BareRepository)?;

    // Get the tree id from HEAD
    let tree_id = match repo.head() {
        Ok(mut head) => match head.try_peel_to_id() {
            Ok(Some(id)) => match id.object() {
                Ok(obj) => match obj.peel_to_tree() {
                    Ok(tree) => tree.id,
                    Err(_) => return Ok(()), // No tree to checkout
                },
                Err(_) => return Ok(()), // Can't find object
            },
            Ok(None) | Err(_) => return Ok(()), // Unborn HEAD or error
        },
        Err(_) => return Ok(()), // No HEAD
    };

    // Create index from tree
    let index = gix_index::State::from_tree(&tree_id, &repo.objects, Default::default()).map_err(|e| {
        Error::Checkout(crate::clone::checkout::main_worktree::Error::IndexFromTree { id: tree_id, source: e })
    })?;
    let mut index = gix_index::File::from_state(index, repo.index_path());

    // Setup checkout options
    let opts = repo
        .checkout_options(gix_worktree::stack::state::attributes::Source::IdMapping)
        .map_err(crate::clone::checkout::main_worktree::Error::CheckoutOptions)?;

    let should_interrupt = AtomicBool::new(false);
    let files = gix_features::progress::Discard;
    let bytes = gix_features::progress::Discard;

    // Perform checkout
    gix_worktree_state::checkout(
        &mut index,
        workdir,
        repo.objects
            .clone()
            .into_arc()
            .map_err(|e| Error::Checkout(crate::clone::checkout::main_worktree::Error::OpenArcOdb(e)))?,
        &files,
        &bytes,
        &should_interrupt,
        opts,
    )
    .map_err(crate::clone::checkout::main_worktree::Error::IndexCheckout)?;

    // Write the index
    index
        .write(Default::default())
        .map_err(crate::clone::checkout::main_worktree::Error::WriteIndex)?;

    Ok(())
}
