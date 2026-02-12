//! Shared helpers for operations that update the index, working tree, and HEAD.
//!
//! Several higher-level commands (reset, stash, cherry-pick, revert) need to:
//! - Check out an index to the working tree
//! - Remove files that are no longer tracked
//! - Update HEAD (and the branch it points to)
//!
//! This module consolidates those patterns into reusable helpers.

/// Errors returned by the shared worktree helpers.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum CheckoutError {
    #[error(transparent)]
    CheckoutOptions(#[from] crate::config::checkout_options::Error),
    #[error(transparent)]
    Checkout(#[from] gix_worktree_state::checkout::Error),
    #[error("Failed to convert object database to Arc for thread-safe access")]
    ObjectsToArc(#[from] std::io::Error),
}

/// Errors returned by [`update_head_ref`](crate::Repository::update_head_ref).
#[cfg(feature = "merge")]
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum UpdateHeadError {
    #[error(transparent)]
    FindHead(#[from] crate::reference::find::existing::Error),
    #[error(transparent)]
    EditReference(#[from] crate::reference::edit::Error),
}

impl crate::Repository {
    /// Check out all entries in the given index to the working tree directory.
    ///
    /// This overwrites existing files and is used after resetting the index to
    /// materialize the new file contents on disk.
    pub(crate) fn checkout_index_to_worktree_impl(
        &self,
        index: &mut gix_index::File,
        workdir: &std::path::Path,
    ) -> Result<(), CheckoutError> {
        use std::sync::atomic::AtomicBool;

        let mut opts = self
            .checkout_options(gix_worktree::stack::state::attributes::Source::IdMapping)
            .map_err(CheckoutError::CheckoutOptions)?;
        opts.overwrite_existing = true;
        opts.destination_is_initially_empty = false;

        let objects = self.objects.clone().into_arc().map_err(CheckoutError::ObjectsToArc)?;
        let should_interrupt = AtomicBool::new(false);
        let files = gix_features::progress::Discard;
        let bytes = gix_features::progress::Discard;

        gix_worktree_state::checkout(index, workdir, objects, &files, &bytes, &should_interrupt, opts)
            .map_err(CheckoutError::Checkout)?;
        Ok(())
    }

    /// Remove working tree files that are in `old_index` but not in `new_index`.
    ///
    /// This handles the case where a reset/checkout moves to a tree that no longer
    /// contains files present in the current tree -- those files need to be deleted.
    ///
    /// When `check_path_traversal` is `true`, paths containing `..` components are
    /// skipped to prevent deleting files outside the worktree (security mitigation
    /// for crafted tree entries, similar to CVE-2018-11235).
    ///
    /// Errors from removing individual files are intentionally ignored, matching
    /// git's behavior.
    pub(crate) fn remove_worktree_files_not_in_index(
        old_index: &gix_index::File,
        new_index: &gix_index::File,
        workdir: &std::path::Path,
        check_path_traversal: bool,
    ) {
        use std::collections::HashSet;

        let new_paths: HashSet<&[u8]> = new_index
            .entries()
            .iter()
            .filter(|e| e.stage() == gix_index::entry::Stage::Unconflicted)
            .map(|e| &**e.path(new_index))
            .collect();

        for entry in old_index.entries() {
            if entry.stage() != gix_index::entry::Stage::Unconflicted {
                continue;
            }
            let path = entry.path(old_index);
            let path_bytes: &[u8] = &**path;
            if new_paths.contains(path_bytes) {
                continue;
            }

            let os_path = gix_path::from_bstr(path);
            if check_path_traversal
                && os_path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                continue;
            }

            let file_path = workdir.join(os_path);
            let _ = std::fs::remove_file(&file_path);
            // Try to remove empty parent directories.
            if let Some(parent) = file_path.parent() {
                let _ = remove_empty_parents(parent, workdir);
            }
        }
    }

    /// Update HEAD (and the branch it points to) to point at `new_id`.
    ///
    /// The reflog message is formatted as `"{operation}: moving to {new_id}"`.
    /// This does **not** set ORIG_HEAD -- callers that need it (e.g. `reset`) should
    /// do so separately.
    #[cfg(feature = "merge")]
    pub(crate) fn update_head_ref(&self, new_id: gix_hash::ObjectId, operation: &str) -> Result<(), UpdateHeadError> {
        use gix_ref::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};

        let head_ref = self.find_reference("HEAD")?;
        let ref_name = match &head_ref.inner.target {
            gix_ref::Target::Symbolic(name) => name.clone(),
            gix_ref::Target::Object(_) => "HEAD".try_into().expect("valid ref name"),
        };

        let reflog_message = format!("{operation}: moving to {new_id}");

        self.edit_reference(RefEdit {
            change: Change::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    force_create_reflog: false,
                    message: reflog_message.into(),
                },
                expected: PreviousValue::Any,
                new: gix_ref::Target::Object(new_id),
            },
            name: ref_name,
            deref: false,
        })?;

        Ok(())
    }
}

/// Remove empty parent directories up to (but not including) `stop_at`.
pub(super) fn remove_empty_parents(dir: &std::path::Path, stop_at: &std::path::Path) -> std::io::Result<()> {
    let mut current = dir;
    while current != stop_at && current.starts_with(stop_at) {
        match std::fs::remove_dir(current) {
            Ok(()) => {}
            Err(_) => break,
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => break,
        }
    }
    Ok(())
}
