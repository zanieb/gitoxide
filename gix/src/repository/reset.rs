use gix_hash::ObjectId;
use gix_ref::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};

/// The kind of reset to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    /// Move HEAD only; leave index and working tree unchanged.
    Soft,
    /// Move HEAD and reset the index to match the target tree; leave working tree unchanged.
    Mixed,
    /// Move HEAD, reset index, and reset working tree to match the target tree.
    Hard,
}

/// The error returned by [`Repository::reset()`](crate::Repository::reset()).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("Cannot do a soft reset in the middle of a merge")]
    SoftResetDuringMerge,
    #[error("Cannot do a soft reset with unmerged index entries")]
    SoftResetWithUnmergedEntries,
    #[error("Cannot do a mixed or hard reset in a bare repository")]
    BareRepository,
    #[error("Object {id} is not a commit but a {actual}")]
    NotACommit {
        id: gix_hash::ObjectId,
        actual: gix_object::Kind,
    },
    #[error(transparent)]
    FindObject(#[from] crate::object::find::existing::Error),
    #[error(transparent)]
    DecodeCommit(#[from] gix_object::decode::Error),
    #[error(transparent)]
    FindHead(#[from] crate::reference::find::existing::Error),
    #[error(transparent)]
    EditReference(#[from] crate::reference::edit::Error),
    #[error(transparent)]
    OpenIndex(#[from] crate::worktree::open_index::Error),
    #[error(transparent)]
    IndexFromTree(#[from] super::index_from_tree::Error),
    #[error(transparent)]
    WriteIndex(#[from] gix_index::file::write::Error),
    #[error(transparent)]
    CheckoutOptions(#[from] crate::config::checkout_options::Error),
    #[error(transparent)]
    Checkout(#[from] gix_worktree_state::checkout::Error),
    #[error("Failed to convert object database to Arc for thread-safe access")]
    ObjectsToArc(#[from] std::io::Error),
    #[error(transparent)]
    BooleanConfig(#[from] crate::config::boolean::Error),
}

impl crate::Repository {
    /// Reset the repository to the given `commit` with the specified `mode`.
    ///
    /// This implements the behavior of `git reset [--soft|--mixed|--hard] <commit>`:
    ///
    /// - **Soft**: Only moves HEAD (and the branch it points to) to `commit`.
    ///   The index and working tree are left as-is, so previous changes appear staged.
    /// - **Mixed**: Moves HEAD and resets the index to match the tree of `commit`.
    ///   The working tree is left as-is, so previous changes appear unstaged.
    /// - **Hard**: Moves HEAD, resets the index, and resets the working tree to match `commit`.
    ///   All uncommitted changes are discarded.
    ///
    /// In all modes, `ORIG_HEAD` is set to the previous HEAD value before the reset.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The repository is bare and mode is `Mixed` or `Hard`
    /// - There is an unmerged index and mode is `Soft`
    /// - The commit or its tree cannot be found
    /// - The index or working tree cannot be updated
    pub fn reset(&self, commit: impl Into<ObjectId>, mode: ResetMode) -> Result<(), Error> {
        let target_id = commit.into();

        // Soft reset with unmerged index should fail (matching git behavior)
        if mode == ResetMode::Soft {
            if self.state() == Some(crate::state::InProgress::Merge) {
                return Err(Error::SoftResetDuringMerge);
            }
            let index = self.index_or_empty().map_err(Error::OpenIndex)?;
            if index
                .entries()
                .iter()
                .any(|e| e.stage() != gix_index::entry::Stage::Unconflicted)
            {
                return Err(Error::SoftResetWithUnmergedEntries);
            }
        }

        // Mixed/Hard need a worktree (can't be bare)
        if mode != ResetMode::Soft && self.is_bare() {
            return Err(Error::BareRepository);
        }

        // Obtain the tree of the target commit
        let target_commit = self
            .find_object(target_id)?
            .try_into_commit()
            .map_err(|err| Error::NotACommit {
                id: err.id,
                actual: err.actual,
            })?;
        let target_tree_id = target_commit.tree_id().map_err(Error::DecodeCommit)?;

        // Update ORIG_HEAD and HEAD
        self.reset_head_to(target_id)?;

        if mode == ResetMode::Soft {
            return Ok(());
        }

        // Mixed and Hard: reset the index to match the target tree
        let workdir = self.workdir().expect("not bare, checked above").to_owned();

        // For hard reset, read the old index so we can remove files that are no longer tracked.
        let old_index = if mode == ResetMode::Hard {
            self.index_or_empty().ok()
        } else {
            None
        };

        let mut index = self.index_from_tree(&target_tree_id).map_err(Error::IndexFromTree)?;

        if mode == ResetMode::Hard {
            // Remove files from the working tree that are in the old index but not the new one.
            if let Some(old_idx) = &old_index {
                Self::remove_worktree_files_not_in_index(old_idx, &index, &workdir, false);
            }
            // Check out all files from the new index into the working tree
            self.checkout_index_to_worktree_impl(&mut index, &workdir)?;
        }

        // Write the updated index
        index.write(Default::default()).map_err(Error::WriteIndex)?;

        // Clean up merge/rebase state files (matches git's remove_branch_state)
        self.cleanup_state();

        Ok(())
    }

    /// Reset specific paths in the index to match the given `tree_ish`, without moving HEAD.
    ///
    /// This implements `git reset [<tree-ish>] -- <paths>...`, which updates only the index
    /// entries that match the given paths. The working tree is not modified.
    ///
    /// For each path in `paths`:
    /// - If the path exists in the target tree, the index entry is updated to match the tree.
    /// - If the path does not exist in the target tree, the entry is removed from the index.
    ///
    /// This is commonly used as `git reset HEAD -- file.txt` to unstage a file.
    pub fn reset_paths(
        &self,
        tree_ish: impl Into<ObjectId>,
        paths: impl IntoIterator<Item = impl AsRef<std::path::Path>>,
    ) -> Result<(), Error> {
        let target_id = tree_ish.into();

        // Resolve the target to a tree.
        let target_obj = self.find_object(target_id)?;
        let obj_kind = target_obj.kind;
        let obj_id = target_obj.id;
        let target_tree_id: ObjectId = match obj_kind {
            gix_object::Kind::Commit => target_obj
                .try_into_commit()
                .map_err(|err| Error::NotACommit {
                    id: err.id,
                    actual: err.actual,
                })?
                .tree_id()
                .map_err(Error::DecodeCommit)?
                .detach(),
            gix_object::Kind::Tree => obj_id,
            _ => {
                return Err(Error::NotACommit {
                    id: obj_id,
                    actual: obj_kind,
                });
            }
        };

        // Build a new index from the target tree for reference.
        let target_index = self.index_from_tree(&target_tree_id).map_err(Error::IndexFromTree)?;

        // Load the current index.
        let mut index = self.open_index().map_err(Error::OpenIndex)?;

        // Collect the paths we need to reset into a HashSet for O(1) lookup.
        let reset_paths: std::collections::HashSet<Vec<u8>> = paths
            .into_iter()
            .map(|p| {
                let path_bstr = gix_path::into_bstr(p.as_ref().to_owned());
                path_bstr.to_vec()
            })
            .collect();

        // Build a lookup of target entries by path.
        let target_entries: std::collections::HashMap<Vec<u8>, (gix_hash::ObjectId, gix_index::entry::Mode)> =
            target_index
                .entries()
                .iter()
                .filter(|e| {
                    let path: &[u8] = &**e.path(&target_index);
                    reset_paths.contains(path)
                })
                .map(|e| {
                    let path: &[u8] = &**e.path(&target_index);
                    (path.to_vec(), (e.id, e.mode))
                })
                .collect();

        // Update matching entries in the current index.
        for (entry, path) in index.entries_mut_with_paths() {
            let path_bytes: &[u8] = &**path;
            if !reset_paths.contains(path_bytes) {
                continue;
            }
            if let Some(&(id, mode)) = target_entries.get(path_bytes) {
                entry.id = id;
                entry.mode = mode;
                entry
                    .flags
                    .remove(gix_index::entry::Flags::INTENT_TO_ADD | gix_index::entry::Flags::SKIP_WORKTREE);
            } else {
                // Path not in target tree: mark for removal.
                entry.flags.insert(gix_index::entry::Flags::REMOVE);
            }
        }

        // Remove entries marked with REMOVE.
        index.remove_entries(|_, _, e| e.flags.contains(gix_index::entry::Flags::REMOVE));

        // Write the updated index.
        index.write(Default::default()).map_err(Error::WriteIndex)?;

        Ok(())
    }

    /// Move HEAD (and the branch it points to) to `target_id`, saving the
    /// previous HEAD as ORIG_HEAD.
    fn reset_head_to(&self, target_id: ObjectId) -> Result<(), Error> {
        use crate::bstr::BString;

        // Read current HEAD id for ORIG_HEAD
        let prev_head_id = self.head().ok().and_then(|h| h.id().map(|id| id.detach()));

        // Set ORIG_HEAD to the old HEAD
        if let Some(prev_id) = prev_head_id {
            let prev_value = self
                .try_find_reference("ORIG_HEAD")
                .ok()
                .flatten()
                .map(|r| PreviousValue::MustExistAndMatch(r.inner.target.clone()))
                .unwrap_or(PreviousValue::Any);
            self.edit_reference(RefEdit {
                change: Change::Update {
                    log: LogChange {
                        mode: RefLog::AndReference,
                        force_create_reflog: false,
                        message: BString::from("updating ORIG_HEAD"),
                    },
                    expected: prev_value,
                    new: gix_ref::Target::Object(prev_id),
                },
                name: "ORIG_HEAD".try_into().expect("valid ref name"),
                deref: false,
            })
            .map_err(Error::EditReference)?;
        }

        // Update HEAD (following the symbolic reference if it points to one)
        let reflog_message = BString::from(format!("reset: moving to {target_id}"));
        let head = self.head().map_err(Error::FindHead)?;

        match &head.kind {
            crate::head::Kind::Symbolic(reference) => {
                // HEAD points to a branch, update the branch ref
                self.edit_reference(RefEdit {
                    change: Change::Update {
                        log: LogChange {
                            mode: RefLog::AndReference,
                            force_create_reflog: false,
                            message: reflog_message,
                        },
                        expected: PreviousValue::Any,
                        new: gix_ref::Target::Object(target_id),
                    },
                    name: reference.name.clone(),
                    deref: false,
                })
                .map_err(Error::EditReference)?;
            }
            crate::head::Kind::Unborn(name) => {
                // Create the branch
                self.edit_reference(RefEdit {
                    change: Change::Update {
                        log: LogChange {
                            mode: RefLog::AndReference,
                            force_create_reflog: false,
                            message: reflog_message,
                        },
                        expected: PreviousValue::Any,
                        new: gix_ref::Target::Object(target_id),
                    },
                    name: name.clone(),
                    deref: false,
                })
                .map_err(Error::EditReference)?;
            }
            crate::head::Kind::Detached { .. } => {
                // HEAD is detached, update HEAD directly
                self.edit_reference(RefEdit {
                    change: Change::Update {
                        log: LogChange {
                            mode: RefLog::AndReference,
                            force_create_reflog: false,
                            message: reflog_message,
                        },
                        expected: PreviousValue::Any,
                        new: gix_ref::Target::Object(target_id),
                    },
                    name: "HEAD".try_into().expect("valid ref name"),
                    deref: false,
                })
                .map_err(Error::EditReference)?;
            }
        }

        Ok(())
    }

    /// Remove state files left by merge, cherry-pick, revert, etc.
    /// Equivalent to git's `remove_branch_state()`.
    ///
    /// Errors from removing individual files are intentionally ignored, matching
    /// git's behavior -- these files may not exist and that is expected.
    fn cleanup_state(&self) {
        let git_dir = self.path();
        for name in &[
            "MERGE_HEAD",
            "MERGE_MSG",
            "MERGE_MODE",
            "MERGE_RR",
            "SQUASH_MSG",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "AUTO_MERGE",
        ] {
            let _ = std::fs::remove_file(git_dir.join(name));
        }
        // Clean up sequencer state directory (used by cherry-pick/revert --continue).
        let sequencer_dir = git_dir.join("sequencer");
        if sequencer_dir.is_dir() {
            let _ = std::fs::remove_dir_all(&sequencer_dir);
        }
    }
}

impl From<super::worktree_ops::CheckoutError> for Error {
    fn from(err: super::worktree_ops::CheckoutError) -> Self {
        match err {
            super::worktree_ops::CheckoutError::CheckoutOptions(e) => Error::CheckoutOptions(e),
            super::worktree_ops::CheckoutError::Checkout(e) => Error::Checkout(e),
            super::worktree_ops::CheckoutError::ObjectsToArc(e) => Error::ObjectsToArc(e),
        }
    }
}
