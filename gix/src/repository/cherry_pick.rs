use gix_hash::ObjectId;
use gix_merge::blob::builtin_driver::text::Labels;

use crate::Repository;

/// Options for cherry-pick and revert operations.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// If `true`, stage the result but do not create a commit (`--no-commit`).
    pub no_commit: bool,
    /// For merge commits, specifies which parent to use as the mainline (1-indexed, like `git cherry-pick -m`).
    ///
    /// - `None`: use the first parent (only valid for non-merge commits; merge commits will error).
    /// - `Some(1)`: use the first parent as the mainline.
    /// - `Some(2)`: use the second parent as the mainline.
    ///
    /// When cherry-picking or reverting a merge commit, this must be set.
    /// When the commit is not a merge, this must be `None`.
    pub mainline: Option<u32>,
}

/// The error returned by [`Repository::cherry_pick()`] and [`Repository::revert()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("cannot cherry-pick in a bare repository")]
    BareRepository,
    #[error(transparent)]
    FindHead(#[from] crate::reference::head_id::Error),
    #[error(transparent)]
    FindObject(#[from] crate::object::find::existing::Error),
    #[error(transparent)]
    DecodeCommit(#[from] gix_object::decode::Error),
    #[error("commit {id} has no parent (cannot cherry-pick a root commit)")]
    NoParent { id: ObjectId },
    #[error("commit {id} is a merge but no -m option was provided")]
    MergeCommitWithoutMainline { id: ObjectId },
    #[error("mainline was specified but commit {id} is not a merge")]
    MainlineForNonMerge { id: ObjectId },
    #[error("mainline parent {mainline} is out of range for commit {id} which has {num_parents} parent(s)")]
    MainlineOutOfRange {
        id: ObjectId,
        mainline: u32,
        num_parents: usize,
    },
    #[error(transparent)]
    MergeTree(#[from] crate::repository::merge_trees::Error),
    #[error(transparent)]
    TreeMergeOptions(#[from] crate::repository::tree_merge_options::Error),
    #[error("cherry-pick or revert resulted in conflicts")]
    Conflict,
    #[error(transparent)]
    WriteObject(#[from] crate::object::write::Error),
    #[error(transparent)]
    EditReference(#[from] crate::reference::edit::Error),
    #[error("committer identity is not configured")]
    CommitterMissing,
    #[error(transparent)]
    ParseTime(#[from] crate::config::time::Error),
    #[error(transparent)]
    OpenIndex(#[from] crate::worktree::open_index::Error),
    #[error(transparent)]
    IndexFromTree(#[from] crate::repository::index_from_tree::Error),
    #[error(transparent)]
    WriteIndex(#[from] gix_index::file::write::Error),
    #[error(transparent)]
    CheckoutOptions(#[from] crate::config::checkout_options::Error),
    #[error(transparent)]
    Checkout(#[from] gix_worktree_state::checkout::Error),
    #[error("failed to convert object database to Arc for thread-safe access")]
    ObjectsToArc(#[from] std::io::Error),
    #[error(transparent)]
    WriteTree(#[from] crate::object::tree::editor::write::Error),
    #[error(transparent)]
    FindHeadExisting(#[from] crate::reference::find::existing::Error),
}

/// The outcome of a cherry-pick or revert operation.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The OID of the newly created commit, or `None` if `--no-commit` was used.
    pub commit_id: Option<ObjectId>,
    /// The OID of the resulting tree.
    pub tree_id: ObjectId,
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

impl From<super::worktree_ops::UpdateHeadError> for Error {
    fn from(err: super::worktree_ops::UpdateHeadError) -> Self {
        match err {
            super::worktree_ops::UpdateHeadError::FindHead(e) => Error::FindHeadExisting(e),
            super::worktree_ops::UpdateHeadError::EditReference(e) => Error::EditReference(e),
        }
    }
}

impl Repository {
    /// Resolve the ancestor (parent) tree ID for a cherry-pick or revert.
    ///
    /// Handles root commits (empty tree), merge commits (mainline selection),
    /// and normal single-parent commits.
    fn resolve_parent_tree_id(
        &self,
        commit_id: ObjectId,
        parent_ids: &[gix_hash::ObjectId],
        mainline: Option<u32>,
    ) -> Result<ObjectId, Error> {
        let num_parents = parent_ids.len();

        if num_parents == 0 {
            // Root commit: use the empty tree as ancestor.
            return Ok(ObjectId::empty_tree(self.object_hash()));
        }

        if num_parents > 1 {
            // Merge commit: mainline must be specified.
            let m = mainline.ok_or(Error::MergeCommitWithoutMainline { id: commit_id })?;
            if m == 0 || m as usize > num_parents {
                return Err(Error::MainlineOutOfRange {
                    id: commit_id,
                    mainline: m,
                    num_parents,
                });
            }
            let parent_id = parent_ids[m as usize - 1];
            let parent_tree_id = self
                .find_object(parent_id)?
                .try_into_commit()
                .map_err(|_| Error::FindObject(crate::object::find::existing::Error::NotFound { oid: parent_id }))?
                .tree_id()?
                .detach();
            return Ok(parent_tree_id);
        }

        // Single-parent commit.
        if mainline.is_some() {
            return Err(Error::MainlineForNonMerge { id: commit_id });
        }
        let parent_id = parent_ids[0];
        let parent_tree_id = self
            .find_object(parent_id)?
            .try_into_commit()
            .map_err(|_| Error::FindObject(crate::object::find::existing::Error::NotFound { oid: parent_id }))?
            .tree_id()?
            .detach();
        Ok(parent_tree_id)
    }

    /// Cherry-pick a single commit onto HEAD.
    ///
    /// This implements `git cherry-pick <commit>`:
    ///
    /// 1. Performs a 3-way merge with the commit's parent as ancestor, HEAD as "ours",
    ///    and the commit as "theirs".
    /// 2. If there are no conflicts, writes the resulting tree and creates a new commit
    ///    with the original commit's message and author.
    /// 3. Updates the index and working tree to match the result.
    /// 4. Writes `.git/CHERRY_PICK_HEAD` and `.git/MERGE_MSG` state files during the
    ///    operation, cleaning them up on successful commit.
    ///
    /// Use `options.no_commit` to stage the result without committing (state files are
    /// left in place for `git status` to show "cherry-pick in progress").
    ///
    /// Use `options.mainline` to cherry-pick a merge commit (1-indexed parent number).
    ///
    /// Root commits (no parents) are supported by using the empty tree as ancestor.
    ///
    /// Returns an error if there are unresolved conflicts.
    pub fn cherry_pick(&self, commit_id: impl Into<ObjectId>, options: Options) -> Result<Outcome, Error> {
        if self.is_bare() {
            return Err(Error::BareRepository);
        }

        let pick_id = commit_id.into();
        let head_id = self.head_id()?;

        // Read the commit being picked.
        let pick_commit = self
            .find_object(pick_id)?
            .try_into_commit()
            .map_err(|_| Error::FindObject(crate::object::find::existing::Error::NotFound { oid: pick_id }))?;
        let parent_ids: Vec<_> = pick_commit.parent_ids().map(|id| id.detach()).collect();

        // Resolve ancestor tree (handles root, merge, and normal commits).
        let parent_tree_id = self.resolve_parent_tree_id(pick_id, &parent_ids, options.mainline)?;

        // Get tree IDs.
        let pick_tree_id = pick_commit.tree_id()?;
        let head_tree_id = self
            .find_object(head_id)?
            .try_into_commit()
            .map_err(|_| Error::FindObject(crate::object::find::existing::Error::NotFound { oid: head_id.detach() }))?
            .tree_id()?;

        // Write state files before the merge so they persist on conflict/error.
        let cherry_pick_head_path = self.git_dir().join("CHERRY_PICK_HEAD");
        let merge_msg_path = self.git_dir().join("MERGE_MSG");
        let message_bytes = pick_commit.message_raw_sloppy().to_vec();

        std::fs::write(&cherry_pick_head_path, format!("{pick_id}\n"))?;
        std::fs::write(&merge_msg_path, &message_bytes)?;

        // 3-way merge: ancestor=parent, ours=HEAD, theirs=commit.
        let merge_options = self.tree_merge_options()?;
        let labels = Labels {
            ancestor: Some("parent-of-picked".into()),
            current: Some("HEAD".into()),
            other: Some("cherry-picked".into()),
        };
        let mut outcome = self.merge_trees(parent_tree_id, head_tree_id, pick_tree_id, labels, merge_options)?;

        let how = crate::merge::tree::TreatAsUnresolved::default();
        if outcome.has_unresolved_conflicts(how) {
            return Err(Error::Conflict);
        }

        // Write the merged tree.
        let result_tree_id = outcome.tree.write()?.detach();

        // Update index and worktree.
        self.update_index_and_worktree_to_tree(result_tree_id)?;

        if options.no_commit {
            // Leave state files in place for "cherry-pick in progress" status.
            return Ok(Outcome {
                commit_id: None,
                tree_id: result_tree_id,
            });
        }

        // Create the cherry-pick commit with original author but our committer.
        let pick_author_ref = pick_commit.author()?;
        let pick_author: gix_actor::Signature = pick_author_ref.into();
        let committer_ref = self
            .committer()
            .ok_or(Error::CommitterMissing)?
            .map_err(Error::ParseTime)?;
        let committer: gix_actor::Signature = committer_ref.into();

        let new_commit = gix_object::Commit {
            tree: result_tree_id,
            parents: smallvec::smallvec![head_id.detach()],
            author: pick_author,
            committer,
            encoding: None,
            message: message_bytes.into(),
            extra_headers: Default::default(),
        };
        let new_commit_id = self.write_object(&new_commit)?;

        // Update HEAD.
        self.update_head_ref(new_commit_id.detach(), "cherry-pick")?;

        // Clean up state files on success.
        let _ = std::fs::remove_file(&cherry_pick_head_path);
        let _ = std::fs::remove_file(&merge_msg_path);

        Ok(Outcome {
            commit_id: Some(new_commit_id.detach()),
            tree_id: result_tree_id,
        })
    }

    /// Revert a single commit, undoing its changes on HEAD.
    ///
    /// This implements `git revert <commit>`:
    ///
    /// 1. Performs a 3-way merge with the commit as ancestor, HEAD as "ours",
    ///    and the commit's parent as "theirs".
    /// 2. If there are no conflicts, writes the resulting tree and creates a new commit
    ///    with a "Revert" message.
    /// 3. Updates the index and working tree to match the result.
    /// 4. Writes `.git/REVERT_HEAD` and `.git/MERGE_MSG` state files during the
    ///    operation, cleaning them up on successful commit.
    ///
    /// Use `options.no_commit` to stage the result without committing (state files are
    /// left in place for `git status` to show "revert in progress").
    ///
    /// Use `options.mainline` to revert a merge commit (1-indexed parent number).
    ///
    /// Returns an error if there are unresolved conflicts.
    pub fn revert(&self, commit_id: impl Into<ObjectId>, options: Options) -> Result<Outcome, Error> {
        if self.is_bare() {
            return Err(Error::BareRepository);
        }

        let revert_id = commit_id.into();
        let head_id = self.head_id()?;

        // Read the commit being reverted.
        let revert_commit = self
            .find_object(revert_id)?
            .try_into_commit()
            .map_err(|_| Error::FindObject(crate::object::find::existing::Error::NotFound { oid: revert_id }))?;
        let parent_ids: Vec<_> = revert_commit.parent_ids().map(|id| id.detach()).collect();

        // Resolve ancestor tree (handles merge and normal commits).
        // For revert, we swap: ancestor = commit tree, theirs = parent tree.
        // But first we need the parent tree for the "theirs" side.
        let parent_tree_id = self.resolve_parent_tree_id(revert_id, &parent_ids, options.mainline)?;

        // Get tree IDs.
        let revert_tree_id = revert_commit.tree_id()?;
        let head_tree_id = self
            .find_object(head_id)?
            .try_into_commit()
            .map_err(|_| Error::FindObject(crate::object::find::existing::Error::NotFound { oid: head_id.detach() }))?
            .tree_id()?;

        // Build the revert commit message.
        let orig_message = revert_commit.message_raw_sloppy();
        let first_line = orig_message
            .iter()
            .position(|&b| b == b'\n')
            .map(|pos| &orig_message[..pos])
            .unwrap_or(orig_message);
        let first_line = String::from_utf8_lossy(first_line);
        let message = format!("Revert \"{first_line}\"\n\nThis reverts commit {revert_id}.\n");

        // Write state files before the merge so they persist on conflict/error.
        let revert_head_path = self.git_dir().join("REVERT_HEAD");
        let merge_msg_path = self.git_dir().join("MERGE_MSG");

        std::fs::write(&revert_head_path, format!("{revert_id}\n"))?;
        std::fs::write(&merge_msg_path, &message)?;

        // 3-way merge: ancestor=commit, ours=HEAD, theirs=parent.
        // This reverses the commit's changes.
        let merge_options = self.tree_merge_options()?;
        let labels = Labels {
            ancestor: Some("reverted-commit".into()),
            current: Some("HEAD".into()),
            other: Some("parent-of-reverted".into()),
        };
        let mut outcome = self.merge_trees(revert_tree_id, head_tree_id, parent_tree_id, labels, merge_options)?;

        let how = crate::merge::tree::TreatAsUnresolved::default();
        if outcome.has_unresolved_conflicts(how) {
            return Err(Error::Conflict);
        }

        // Write the merged tree.
        let result_tree_id = outcome.tree.write()?.detach();

        // Update index and worktree.
        self.update_index_and_worktree_to_tree(result_tree_id)?;

        if options.no_commit {
            // Leave state files in place for "revert in progress" status.
            return Ok(Outcome {
                commit_id: None,
                tree_id: result_tree_id,
            });
        }

        // Create the revert commit.
        let committer_ref = self
            .committer()
            .ok_or(Error::CommitterMissing)?
            .map_err(Error::ParseTime)?;
        let committer: gix_actor::Signature = committer_ref.into();

        let new_commit = gix_object::Commit {
            tree: result_tree_id,
            parents: smallvec::smallvec![head_id.detach()],
            author: committer.clone(),
            committer,
            encoding: None,
            message: message.into(),
            extra_headers: Default::default(),
        };
        let new_commit_id = self.write_object(&new_commit)?;

        // Update HEAD.
        self.update_head_ref(new_commit_id.detach(), "revert")?;

        // Clean up state files on success.
        let _ = std::fs::remove_file(&revert_head_path);
        let _ = std::fs::remove_file(&merge_msg_path);

        Ok(Outcome {
            commit_id: Some(new_commit_id.detach()),
            tree_id: result_tree_id,
        })
    }

    /// Update the index and working tree to match the given tree.
    fn update_index_and_worktree_to_tree(&self, tree_id: ObjectId) -> Result<(), Error> {
        let workdir = self.workdir().expect("not bare, checked above").to_owned();

        // Read the old index so we can detect deleted files.
        let old_index = self.open_index().ok();

        let mut index = self.index_from_tree(&tree_id)?;

        // Remove files from the worktree that are in the old index but not in the new one.
        // Path traversal check is enabled to prevent deleting files outside the worktree.
        if let Some(old_idx) = &old_index {
            Self::remove_worktree_files_not_in_index(old_idx, &index, &workdir, true);
        }

        // Write the new index.
        index.write(Default::default())?;

        // Check out files to the working tree.
        self.checkout_index_to_worktree_impl(&mut index, &workdir)?;

        Ok(())
    }
}
