use std::{path::Path, sync::atomic::AtomicBool};

use crate::{
    Result,
    bstr::{BStr, BString, ByteSlice},
    worktree::{
        Proxy,
        add::{Error, Head},
    },
};
use gix_error::{ResultExt, ensure, message};
use gix_ref::transaction::PreviousValue;

/// Options for the [`Repository::worktree_add()`](crate::Repository::worktree_add()) method.
#[derive(Debug, Default, Clone)]
pub struct Options<'a> {
    /// The name of an existing branch to check out in the new worktree.
    /// Mutually exclusive with `new_branch` and `detach`.
    pub branch: Option<&'a BStr>,
    /// Create a new branch with this name before checking out.
    /// Mutually exclusive with `branch` and `detach`.
    pub new_branch: Option<&'a BStr>,
    /// Create or reset a branch with this name before checking it out.
    /// Mutually exclusive with `branch`, `new_branch`, and `detach`.
    pub reset_branch: Option<&'a BStr>,
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
impl crate::Repository {
    /// Add a linked worktree using branch-selection options like `git worktree add`.
    pub fn worktree_add(&self, path: impl AsRef<Path>, options: Options<'_>) -> Result<Proxy<'_>> {
        self.worktree_add_with_progress(path, options, gix_utils::progress::Discard, &AtomicBool::new(false))
    }

    /// Add a linked worktree, reporting checkout progress and honoring cancellation.
    pub fn worktree_add_with_progress<P>(
        &self,
        path: impl AsRef<Path>,
        options: Options<'_>,
        progress: P,
        interrupt: &AtomicBool,
    ) -> Result<Proxy<'_>>
    where
        P: gix_utils::progress::NestedProgress,
        P::SubProgress: gix_utils::progress::NestedProgress + 'static,
    {
        let path = path.as_ref();
        let mut repo = self.clone();
        repo.clear_namespace();
        let choices = [
            options.branch.is_some(),
            options.new_branch.is_some(),
            options.reset_branch.is_some(),
            options.detach,
        ];
        ensure!(
            choices.into_iter().filter(|set| *set).count() <= 1,
            gix_error::validation("Only one branch-selection option may be used")
        );
        let inferred = inferred_branch_name(path);
        let target = repo.resolve_head_target(&options, inferred.as_ref())?;
        let head = match target {
            HeadTarget::Detached(id) => Head::Detached(id),
            HeadTarget::Symbolic { branch, create } => {
                let short = branch.strip_prefix(b"refs/heads/").expect("local branch");
                ensure!(
                    !short.starts_with(b"-"),
                    gix_error::validation("Branch names must not start with '-'")
                );
                gix_validate::reference::branch_name(branch.as_bstr()).or_error()?;
                let name: gix_ref::FullName = gix_ref::FullName::try_from(branch).or_error()?;
                if let Some(occupied) = repo.checked_out_branches_without_namespace()?.remove(&name) {
                    gix_error::bail!(Error::CheckedOut {
                        name,
                        worktree_dirs: occupied
                    });
                }
                match create {
                    BranchCreation::Unborn => Head::Unborn(name),
                    BranchCreation::None => Head::Attached(name),
                    BranchCreation::At(id) | BranchCreation::ResetAt(id) => {
                        let previous = if matches!(create, BranchCreation::At(_)) {
                            PreviousValue::MustNotExist
                        } else {
                            repo.try_find_reference(name.as_ref())?
                                .map_or(PreviousValue::MustNotExist, |r| {
                                    PreviousValue::MustExistAndMatch(r.inner.target)
                                })
                        };
                        let mut reflog = BString::from(if matches!(create, BranchCreation::ResetAt(_)) {
                            "branch: Reset to "
                        } else {
                            "branch: Created from "
                        });
                        reflog.extend_from_slice(options.start_point.unwrap_or_else(|| b"HEAD".as_bstr()));
                        repo.reference(name.clone(), id, previous, reflog)?;
                        Head::Attached(name)
                    }
                }
            }
        };
        let prepared = repo.prepare_add_worktree(path, head, interrupt)?;
        let linked = if options.no_checkout {
            prepared.persist()?
        } else {
            let (linked, checkout) = prepared.checkout(progress, interrupt)?;
            if let Some(err) = checkout.errors.into_iter().next() {
                return Err(err.error).or_raise(|| message!("Failed to check out {:?}", err.path));
            }
            if let Some(collision) = checkout.collisions.into_iter().next() {
                return Err(std::io::Error::from(collision.error_kind))
                    .or_raise(|| message!("Checkout collision at {:?}", collision.path));
            }
            ensure!(
                checkout.delayed_paths_unprocessed.is_empty() && checkout.delayed_paths_unknown.is_empty(),
                message!(
                    "Checkout filters left unprocessed paths {:?} and returned unexpected paths {:?}",
                    checkout.delayed_paths_unprocessed,
                    checkout.delayed_paths_unknown
                )
            );
            linked
        };
        let proxy = Proxy::new(self, linked.git_dir());
        if options.lock || options.lock_reason.is_some() {
            proxy.set_lock(Some(options.lock_reason.unwrap_or_else(|| b"".as_bstr())))?;
        }
        Ok(proxy)
    }
    fn resolve_head_target(&self, options: &Options<'_>, default_branch_name: &BStr) -> Result<HeadTarget> {
        // If start_point is specified, resolve it first
        let start_point_id = if let Some(spec) = options.start_point {
            Some(
                self.rev_parse_single(spec)
                    .or_raise(|| message!("Could not resolve worktree start point {spec}"))?
                    .object()?
                    .peel_to_commit()?
                    .id,
            )
        } else {
            None
        };

        if options.detach {
            // Detached HEAD mode
            let id = start_point_id
                .or(self.current_head_id()?)
                .ok_or_else(|| gix_error::message!("Cannot detach or reset an unborn HEAD").validation_error())?;
            Ok(HeadTarget::Detached(id))
        } else if let Some(branch) = options.branch {
            // Check out an existing branch
            let branch_ref = local_branch_ref(branch);
            // Verify the branch exists
            if self.try_find_reference(branch_ref.as_bstr())?.is_none() {
                gix_error::bail!(message!("Branch {branch} does not exist").not_found());
            }
            Ok(HeadTarget::Symbolic {
                branch: branch_ref,
                create: BranchCreation::None,
            })
        } else if let Some(new_branch) = options.new_branch {
            let branch_ref = local_branch_ref(new_branch);
            if self.try_find_reference(branch_ref.as_bstr())?.is_some() {
                gix_error::bail!(message!("Branch {new_branch} already exists").conflict());
            }
            Ok(HeadTarget::Symbolic {
                branch: branch_ref,
                create: start_point_id
                    .or(self.current_head_id()?)
                    .map_or(BranchCreation::Unborn, BranchCreation::At),
            })
        } else if let Some(reset_branch) = options.reset_branch {
            let branch_ref = local_branch_ref(reset_branch);
            let target_id = start_point_id
                .or(self.current_head_id()?)
                .ok_or_else(|| gix_error::message!("Cannot detach or reset an unborn HEAD").validation_error())?;
            Ok(HeadTarget::Symbolic {
                branch: branch_ref,
                create: BranchCreation::ResetAt(target_id),
            })
        } else if let Some(id) = start_point_id {
            let branch_ref = match options.start_point {
                Some(spec) => self.existing_local_branch_for_start_point(spec)?,
                None => None,
            };
            if let Some(branch_ref) = branch_ref {
                Ok(HeadTarget::Symbolic {
                    branch: branch_ref,
                    create: BranchCreation::None,
                })
            } else {
                Ok(HeadTarget::Detached(id))
            }
        } else {
            // Default: check out an existing branch named after the worktree path, or create it.
            let branch_ref = local_branch_ref(default_branch_name);
            if self.try_find_reference(branch_ref.as_bstr())?.is_some() {
                Ok(HeadTarget::Symbolic {
                    branch: branch_ref,
                    create: BranchCreation::None,
                })
            } else {
                Ok(HeadTarget::Symbolic {
                    branch: branch_ref,
                    create: self
                        .current_head_id()?
                        .map_or(BranchCreation::Unborn, BranchCreation::At),
                })
            }
        }
    }
    fn current_head_id(&self) -> Result<Option<gix_hash::ObjectId>> {
        Ok(self.head()?.try_peel_to_id()?.map(crate::Id::detach))
    }
    fn existing_local_branch_for_start_point(&self, spec: &BStr) -> Result<Option<BString>> {
        let Some(branch_ref) = local_branch_ref_for_start_point(spec) else {
            return Ok(None);
        };
        if gix_validate::reference::name(branch_ref.as_bstr()).is_err() {
            return Ok(None);
        }
        Ok(self.try_find_reference(branch_ref.as_bstr())?.map(|_| branch_ref))
    }
}
enum HeadTarget {
    Symbolic { branch: BString, create: BranchCreation },
    Detached(gix_hash::ObjectId),
}
enum BranchCreation {
    None,
    At(gix_hash::ObjectId),
    ResetAt(gix_hash::ObjectId),
    Unborn,
}
fn inferred_branch_name(path: &Path) -> BString {
    path.file_name()
        .and_then(|name| gix_path::os_str_into_bstr(name).ok())
        .map_or_else(|| "worktree".into(), ToOwned::to_owned)
}
fn local_branch_ref(name: &BStr) -> BString {
    let mut branch_ref = BString::from("refs/heads/");
    branch_ref.extend_from_slice(name);
    branch_ref
}
fn local_branch_ref_for_start_point(spec: &BStr) -> Option<BString> {
    if spec.starts_with_str("refs/heads/") {
        Some(spec.to_owned())
    } else if spec.starts_with_str("heads/") {
        let mut branch_ref = BString::from("refs/");
        branch_ref.extend_from_slice(spec);
        Some(branch_ref)
    } else if spec.starts_with_str("refs/") {
        None
    } else {
        Some(local_branch_ref(spec))
    }
}
