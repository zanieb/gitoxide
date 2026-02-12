use crate::bstr::BString;
use crate::ext::ObjectIdExt;
use crate::{Id, Repository};
use gix_hash::ObjectId;

/// Validate that a path constructed from a tree/index entry stays within the worktree.
/// Rejects paths containing `..` components that could escape the worktree boundary.
///
/// Security: Mitigates path traversal via crafted tree entries (e.g. `../../etc/passwd`).
/// Similar in spirit to CVE-2018-11235 (malicious .gitmodules path traversal).
fn validate_path_within_worktree(path: &[u8]) -> Result<(), ApplyError> {
    use std::path::Component;
    let os_path = gix_path::from_bstr(<&[u8] as Into<&crate::bstr::BStr>>::into(path));
    for component in os_path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(ApplyError::PathTraversal {
                path: BString::from(path),
            });
        }
    }
    Ok(())
}

/// A single entry from the stash reflog.
#[derive(Debug)]
pub struct StashEntry<'repo> {
    /// The stash commit object id.
    pub commit_id: Id<'repo>,
    /// The reflog message, e.g. "WIP on main: abc1234 Some commit message".
    pub message: crate::bstr::BString,
    /// The reflog index (0 = most recent stash, 1 = next, etc).
    pub index: usize,
}

/// The error returned by stash list operations.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ListError {
    #[error(transparent)]
    FindReference(#[from] crate::reference::find::Error),
    #[error("Could not iterate reflog")]
    ReflogIter(#[from] std::io::Error),
}

/// Options for [`Repository::stash_save_opts()`].
#[derive(Debug, Clone, Default)]
pub struct StashSaveOptions<'a> {
    /// An optional message for the stash entry. If `None`, a default "WIP on ..." message is used.
    pub message: Option<&'a str>,
    /// If `true`, staged changes are left in the index after saving (like `git stash --keep-index`).
    ///
    /// The worktree is still reset, but the index retains its staged state. This is useful
    /// for testing staged changes while stashing the rest.
    pub keep_index: bool,
    /// If `true`, untracked files are included in the stash (like `git stash --include-untracked`).
    ///
    /// Untracked files (not ignored, not in the index) are captured as a third parent commit
    /// on the stash commit, matching C Git's format. After stashing, the untracked files
    /// are removed from the worktree.
    pub include_untracked: bool,
}

/// The error returned by [`Repository::stash_save()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum SaveError {
    #[error("Nothing to stash: the working tree is clean")]
    NothingToStash,
    #[error("Cannot stash in a bare repository")]
    BareRepository,
    #[error("No HEAD commit to base stash on (unborn branch)")]
    NoHeadCommit,
    #[error(transparent)]
    FindHead(#[from] crate::reference::head_id::Error),
    #[error(transparent)]
    FindObject(#[from] crate::object::find::existing::Error),
    #[error(transparent)]
    DecodeCommit(#[from] gix_object::decode::Error),
    #[error(transparent)]
    OpenIndex(#[from] crate::worktree::open_index::Error),
    #[error("Failed to write tree from index")]
    WriteTree(#[from] gix_index::write_tree::Error),
    #[error(transparent)]
    WriteObject(#[from] crate::object::write::Error),
    #[error(transparent)]
    EditReference(#[from] crate::reference::edit::Error),
    #[error("Committer identity is not configured")]
    CommitterMissing,
    #[error(transparent)]
    ParseTime(#[from] crate::config::time::Error),
    #[error(transparent)]
    FindHeadExisting(#[from] crate::reference::find::existing::Error),
    #[error("Failed to reset index/worktree after saving stash")]
    Reset(#[from] super::reset::Error),
    #[error("Failed to read worktree file")]
    ReadWorktreeFile(std::io::Error),
    #[error(transparent)]
    IndexFromTree(#[from] crate::repository::index_from_tree::Error),
    #[error(transparent)]
    HashObject(#[from] gix_hash::hasher::Error),
    #[error("Failed to write index")]
    WriteIndex(#[from] gix_index::file::write::Error),
    #[error("Failed to remove untracked file from worktree")]
    RemoveFile(std::io::Error),
}

/// The error returned by [`Repository::stash_apply()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ApplyError {
    #[error("No stash entries found")]
    NoStash,
    #[error("Stash index {index} is out of range (only {count} entries)")]
    IndexOutOfRange { index: usize, count: usize },
    #[error(transparent)]
    List(#[from] ListError),
    #[error(transparent)]
    FindObject(#[from] crate::object::find::existing::Error),
    #[error(transparent)]
    DecodeCommit(#[from] gix_object::decode::Error),
    #[error(transparent)]
    OpenIndex(#[from] crate::worktree::open_index::Error),
    #[error(transparent)]
    IndexFromTree(#[from] crate::repository::index_from_tree::Error),
    #[error(transparent)]
    WriteIndex(#[from] gix_index::file::write::Error),
    #[error("Cannot apply stash in a bare repository")]
    BareRepository,
    #[error("Your local changes to the following files would be overwritten by stash apply: {paths}")]
    Conflict { paths: BString },
    #[error(transparent)]
    CheckoutOptions(#[from] crate::config::checkout_options::Error),
    #[error(transparent)]
    Checkout(#[from] gix_worktree_state::checkout::Error),
    #[error("Failed to convert object database to Arc for thread-safe access")]
    ObjectsToArc(#[from] std::io::Error),
    #[error("Path traversal rejected: entry path '{path}' contains '..' components")]
    PathTraversal { path: BString },
}

/// The error returned by [`Repository::stash_pop()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum PopError {
    #[error(transparent)]
    Apply(#[from] ApplyError),
    #[error(transparent)]
    Drop(#[from] DropError),
}

/// The error returned by [`Repository::stash_drop()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum DropError {
    #[error("No stash entries found")]
    NoStash,
    #[error("Stash index {index} is out of range (only {count} entries)")]
    IndexOutOfRange { index: usize, count: usize },
    #[error(transparent)]
    List(#[from] ListError),
    #[error(transparent)]
    EditReference(#[from] crate::reference::edit::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Could not acquire lock for reflog or ref update")]
    LockAcquire(#[from] gix_lock::acquire::Error),
    #[error("Could not commit lock file")]
    LockCommit(String),
}

impl From<super::worktree_ops::CheckoutError> for ApplyError {
    fn from(err: super::worktree_ops::CheckoutError) -> Self {
        match err {
            super::worktree_ops::CheckoutError::CheckoutOptions(e) => ApplyError::CheckoutOptions(e),
            super::worktree_ops::CheckoutError::Checkout(e) => ApplyError::Checkout(e),
            super::worktree_ops::CheckoutError::ObjectsToArc(e) => ApplyError::ObjectsToArc(e),
        }
    }
}

impl From<super::worktree_ops::CheckoutError> for SaveError {
    fn from(err: super::worktree_ops::CheckoutError) -> Self {
        match err {
            super::worktree_ops::CheckoutError::CheckoutOptions(e) => {
                SaveError::Reset(super::reset::Error::CheckoutOptions(e))
            }
            super::worktree_ops::CheckoutError::Checkout(e) => SaveError::Reset(super::reset::Error::Checkout(e)),
            super::worktree_ops::CheckoutError::ObjectsToArc(e) => SaveError::ReadWorktreeFile(e),
        }
    }
}

impl Repository {
    /// List all stash entries by iterating the reflog of `refs/stash`.
    ///
    /// Returns an empty list if there are no stashes.
    /// The most recent stash is at index 0.
    pub fn stash_list(&self) -> Result<Vec<StashEntry<'_>>, ListError> {
        let reference = match self.try_find_reference("refs/stash")? {
            Some(r) => r,
            None => return Ok(Vec::new()),
        };

        let mut entries = Vec::new();
        let mut log_iter = reference.log_iter();
        let Some(log) = log_iter.rev()? else {
            return Ok(entries);
        };

        for (index, entry) in log.enumerate() {
            let Ok(entry) = entry else { continue };
            entries.push(StashEntry {
                commit_id: entry.new_oid.attach(self),
                message: entry.message,
                index,
            });
        }
        Ok(entries)
    }

    /// Save the current state of the index and working tree to the stash.
    ///
    /// This is a convenience wrapper around [`stash_save_opts()`](Self::stash_save_opts)
    /// with default options (no keep-index, no include-untracked).
    ///
    /// See [`stash_save_opts()`](Self::stash_save_opts) for full documentation.
    pub fn stash_save(&self, message: Option<&str>) -> Result<Id<'_>, SaveError> {
        self.stash_save_opts(StashSaveOptions {
            message,
            ..Default::default()
        })
    }

    /// Save the current state of the index and working tree to the stash, with options.
    ///
    /// This implements the core of `git stash push` / `git stash save`:
    ///
    /// 1. Creates an "index commit" whose tree matches the current index and whose parent is HEAD.
    /// 2. Creates a "stash commit" whose tree reflects the **worktree** state
    ///    (including both staged and unstaged modifications), with parents `[HEAD, index_commit]`
    ///    (or `[HEAD, index_commit, untracked_commit]` when `include_untracked` is set).
    /// 3. Updates `refs/stash` to point at the stash commit, with a reflog entry.
    /// 4. Resets the index and working tree back to HEAD (hard reset), unless `keep_index` is set,
    ///    in which case only unstaged changes are reset (the index retains its staged state).
    ///
    /// Returns the id of the newly created stash commit.
    ///
    /// # Commit Structure
    ///
    /// Matches C Git's stash format:
    /// - Index commit: tree = index state, parent = HEAD
    /// - Stash commit: tree = worktree state, parents = [HEAD, index_commit] (2 parents)
    /// - With `include_untracked`: parents = [HEAD, index_commit, untracked_commit] (3 parents)
    ///
    /// # Errors
    ///
    /// Returns [`SaveError::BareRepository`] if called on a bare repository.
    /// Returns [`SaveError::NothingToStash`] if neither the index nor the worktree
    /// differs from HEAD and there are no untracked files to capture.
    /// Returns [`SaveError::NoHeadCommit`] if HEAD does not point to a valid commit.
    /// Returns [`SaveError::CommitterMissing`] if the committer identity is not configured.
    /// Other variants cover object lookup/write, index I/O, reference editing,
    /// worktree file read, and reset failures.
    pub fn stash_save_opts(&self, options: StashSaveOptions<'_>) -> Result<Id<'_>, SaveError> {
        if self.is_bare() {
            return Err(SaveError::BareRepository);
        }

        let head_id = self.head_id().map_err(SaveError::FindHead)?;
        let head_commit = self
            .find_object(head_id)?
            .try_into_commit()
            .map_err(|_| SaveError::NoHeadCommit)?;
        let head_tree_id = head_commit.tree_id().map_err(SaveError::DecodeCommit)?;

        // Read the current index and write its tree.
        // Use index_or_load_from_head to handle repos where .git/index doesn't exist yet
        // (e.g. newly initialized repos where no `git add` has been done).
        let mut index = match self.open_index() {
            Ok(idx) => idx,
            Err(_) => self.index_from_tree(&head_tree_id)?,
        };
        let outcome = index.write_tree_to(|tree| self.write_object(tree).map(|id| id.detach()))?;
        let index_tree_id = outcome.tree_id;

        // Build a worktree tree that reflects the actual file contents on disk.
        // Start from the index and overlay any worktree modifications.
        let workdir = self.workdir().ok_or(SaveError::BareRepository)?.to_owned();
        let worktree_tree_id = self.build_worktree_tree(&index, &workdir, index_tree_id)?;

        // Collect untracked files if requested.
        let untracked_files = if options.include_untracked {
            self.collect_untracked_files(&index, &workdir)?
        } else {
            Vec::new()
        };

        // If neither the index nor the worktree differs from HEAD, and there are no
        // untracked files to capture, there's nothing to stash.
        let has_staged_changes = index_tree_id != head_tree_id.detach();
        let has_worktree_changes = worktree_tree_id != index_tree_id;
        let has_untracked = !untracked_files.is_empty();
        if !has_staged_changes && !has_worktree_changes && !has_untracked {
            return Err(SaveError::NothingToStash);
        }

        // Build the reflog message.
        let branch_name = self
            .head_name()
            .ok()
            .flatten()
            .map(|n| n.shorten().to_string())
            .unwrap_or_else(|| "(no branch)".to_string());

        let head_id_hex = head_id.to_string();
        let head_id_short = &head_id_hex[..7.min(head_id_hex.len())];
        let head_msg_raw = head_commit.message_raw_sloppy();
        let first_newline = head_msg_raw.iter().position(|&b| b == b'\n');
        let head_msg_first_line: &crate::bstr::BStr = match first_newline {
            Some(pos) => head_msg_raw[..pos].as_ref(),
            None => head_msg_raw[..].as_ref(),
        };

        let reflog_message = if let Some(msg) = options.message {
            // Sanitize newlines in the reflog message to match C Git/libgit2 behavior.
            // Reflog entries are line-oriented, so newlines must be collapsed to spaces.
            let sanitized_msg = msg.replace('\n', " ").replace('\r', " ");
            format!("On {branch_name}: {sanitized_msg}")
        } else {
            format!("WIP on {branch_name}: {head_id_short} {head_msg_first_line}")
        };

        // Get committer identity for the stash commits.
        let committer_ref = self
            .committer()
            .ok_or(SaveError::CommitterMissing)?
            .map_err(SaveError::ParseTime)?;
        let committer: gix_actor::Signature = committer_ref.into();

        // Create the index commit: parent=HEAD, tree=index_tree.
        let index_commit = gix_object::Commit {
            tree: index_tree_id,
            parents: smallvec::smallvec![head_id.detach()],
            author: committer.clone(),
            committer: committer.clone(),
            encoding: None,
            message: format!("index on {branch_name}: {head_id_short} {head_msg_first_line}").into(),
            extra_headers: Default::default(),
        };
        let index_commit_id = self.write_object(&index_commit)?;

        // Build parents list for the stash commit.
        let mut parents = smallvec::smallvec![head_id.detach(), index_commit_id.detach()];

        // If include_untracked is set, always create the third parent commit.
        // C Git and libgit2 always produce a 3-parent stash when --include-untracked
        // is requested, even if there are no untracked files (using the empty tree).
        // Consumers check parentcount == 3 to detect the untracked format.
        if options.include_untracked {
            let untracked_tree_id = if has_untracked {
                self.build_untracked_tree(&untracked_files)?
            } else {
                gix_hash::ObjectId::empty_tree(self.object_hash())
            };
            let untracked_commit = gix_object::Commit {
                tree: untracked_tree_id,
                parents: smallvec::smallvec![],
                author: committer.clone(),
                committer: committer.clone(),
                encoding: None,
                message: format!("untracked files on {branch_name}: {head_id_short} {head_msg_first_line}").into(),
                extra_headers: Default::default(),
            };
            let untracked_commit_id = self.write_object(&untracked_commit)?;
            parents.push(untracked_commit_id.detach());
        }

        // Create the stash commit: parents=[HEAD, index_commit, ?untracked], tree=worktree_tree.
        let stash_commit = gix_object::Commit {
            tree: worktree_tree_id,
            parents,
            author: committer.clone(),
            committer,
            encoding: None,
            message: reflog_message.clone().into(),
            extra_headers: Default::default(),
        };
        let stash_commit_id = self.write_object(&stash_commit)?;

        // Update refs/stash with a reflog entry.
        // TODO(CODE-REVIEW#27): This reflog manipulation does not hold a lock across
        // the read-modify-write of refs/stash. Concurrent stash operations could race.
        // Consider using a ref transaction with proper locking semantics.
        use gix_ref::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};

        let prev_value = self
            .try_find_reference("refs/stash")
            .ok()
            .flatten()
            .map(|r| PreviousValue::MustExistAndMatch(r.inner.target.clone()))
            .unwrap_or(PreviousValue::Any);

        self.edit_reference(RefEdit {
            change: Change::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    force_create_reflog: true,
                    message: reflog_message.into(),
                },
                expected: prev_value,
                new: gix_ref::Target::Object(stash_commit_id.detach()),
            },
            name: "refs/stash".try_into().expect("valid ref name"),
            deref: false,
        })
        .map_err(SaveError::EditReference)?;

        // Reset behavior depends on keep_index.
        if options.keep_index {
            // Reset the worktree to HEAD, but keep the index as-is.
            // We do a hard reset first, then restore the index to its pre-stash state.
            self.reset(head_id.detach(), super::reset::ResetMode::Hard)?;

            // Restore the original index (staged changes remain).
            let mut restored_index = self.index_from_tree(&index_tree_id)?;
            restored_index
                .write(Default::default())
                .map_err(SaveError::WriteIndex)?;

            // Check out the index state to the worktree so staged files are visible.
            self.checkout_index_to_worktree_impl(&mut restored_index, &workdir)?;
        } else {
            // Standard behavior: reset both index and worktree to HEAD.
            self.reset(head_id.detach(), super::reset::ResetMode::Hard)?;
        }

        // Remove untracked files from the worktree after stashing them.
        if options.include_untracked && has_untracked {
            for (path, _) in &untracked_files {
                let file_path = workdir.join(gix_path::from_bstr(<&[u8] as Into<&crate::bstr::BStr>>::into(
                    path.as_slice(),
                )));
                if file_path.exists() {
                    std::fs::remove_file(&file_path).map_err(SaveError::RemoveFile)?;
                }
            }
        }

        Ok(stash_commit_id)
    }

    /// Apply a stash entry by its index (0 = most recent) to the current index and working tree.
    ///
    /// This implements the core of `git stash apply [stash@{n}]`:
    ///
    /// 1. Reads the stash commit and its first parent (HEAD at stash time).
    /// 2. Computes which index entries were changed by the stash (diff stash-parent-tree vs stash-tree).
    /// 3. Applies those changes to the current index, failing if any affected path has local modifications.
    /// 4. Writes the updated index and checks out changed files to the worktree.
    ///
    /// The stash entry is **not** removed; use [`stash_pop()`](Self::stash_pop) for apply+drop.
    ///
    /// # Limitations
    ///
    /// The stash save implementation captures full worktree state (via a separate worktree tree),
    /// and apply diffs the stash's worktree tree against the parent tree to restore changes to
    /// both the index and working tree. However, three-way merge conflict handling and
    /// `--index` mode (restoring staged vs unstaged distinction) are not yet implemented.
    ///
    /// # Errors
    ///
    /// Returns [`ApplyError::BareRepository`] if called on a bare repository.
    /// Returns [`ApplyError::NoStash`] if there are no stash entries.
    /// Returns [`ApplyError::IndexOutOfRange`] if `index` exceeds the number of stash entries.
    /// Returns [`ApplyError::Conflict`] if applying the stash would overwrite local modifications.
    /// Returns [`ApplyError::PathTraversal`] if an untracked file path contains `..` components.
    /// Other variants cover object lookup, commit decoding, index I/O, and checkout failures.
    pub fn stash_apply(&self, index: usize) -> Result<(), ApplyError> {
        if self.is_bare() {
            return Err(ApplyError::BareRepository);
        }

        let entries = self.stash_list().map_err(ApplyError::List)?;
        if entries.is_empty() {
            return Err(ApplyError::NoStash);
        }
        let entry = entries.get(index).ok_or(ApplyError::IndexOutOfRange {
            index,
            count: entries.len(),
        })?;

        // Read the stash commit to get its tree and parent.
        let stash_commit = self
            .find_object(entry.commit_id.detach())?
            .try_into_commit()
            .map_err(|_| ApplyError::NoStash)?;
        let stash_tree_id = stash_commit.tree_id().map_err(ApplyError::DecodeCommit)?;

        let parent_ids = stash_commit.parent_ids().collect::<Vec<_>>();
        let stash_parent_id = parent_ids.first().ok_or(ApplyError::NoStash)?.detach();

        let stash_parent_commit = self
            .find_object(stash_parent_id)?
            .try_into_commit()
            .map_err(|_| ApplyError::NoStash)?;
        let stash_parent_tree_id = stash_parent_commit.tree_id().map_err(ApplyError::DecodeCommit)?;

        // Build indexes from the stash parent tree and stash tree to compute the diff.
        // The indexes are sorted, so we use binary search instead of building HashMaps.
        let parent_index = self
            .index_from_tree(&stash_parent_tree_id)
            .map_err(ApplyError::IndexFromTree)?;
        let stash_index = self
            .index_from_tree(&stash_tree_id)
            .map_err(ApplyError::IndexFromTree)?;

        // Compute which entries changed in the stash vs its parent using binary search
        // on sorted index entries, avoiding HashMap construction with cloned paths.
        let unconflicted = gix_index::entry::Stage::Unconflicted;
        let mut changed: Vec<(Vec<u8>, ObjectId, gix_index::entry::Mode)> = Vec::new();
        let mut deleted: Vec<Vec<u8>> = Vec::new();

        for stash_entry in stash_index.entries() {
            let path = stash_entry.path(&stash_index);
            match parent_index.entry_by_path_and_stage(path, unconflicted) {
                Some(parent_entry) => {
                    if stash_entry.id != parent_entry.id || stash_entry.mode != parent_entry.mode {
                        let path_bytes: &[u8] = &**path;
                        changed.push((path_bytes.to_vec(), stash_entry.id, stash_entry.mode));
                    }
                }
                None => {
                    let path_bytes: &[u8] = &**path;
                    changed.push((path_bytes.to_vec(), stash_entry.id, stash_entry.mode));
                }
            }
        }

        for parent_entry in parent_index.entries() {
            let path = parent_entry.path(&parent_index);
            if stash_index.entry_by_path_and_stage(path, unconflicted).is_none() {
                let path_bytes: &[u8] = &**path;
                deleted.push(path_bytes.to_vec());
            }
        }

        // Collect untracked files from the third parent (if any).
        let mut untracked_files: Vec<(Vec<u8>, ObjectId)> = Vec::new();
        if parent_ids.len() >= 3 {
            let untracked_parent_id = parent_ids[2].detach();
            let untracked_commit = self
                .find_object(untracked_parent_id)?
                .try_into_commit()
                .map_err(|_| ApplyError::NoStash)?;
            let untracked_tree_id = untracked_commit.tree_id().map_err(ApplyError::DecodeCommit)?;
            let untracked_index = self
                .index_from_tree(&untracked_tree_id)
                .map_err(ApplyError::IndexFromTree)?;
            for untracked_entry in untracked_index.entries() {
                let path: &[u8] = &**untracked_entry.path(&untracked_index);
                untracked_files.push((path.to_vec(), untracked_entry.id));
            }
        }

        if changed.is_empty() && deleted.is_empty() && untracked_files.is_empty() {
            return Ok(());
        }

        // Load the current index and check for conflicts.
        // Handle repos where .git/index doesn't exist yet by loading from the parent tree.
        let mut current_index = match self.open_index() {
            Ok(idx) => idx,
            Err(_) => self
                .index_from_tree(&stash_parent_tree_id)
                .map_err(ApplyError::IndexFromTree)?,
        };

        // Get the workdir for worktree conflict checks.
        let workdir = self.workdir().ok_or(ApplyError::BareRepository)?.to_owned();

        // Verify no changed path has local modifications that would be overwritten.
        // Check both index-level changes AND worktree-level changes.
        // Uses binary search on sorted indexes instead of HashMaps.
        let mut conflicts: Vec<BString> = Vec::new();
        for (path, stash_id, _mode) in &changed {
            use crate::bstr::ByteSlice;
            let path_bstr = path.as_bstr();
            if let Some(current_entry) = current_index.entry_by_path_and_stage(path_bstr, unconflicted) {
                // Check index-level conflict: current index differs from stash parent.
                if let Some(parent_entry) = parent_index.entry_by_path_and_stage(path_bstr, unconflicted) {
                    if current_entry.id != parent_entry.id || current_entry.mode != parent_entry.mode {
                        conflicts.push(BString::from(path.as_slice()));
                        continue;
                    }
                }

                // Check worktree-level conflict: file on disk differs from current index.
                let file_path = workdir.join(gix_path::from_bstr(<&[u8] as Into<&crate::bstr::BStr>>::into(
                    path.as_slice(),
                )));
                if let Ok(content) = std::fs::read(&file_path) {
                    if let Ok(worktree_oid) =
                        gix_object::compute_hash(self.object_hash(), gix_object::Kind::Blob, &content)
                    {
                        if worktree_oid != current_entry.id {
                            conflicts.push(BString::from(path.as_slice()));
                        }
                    }
                }
            } else {
                // Path is not in the current index (new file from stash).
                // Check if it exists in the worktree -- if so, it's a conflict
                // unless the worktree content matches what the stash would write.
                let file_path = workdir.join(gix_path::from_bstr(<&[u8] as Into<&crate::bstr::BStr>>::into(
                    path.as_slice(),
                )));
                if let Ok(content) = std::fs::read(&file_path) {
                    if let Ok(worktree_oid) =
                        gix_object::compute_hash(self.object_hash(), gix_object::Kind::Blob, &content)
                    {
                        if worktree_oid != *stash_id {
                            conflicts.push(BString::from(path.as_slice()));
                        }
                    }
                }
            }
        }
        for path in &deleted {
            use crate::bstr::ByteSlice;
            let path_bstr = path.as_bstr();
            if let Some(current_entry) = current_index.entry_by_path_and_stage(path_bstr, unconflicted) {
                if let Some(parent_entry) = parent_index.entry_by_path_and_stage(path_bstr, unconflicted) {
                    if current_entry.id != parent_entry.id || current_entry.mode != parent_entry.mode {
                        conflicts.push(BString::from(path.as_slice()));
                        continue;
                    }
                }

                // Check worktree-level conflict for deleted paths too.
                let file_path = workdir.join(gix_path::from_bstr(<&[u8] as Into<&crate::bstr::BStr>>::into(
                    path.as_slice(),
                )));
                if let Ok(content) = std::fs::read(&file_path) {
                    if let Ok(worktree_oid) =
                        gix_object::compute_hash(self.object_hash(), gix_object::Kind::Blob, &content)
                    {
                        if worktree_oid != current_entry.id {
                            conflicts.push(BString::from(path.as_slice()));
                        }
                    }
                }
            }
        }

        // Check untracked files for conflicts: if a file from the untracked commit
        // already exists in the worktree with different content, it's a conflict.
        for (path, blob_id) in &untracked_files {
            let file_path = workdir.join(gix_path::from_bstr(<&[u8] as Into<&crate::bstr::BStr>>::into(
                path.as_slice(),
            )));
            if let Ok(content) = std::fs::read(&file_path) {
                if let Ok(worktree_oid) = gix_object::compute_hash(self.object_hash(), gix_object::Kind::Blob, &content)
                {
                    if worktree_oid != *blob_id {
                        conflicts.push(BString::from(path.as_slice()));
                    }
                }
            }
        }

        if !conflicts.is_empty() {
            let mut joined = BString::new(Vec::new());
            for (i, p) in conflicts.iter().enumerate() {
                if i > 0 {
                    joined.extend_from_slice(b", ");
                }
                joined.extend_from_slice(p);
            }
            return Err(ApplyError::Conflict { paths: joined });
        }

        // Apply changes to the current index using binary search for lookups.
        for (path, new_id, new_mode) in &changed {
            use crate::bstr::ByteSlice;
            let path_bstr = path.as_bstr();
            if let Some(idx) = current_index.entry_index_by_path_and_stage(path_bstr, unconflicted) {
                current_index.entries_mut()[idx].id = *new_id;
                current_index.entries_mut()[idx].mode = *new_mode;
            } else {
                current_index.add_entry(
                    gix_index::entry::Stat::default(),
                    *new_id,
                    gix_index::entry::Flags::empty(),
                    *new_mode,
                    path_bstr,
                );
            }
        }

        // Remove deleted entries.
        if !deleted.is_empty() {
            current_index.remove_entries(|_, path, _entry| {
                let path_bytes: &[u8] = &**path;
                deleted.iter().any(|dp| dp.as_slice() == path_bytes)
            });
        }

        current_index.sort_entries();

        // Write the updated index.
        current_index
            .write(Default::default())
            .map_err(ApplyError::WriteIndex)?;

        // Check out affected files to the worktree.
        self.checkout_index_to_worktree_impl(&mut current_index, &workdir)?;

        // Restore untracked files from the third parent.
        for (path, blob_id) in &untracked_files {
            // Reject paths with '..' components to prevent writing outside worktree.
            validate_path_within_worktree(path)?;
            let file_path = workdir.join(gix_path::from_bstr(<&[u8] as Into<&crate::bstr::BStr>>::into(
                path.as_slice(),
            )));
            // Create parent directories if needed.
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).map_err(ApplyError::ObjectsToArc)?;
            }
            let blob = self.find_object(*blob_id)?;
            let blob_data: &[u8] = blob.data.as_ref();
            std::fs::write(&file_path, blob_data).map_err(ApplyError::ObjectsToArc)?;
        }

        Ok(())
    }

    /// Apply a stash entry and then drop it. This is `git stash pop`.
    ///
    /// If the apply fails, the stash entry is **not** dropped.
    pub fn stash_pop(&self, index: usize) -> Result<(), PopError> {
        self.stash_apply(index).map_err(PopError::Apply)?;
        self.stash_drop(index).map_err(PopError::Drop)?;
        Ok(())
    }

    /// Drop a stash entry by its index (0 = most recent).
    ///
    /// This implements `git stash drop [stash@{n}]`.
    ///
    /// If this was the only stash, the `refs/stash` reference and its reflog are removed.
    /// Otherwise the reflog is rewritten to exclude the dropped entry, and the ref is
    /// updated to point at the new most-recent stash.
    pub fn stash_drop(&self, index: usize) -> Result<(), DropError> {
        let entries = self.stash_list().map_err(DropError::List)?;
        if entries.is_empty() {
            return Err(DropError::NoStash);
        }
        if index >= entries.len() {
            return Err(DropError::IndexOutOfRange {
                index,
                count: entries.len(),
            });
        }

        use gix_ref::transaction::{Change, PreviousValue, RefEdit, RefLog};

        if entries.len() == 1 {
            // Last stash entry: delete the ref entirely.
            self.edit_reference(RefEdit {
                change: Change::Delete {
                    expected: PreviousValue::Any,
                    log: RefLog::AndReference,
                },
                name: "refs/stash".try_into().expect("valid ref name"),
                deref: false,
            })
            .map_err(DropError::EditReference)?;

            // Also remove the reflog file.
            let reflog_path = self.common_dir().join("logs").join("refs").join("stash");
            let _ = std::fs::remove_file(reflog_path);
        } else {
            // Rewrite the reflog to remove the entry at the given index.
            // The reflog is stored chronologically (oldest first), but stash entries
            // are presented in reverse order (newest first). So stash@{0} is the
            // last line of the reflog file.
            let reflog_path = self.common_dir().join("logs").join("refs").join("stash");
            let content = std::fs::read(&reflog_path)?;

            let mut lines: Vec<&[u8]> = content.split(|&b| b == b'\n').collect();
            // Remove trailing empty element from final newline.
            if lines.last().is_some_and(|l| l.is_empty()) {
                lines.pop();
            }

            if lines.is_empty() {
                return Err(DropError::NoStash);
            }

            // stash@{0} = last line, stash@{1} = second to last, etc.
            let line_index = lines.len().checked_sub(1 + index).ok_or(DropError::IndexOutOfRange {
                index,
                count: lines.len(),
            })?;
            lines.remove(line_index);

            // Build the new reflog content.
            let mut new_content = Vec::with_capacity(content.len());
            for line in &lines {
                new_content.extend_from_slice(line);
                new_content.push(b'\n');
            }

            // Use gix_lock for atomic reflog rewrite.
            use std::io::Write;
            let mut reflog_lock =
                gix_lock::File::acquire_to_update_resource(&reflog_path, gix_lock::acquire::Fail::Immediately, None)?;
            reflog_lock.write_all(&new_content)?;
            reflog_lock
                .commit()
                .map_err(|e| DropError::LockCommit(e.error.to_string()))?;

            // Update refs/stash to point at the new most recent stash (last reflog line).
            // Use gix_lock for atomic ref update.
            // TODO: This bypasses the ref transaction layer (edit_reference), which means hooks
            // (reference-transaction) won't fire and alternative ref backends (e.g., reftable)
            // won't be updated. This should be refactored once the ref transaction API supports
            // reflog entry removal.
            if let Some(last_line) = lines.last() {
                if let Some(new_stash_id) = parse_reflog_new_oid(last_line, self.object_hash()) {
                    let ref_path = self.common_dir().join("refs").join("stash");
                    let hex = new_stash_id.to_string();
                    let mut ref_lock = gix_lock::File::acquire_to_update_resource(
                        &ref_path,
                        gix_lock::acquire::Fail::Immediately,
                        None,
                    )?;
                    ref_lock.write_all(format!("{hex}\n").as_bytes())?;
                    ref_lock
                        .commit()
                        .map_err(|e| DropError::LockCommit(e.error.to_string()))?;
                }
            }
        }

        Ok(())
    }

    /// Collect untracked files in the worktree (not in the index, not ignored).
    ///
    /// Uses the gitignore infrastructure (`gix-worktree` excludes stack) to skip ignored
    /// files and directories (e.g. `target/`, `node_modules/`). Without this, `--include-untracked`
    /// would capture every non-indexed file including build artifacts.
    ///
    /// Files are written as blobs to the ODB as they are discovered, so only the ObjectId
    /// is stored per file rather than the full file content. This reduces peak memory usage
    /// for repos with large untracked files.
    ///
    /// Returns a list of (path, blob_oid) pairs.
    fn collect_untracked_files(
        &self,
        index: &gix_index::File,
        workdir: &std::path::Path,
    ) -> Result<Vec<(Vec<u8>, ObjectId)>, SaveError> {
        use std::collections::HashSet;

        // Build a set of all paths in the index for fast lookup.
        let indexed_paths: HashSet<Vec<u8>> = index
            .entries()
            .iter()
            .map(|e| {
                let path: &[u8] = &**e.path(index);
                path.to_vec()
            })
            .collect();

        // Set up the excludes stack to check .gitignore patterns.
        let excludes_stack = self.excludes(
            index,
            None,
            gix_worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
        );

        let mut untracked = Vec::new();
        match excludes_stack {
            Ok(mut stack) => {
                self.walk_worktree_for_untracked(workdir, workdir, &indexed_paths, &mut untracked, Some(&mut stack))?;
            }
            Err(_) => {
                // If we cannot set up excludes (e.g. missing config), fall back to
                // the simple walk that only skips .git.
                self.walk_worktree_for_untracked(workdir, workdir, &indexed_paths, &mut untracked, None)?;
            }
        }
        Ok(untracked)
    }

    /// Recursively walk the worktree to find untracked files.
    ///
    /// When `excludes` is provided, files and directories matching .gitignore patterns are skipped.
    /// Each discovered file is immediately written as a blob to the ODB, storing only the ObjectId
    /// rather than holding all file contents in memory.
    ///
    /// The walk is limited to `MAX_WALK_DEPTH` levels to prevent stack overflow on
    /// adversarial directory structures.
    fn walk_worktree_for_untracked(
        &self,
        root: &std::path::Path,
        dir: &std::path::Path,
        indexed_paths: &std::collections::HashSet<Vec<u8>>,
        out: &mut Vec<(Vec<u8>, ObjectId)>,
        excludes: Option<&mut crate::AttributeStack<'_>>,
    ) -> Result<(), SaveError> {
        self.walk_worktree_for_untracked_inner(root, dir, indexed_paths, out, excludes, 0)
    }

    /// Maximum directory depth for worktree walks (security: prevents stack overflow).
    const MAX_WALK_DEPTH: usize = 1024;

    fn walk_worktree_for_untracked_inner(
        &self,
        root: &std::path::Path,
        dir: &std::path::Path,
        indexed_paths: &std::collections::HashSet<Vec<u8>>,
        out: &mut Vec<(Vec<u8>, ObjectId)>,
        mut excludes: Option<&mut crate::AttributeStack<'_>>,
        depth: usize,
    ) -> Result<(), SaveError> {
        if depth >= Self::MAX_WALK_DEPTH {
            return Ok(());
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };

        // Collect entries so we can iterate while holding `excludes`.
        let mut dir_entries: Vec<_> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(SaveError::ReadWorktreeFile)?;
            dir_entries.push(entry);
        }

        // Determine which entries to skip via gitignore, then process them.
        // We check all entries first, collecting skip flags, so the mutable borrow
        // on the excludes stack is released before we recurse.
        let mut skip_flags = vec![false; dir_entries.len()];
        let mut relative_paths: Vec<Vec<u8>> = Vec::with_capacity(dir_entries.len());
        for (i, entry) in dir_entries.iter().enumerate() {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let relative_bytes: Vec<u8> = relative.to_string_lossy().replace('\\', "/").into_bytes();

            // Skip .git directory.
            if path.file_name().is_some_and(|n| n == ".git") {
                skip_flags[i] = true;
                relative_paths.push(relative_bytes);
                continue;
            }

            // Check gitignore if excludes stack is available.
            if let Some(ref mut stack) = excludes {
                let file_type = entry.file_type().map_err(SaveError::ReadWorktreeFile)?;
                let mode = if file_type.is_dir() {
                    Some(gix_index::entry::Mode::DIR)
                } else {
                    None
                };
                if let Ok(platform) = stack.at_path(
                    gix_path::from_bstr(<&[u8] as Into<&crate::bstr::BStr>>::into(relative_bytes.as_slice())),
                    mode,
                ) {
                    if platform.is_excluded() {
                        skip_flags[i] = true;
                    }
                }
            }
            relative_paths.push(relative_bytes);
        }

        // Now process the entries, recursing into non-skipped directories.
        for (i, entry) in dir_entries.iter().enumerate() {
            if skip_flags[i] {
                continue;
            }
            let file_type = entry.file_type().map_err(SaveError::ReadWorktreeFile)?;
            let path = entry.path();

            if file_type.is_dir() {
                self.walk_worktree_for_untracked_inner(
                    root,
                    &path,
                    indexed_paths,
                    out,
                    excludes.as_deref_mut(),
                    depth + 1,
                )?;
            } else if file_type.is_file() {
                if !indexed_paths.contains(&relative_paths[i]) {
                    // Write blob to ODB immediately to avoid holding file contents in memory.
                    let content = std::fs::read(&path).map_err(SaveError::ReadWorktreeFile)?;
                    let blob_id = self.write_blob(&content)?.detach();
                    out.push((relative_paths[i].clone(), blob_id));
                }
            }
        }
        Ok(())
    }

    /// Build a tree from a list of untracked files whose blobs are already in the ODB.
    ///
    /// Builds a tree hierarchy from the (path, blob_oid) pairs.
    /// Paths may contain directory separators (e.g., `subdir/file.txt`).
    fn build_untracked_tree(&self, files: &[(Vec<u8>, ObjectId)]) -> Result<ObjectId, SaveError> {
        use std::collections::BTreeMap;

        // Build a nested map: directory -> (filename -> blob_oid).
        // This handles nested paths like "a/b/file.txt".
        #[derive(Default)]
        struct DirNode {
            files: BTreeMap<Vec<u8>, ObjectId>,
            dirs: BTreeMap<Vec<u8>, DirNode>,
        }

        let mut root = DirNode::default();

        for (path, blob_id) in files {
            // Split path into components.
            let parts: Vec<&[u8]> = path.split(|&b| b == b'/').collect();
            let (filename, dir_parts) = parts.split_last().expect("path is not empty");

            let mut node = &mut root;
            for dir in dir_parts {
                node = node.dirs.entry(dir.to_vec()).or_default();
            }
            node.files.insert(filename.to_vec(), *blob_id);
        }

        // Recursively write trees bottom-up.
        fn write_tree_node(node: &DirNode, repo: &Repository) -> Result<ObjectId, SaveError> {
            let mut entries = Vec::new();

            // Add subdirectories first.
            for (name, subdir) in &node.dirs {
                let subtree_id = write_tree_node(subdir, repo)?;
                entries.push(gix_object::tree::Entry {
                    mode: gix_object::tree::EntryKind::Tree.into(),
                    filename: name.as_slice().into(),
                    oid: subtree_id,
                });
            }

            // Add files.
            for (name, oid) in &node.files {
                entries.push(gix_object::tree::Entry {
                    mode: gix_object::tree::EntryKind::Blob.into(),
                    filename: name.as_slice().into(),
                    oid: *oid,
                });
            }

            entries.sort();
            let tree = gix_object::Tree { entries };
            let tree_id = repo.write_object(&tree)?.detach();
            Ok(tree_id)
        }

        write_tree_node(&root, self)
    }

    /// Build a tree object that represents the current worktree state.
    ///
    /// This starts from the index and checks each tracked file against its on-disk version.
    /// For files whose worktree content differs from the index entry, a new blob is written
    /// with the worktree content. The result is a tree that reflects both staged (index) and
    /// unstaged (worktree) modifications.
    ///
    /// Uses stat-based change detection: only files whose filesystem stat (mtime, ctime, size)
    /// differs from the index entry's stored stat are read and hashed. This avoids reading
    /// every tracked file, providing a ~N/M speedup where N is total files and M is changed files.
    ///
    /// If no worktree files differ from the index, `index_tree_id` is returned unchanged.
    fn build_worktree_tree(
        &self,
        index: &gix_index::File,
        workdir: &std::path::Path,
        index_tree_id: ObjectId,
    ) -> Result<ObjectId, SaveError> {
        use std::collections::HashMap;

        let stat_options = gix_index::entry::stat::Options::default();
        let mut worktree_overrides: HashMap<Vec<u8>, ObjectId> = HashMap::new();

        for entry in index.entries() {
            if entry.stage() != gix_index::entry::Stage::Unconflicted {
                continue;
            }
            if entry.mode != gix_index::entry::Mode::FILE && entry.mode != gix_index::entry::Mode::FILE_EXECUTABLE {
                continue;
            }

            let path = entry.path(index);
            let file_path = workdir.join(gix_path::from_bstr(path));

            // Use stat-based change detection: compare filesystem metadata against the
            // index entry's stored stat. Only read the file if stat indicates a change.
            let fs_meta = match gix_index::fs::Metadata::from_path_no_follow(&file_path) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // File deleted from worktree but still in index - use index version.
                    continue;
                }
                Err(e) => return Err(SaveError::ReadWorktreeFile(e)),
            };

            let fs_stat = match gix_index::entry::Stat::from_fs(&fs_meta) {
                Ok(s) => s,
                Err(_) => {
                    // If stat conversion fails (e.g. time before epoch), fall back to reading.
                    gix_index::entry::Stat::default()
                }
            };

            if entry.stat.matches(&fs_stat, stat_options) {
                // Stat matches the index entry -- file is unchanged, skip it.
                continue;
            }

            // Stat differs, read and hash the file to confirm it actually changed.
            let content = match std::fs::read(&file_path) {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(SaveError::ReadWorktreeFile(e)),
            };

            let worktree_oid = gix_object::compute_hash(self.object_hash(), gix_object::Kind::Blob, &content)?;
            if worktree_oid != entry.id {
                let blob_id = self.write_blob(&content)?;
                let path_bytes: &[u8] = &**path;
                worktree_overrides.insert(path_bytes.to_vec(), blob_id.detach());
            }
        }

        // Also capture files with conflict stage entries (stages 1/2/3) that exist on disk.
        // During a merge conflict, the index has conflict entries but no unconflicted entry.
        // If the file exists on disk (e.g. with conflict markers), capture the worktree version.
        // This matches libgit2's stash_delta_merge behavior.
        {
            use std::collections::HashSet;
            // Collect paths that have conflict entries.
            let mut conflict_paths: HashSet<Vec<u8>> = HashSet::new();
            let mut unconflicted_paths: HashSet<Vec<u8>> = HashSet::new();
            for entry in index.entries() {
                let path = entry.path(index);
                let path_bytes: Vec<u8> = (&**path).to_vec();
                if entry.stage() == gix_index::entry::Stage::Unconflicted {
                    unconflicted_paths.insert(path_bytes);
                } else {
                    conflict_paths.insert(path_bytes);
                }
            }
            // For paths that have conflict entries but no unconflicted entry,
            // check if the file exists on disk and capture it.
            for path in &conflict_paths {
                if unconflicted_paths.contains(path) {
                    continue;
                }
                let file_path = workdir.join(gix_path::from_bstr(<&[u8] as Into<&crate::bstr::BStr>>::into(
                    path.as_slice(),
                )));
                let content = match std::fs::read(&file_path) {
                    Ok(c) => c,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(SaveError::ReadWorktreeFile(e)),
                };
                let blob_id = self.write_blob(&content)?;
                worktree_overrides.insert(path.clone(), blob_id.detach());
            }
        }

        if worktree_overrides.is_empty() {
            return Ok(index_tree_id);
        }

        // Build a modified index with the worktree blobs, then write it as a tree.
        let mut worktree_index = self.index_from_tree(&index_tree_id)?;
        for (entry, entry_path) in worktree_index.entries_mut_with_paths() {
            let path_bytes: &[u8] = &**entry_path;
            if let Some(&new_oid) = worktree_overrides.get(path_bytes) {
                entry.id = new_oid;
            }
        }

        let outcome = worktree_index.write_tree_to(|tree| self.write_object(tree).map(|id| id.detach()))?;

        Ok(outcome.tree_id)
    }
}

/// Parse the `new_oid` field from a reflog line.
///
/// A reflog line has the format:
/// `<old-oid> <new-oid> <author> <timestamp> <timezone>\t<message>\n`
fn parse_reflog_new_oid(line: &[u8], hash_kind: gix_hash::Kind) -> Option<ObjectId> {
    let hex_len = hash_kind.len_in_hex();
    let start = hex_len + 1;
    let end = start + hex_len;
    if line.len() < end {
        return None;
    }
    let hex = &line[start..end];
    ObjectId::from_hex(hex).ok()
}
