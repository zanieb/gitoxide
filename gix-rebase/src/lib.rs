//! Implement git rebase operations.
//!
//! This crate provides types for reading and writing rebase state, as well as
//! driving a merge-based rebase operation.
//!
//! ## Rebase state
//!
//! Git stores rebase state in `.git/rebase-merge/` (for merge-based rebase,
//! which is the default) or `.git/rebase-apply/` (for apply-based rebase using
//! `git format-patch` + `git am`).
//!
//! The [`MergeState`] type can read and write the merge-based rebase state directory.
//!
//! ## Driving a rebase
//!
//! The [`MergeState::step()`] method processes the next operation in the todo list.
//! It takes a [`Driver`] implementation that provides the actual commit/merge operations.
//! This decouples the rebase logic from the specific repository implementation.
#![deny(rust_2018_idioms)]
#![deny(missing_docs)]
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use bstr::{BString, ByteSlice};
use gix_hash::ObjectId;

/// The state of a merge-based rebase operation (`.git/rebase-merge/`).
///
/// This is used by `git rebase` (the default merge strategy) and `git rebase --interactive`.
#[derive(Debug, Clone)]
pub struct MergeState {
    /// The name of the branch being rebased, e.g. `refs/heads/feature`.
    /// If rebasing a detached HEAD, this is `detached HEAD`.
    pub head_name: BString,
    /// The commit that is being rebased onto.
    pub onto: ObjectId,
    /// The original HEAD commit before the rebase started.
    pub orig_head: ObjectId,
    /// Whether this is an interactive rebase.
    pub interactive: bool,
    /// The remaining todo list of operations.
    pub todo: gix_sequencer::todo::TodoList,
    /// The operations that have already been completed.
    pub done: gix_sequencer::todo::TodoList,
    /// The current step number (1-based).
    pub current_step: usize,
    /// The total number of steps.
    pub total_steps: usize,
    /// If the rebase is stopped (e.g. for `edit` or conflict), this holds the commit hash.
    pub stopped_sha: Option<ObjectId>,
    /// Accumulated message for chained squash/fixup operations.
    ///
    /// When multiple squash or fixup operations are chained (e.g., `pick A`, `squash B`,
    /// `squash C`), the combined message is stored here so that each subsequent squash
    /// sees the accumulated result rather than reading the original commit message.
    /// Cleared when a non-squash/fixup operation runs.
    pub accumulated_squash_message: Option<Vec<u8>>,
}

/// Errors that can occur when reading rebase state.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ReadStateError {
    #[error("could not read file '{path}'")]
    ReadFile { path: PathBuf, source: std::io::Error },
    #[error("could not parse object id from '{path}': '{content}'")]
    ParseObjectId {
        path: PathBuf,
        content: String,
        source: gix_hash::decode::Error,
    },
    #[error("could not parse step number from '{path}': '{content}'")]
    ParseNumber { path: PathBuf, content: String },
    #[error("could not parse todo list")]
    ParseTodo(#[from] gix_sequencer::todo::parse::Error),
}

/// Errors that can occur when writing rebase state.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum WriteStateError {
    #[error("could not create rebase state directory '{path}'")]
    CreateDir { path: PathBuf, source: std::io::Error },
    #[error("could not write file '{path}'")]
    WriteFile { path: PathBuf, source: std::io::Error },
}

/// Errors that can occur when stepping through a rebase.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum StepError {
    #[error("no more operations in the todo list")]
    TodoEmpty,
    #[error("could not resolve abbreviated commit prefix to a full object id")]
    ResolvePrefix(#[from] Box<dyn std::error::Error + Send + Sync>),
    #[error("cherry-pick failed")]
    CherryPick(#[from] CherryPickError),
    #[error("could not write rebase state")]
    WriteState(#[from] WriteStateError),
    #[error("the rebase is not currently stopped")]
    NotStopped,
    #[error("could not read commit message")]
    ReadCommitMessage(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("could not update HEAD")]
    UpdateHead(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// An error originating from the [`Driver::cherry_pick()`] callback.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum CherryPickError {
    #[error("merge conflict while applying {commit_id}")]
    Conflict { commit_id: ObjectId },
    #[error("driver error: {message}")]
    Other {
        message: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// The outcome of a single rebase step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// The operation was applied successfully and produced a new commit.
    Applied {
        /// The id of the newly created commit.
        new_commit: ObjectId,
    },
    /// The operation was a `drop` or `noop`, no commit was created.
    Skipped,
    /// The rebase is paused (e.g., `edit`, `reword`, or `break`). The caller should
    /// let the user make changes, then call [`MergeState::continue_rebase()`].
    Paused {
        /// The commit associated with the pause (if any, e.g., from `edit`).
        commit_id: Option<ObjectId>,
        /// For `reword` operations, this contains the original commit message
        /// that the caller should present for editing. After editing, the caller
        /// should amend HEAD with the new message before calling `continue_rebase()`.
        original_message: Option<Vec<u8>>,
    },
    /// The todo list is empty; the rebase is complete.
    Done,
}

/// The outcome of a cherry-pick performed by the [`Driver`].
#[derive(Debug, Clone)]
pub struct CherryPickOutcome {
    /// The id of the newly created commit.
    pub new_commit_id: ObjectId,
}

/// A trait providing the repository operations needed to drive a rebase.
///
/// This decouples the rebase state machine from the specific repository implementation,
/// allowing `gix-rebase` to remain a low-level crate that doesn't depend on `gix`.
///
/// Implementors should resolve abbreviated hashes, perform 3-way merges (cherry-pick),
/// and create new commits as needed.
pub trait Driver {
    /// Resolve an abbreviated commit [`gix_hash::Prefix`] to a full [`ObjectId`].
    fn resolve_commit(&self, prefix: &gix_hash::Prefix) -> Result<ObjectId, Box<dyn std::error::Error + Send + Sync>>;

    /// Cherry-pick `commit_id` onto the current HEAD.
    ///
    /// The implementation should:
    /// 1. Perform a 3-way merge: ancestor = parent of `commit_id`, ours = HEAD, theirs = `commit_id`.
    /// 2. If the merge is clean, create a commit with the given `message` (which may differ
    ///    from the original commit message, e.g. for squash/fixup).
    /// 3. Update HEAD to the new commit.
    /// 4. Update the index and worktree.
    ///
    /// Returns the id of the newly created commit.
    fn cherry_pick(&self, commit_id: ObjectId, message: Option<&[u8]>) -> Result<CherryPickOutcome, CherryPickError>;

    /// Read the raw message of a commit.
    fn read_commit_message(&self, commit_id: ObjectId) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>;

    /// Update HEAD (and the branch it points to, if any) to the given commit.
    fn update_head(&self, commit_id: ObjectId) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

impl MergeState {
    /// Read the rebase merge state from the given `rebase_merge_dir` (typically `.git/rebase-merge/`).
    pub fn read_from(rebase_merge_dir: &Path, hash_kind: gix_hash::Kind) -> Result<Self, ReadStateError> {
        let head_name = read_file_trimmed(rebase_merge_dir, "head-name")?;
        let onto = read_object_id(rebase_merge_dir, "onto", hash_kind)?;
        let orig_head = read_object_id(rebase_merge_dir, "orig-head", hash_kind)?;
        let interactive = rebase_merge_dir.join("interactive").exists();

        let todo_content = read_file_bytes(rebase_merge_dir, "git-rebase-todo")?;
        let todo = gix_sequencer::todo::TodoList::parse(todo_content.as_bstr(), hash_kind)?;

        let done = if rebase_merge_dir.join("done").exists() {
            let done_content = read_file_bytes(rebase_merge_dir, "done")?;
            gix_sequencer::todo::TodoList::parse(done_content.as_bstr(), hash_kind)?
        } else {
            gix_sequencer::todo::TodoList {
                operations: std::collections::VecDeque::new(),
            }
        };

        let current_step = read_number(rebase_merge_dir, "msgnum").unwrap_or(1);
        let total_steps = read_number(rebase_merge_dir, "end").unwrap_or(todo.operations.len() + done.operations.len());

        let stopped_sha = if rebase_merge_dir.join("stopped-sha").exists() {
            Some(read_object_id(rebase_merge_dir, "stopped-sha", hash_kind)?)
        } else {
            None
        };

        let accumulated_squash_message = if rebase_merge_dir.join("message-squash").exists() {
            Some(read_file_bytes(rebase_merge_dir, "message-squash")?)
        } else {
            None
        };

        Ok(MergeState {
            head_name: BString::from(head_name),
            onto,
            orig_head,
            interactive,
            todo,
            done,
            current_step,
            total_steps,
            stopped_sha,
            accumulated_squash_message,
        })
    }

    /// Write this rebase state into the given `rebase_merge_dir`.
    ///
    /// The directory is created if it does not exist.
    pub fn write_to(&self, rebase_merge_dir: &Path) -> Result<(), WriteStateError> {
        std::fs::create_dir_all(rebase_merge_dir).map_err(|source| WriteStateError::CreateDir {
            path: rebase_merge_dir.to_owned(),
            source,
        })?;

        write_file(rebase_merge_dir, "head-name", self.head_name.as_bytes())?;
        write_file(rebase_merge_dir, "onto", self.onto.to_hex().to_string().as_bytes())?;
        write_file(
            rebase_merge_dir,
            "orig-head",
            self.orig_head.to_hex().to_string().as_bytes(),
        )?;

        if self.interactive {
            write_file(rebase_merge_dir, "interactive", b"")?;
        }

        let mut todo_buf = Vec::new();
        self.todo
            .write_to(&mut todo_buf)
            .map_err(|source| WriteStateError::WriteFile {
                path: rebase_merge_dir.join("git-rebase-todo"),
                source,
            })?;
        write_file(rebase_merge_dir, "git-rebase-todo", &todo_buf)?;

        let mut done_buf = Vec::new();
        self.done
            .write_to(&mut done_buf)
            .map_err(|source| WriteStateError::WriteFile {
                path: rebase_merge_dir.join("done"),
                source,
            })?;
        write_file(rebase_merge_dir, "done", &done_buf)?;

        write_file(rebase_merge_dir, "msgnum", self.current_step.to_string().as_bytes())?;
        write_file(rebase_merge_dir, "end", self.total_steps.to_string().as_bytes())?;

        if let Some(ref stopped) = self.stopped_sha {
            write_file(rebase_merge_dir, "stopped-sha", stopped.to_hex().to_string().as_bytes())?;
        } else {
            let stopped_path = rebase_merge_dir.join("stopped-sha");
            if stopped_path.exists() {
                let _ = std::fs::remove_file(&stopped_path);
            }
        }

        if let Some(ref msg) = self.accumulated_squash_message {
            write_file(rebase_merge_dir, "message-squash", msg)?;
        } else {
            let msg_path = rebase_merge_dir.join("message-squash");
            if msg_path.exists() {
                let _ = std::fs::remove_file(&msg_path);
            }
        }

        Ok(())
    }

    /// Remove the rebase state directory, aborting the rebase.
    pub fn remove(rebase_merge_dir: &Path) -> Result<(), std::io::Error> {
        if rebase_merge_dir.exists() {
            std::fs::remove_dir_all(rebase_merge_dir)?;
        }
        Ok(())
    }

    /// Execute the next operation in the todo list.
    ///
    /// This pops the first operation from `todo`, executes it via the provided [`Driver`],
    /// and moves it to `done`. The rebase state is written to `rebase_merge_dir` after
    /// each step.
    ///
    /// Returns a [`StepOutcome`] indicating what happened:
    /// - `Applied` if a commit was created (pick, reword, squash, fixup).
    /// - `Skipped` if the operation was a drop or noop.
    /// - `Paused` if the rebase should pause (edit, break).
    /// - `Done` if the todo list was already empty.
    ///
    /// # Errors
    ///
    /// Returns [`StepError::ResolvePrefix`] if an abbreviated commit hash cannot be resolved.
    /// Returns [`StepError::CherryPick`] if the cherry-pick (3-way merge) fails, including
    /// on merge conflicts ([`CherryPickError::Conflict`]).
    /// Returns [`StepError::WriteState`] if the rebase state cannot be written to disk.
    /// Returns [`StepError::ReadCommitMessage`] if a commit message cannot be read
    /// (needed for squash/fixup operations).
    /// Also returns `StepError::ResolvePrefix` for unsupported operations (exec, label,
    /// reset, update-ref, revert, merge) rather than silently skipping them.
    pub fn step(&mut self, driver: &dyn Driver, rebase_merge_dir: &Path) -> Result<StepOutcome, StepError> {
        if self.todo.operations.is_empty() {
            return Ok(StepOutcome::Done);
        }

        let op = self.todo.operations.pop_front().expect("todo list is not empty");
        // Push to done BEFORE executing, so on failure the operation is not lost
        // from both lists. This matches C Git behavior where the failed operation
        // appears in the `done` file.
        let op_for_exec = op.clone();
        self.done.operations.push_back(op);
        self.current_step += 1;

        let outcome = match self.execute_operation(&op_for_exec, driver) {
            Ok(outcome) => outcome,
            Err(e) => {
                // Write state so the failed operation is persisted in `done`.
                self.stopped_sha = None;
                let _ = self.write_to(rebase_merge_dir);
                return Err(e);
            }
        };

        match &outcome {
            StepOutcome::Paused { commit_id, .. } => {
                self.stopped_sha = *commit_id;
            }
            _ => {
                self.stopped_sha = None;
            }
        }

        self.write_to(rebase_merge_dir)?;
        Ok(outcome)
    }

    /// Resume the rebase after it was paused (by `edit` or a conflict).
    ///
    /// The current HEAD is assumed to contain the user's changes. This
    /// simply continues by executing the next step. If the todo list is
    /// empty, the rebase is complete.
    pub fn continue_rebase(&mut self, driver: &dyn Driver, rebase_merge_dir: &Path) -> Result<StepOutcome, StepError> {
        if self.stopped_sha.is_none() && !self.todo.operations.is_empty() {
            // Not stopped, but there are operations -- just step.
        } else if self.stopped_sha.is_none() && self.todo.operations.is_empty() {
            return Ok(StepOutcome::Done);
        }

        self.stopped_sha = None;
        // Remove the stopped-sha file if it exists.
        let stopped_path = rebase_merge_dir.join("stopped-sha");
        if stopped_path.exists() {
            let _ = std::fs::remove_file(&stopped_path);
        }

        if self.todo.operations.is_empty() {
            return Ok(StepOutcome::Done);
        }

        self.step(driver, rebase_merge_dir)
    }

    /// Abort the rebase, restoring HEAD to the original commit.
    ///
    /// This resets HEAD to `orig_head` and removes the rebase state directory.
    pub fn abort(&self, driver: &dyn Driver, rebase_merge_dir: &Path) -> Result<(), StepError> {
        driver.update_head(self.orig_head).map_err(StepError::UpdateHead)?;
        Self::remove(rebase_merge_dir).map_err(|source| {
            StepError::WriteState(WriteStateError::WriteFile {
                path: rebase_merge_dir.to_owned(),
                source,
            })
        })?;
        Ok(())
    }

    /// Execute a single sequencer operation.
    fn execute_operation(
        &mut self,
        op: &gix_sequencer::todo::Operation,
        driver: &dyn Driver,
    ) -> Result<StepOutcome, StepError> {
        use gix_sequencer::todo::Operation;

        match op {
            Operation::Pick { commit, .. } => {
                let commit_id = driver.resolve_commit(commit)?;
                let result = driver.cherry_pick(commit_id, None)?;
                // Clear accumulated squash message -- a non-squash/fixup operation
                // starts a new sequence.
                self.accumulated_squash_message = None;
                Ok(StepOutcome::Applied {
                    new_commit: result.new_commit_id,
                })
            }
            Operation::Reword { commit, .. } => {
                let commit_id = driver.resolve_commit(commit)?;
                // Read the original message BEFORE cherry-picking so we can provide
                // it to the caller for editing.
                let original_message = driver
                    .read_commit_message(commit_id)
                    .map_err(StepError::ReadCommitMessage)?;
                // Cherry-pick the commit (creates the commit with the original message).
                // The caller is expected to amend HEAD with the edited message after
                // receiving this Paused outcome.
                let result = driver.cherry_pick(commit_id, None)?;
                self.accumulated_squash_message = None;
                Ok(StepOutcome::Paused {
                    commit_id: Some(result.new_commit_id),
                    original_message: Some(original_message),
                })
            }
            Operation::Edit { commit, .. } => {
                let commit_id = driver.resolve_commit(commit)?;
                let result = driver.cherry_pick(commit_id, None)?;
                self.accumulated_squash_message = None;
                Ok(StepOutcome::Paused {
                    commit_id: Some(result.new_commit_id),
                    original_message: None,
                })
            }
            Operation::Squash { commit, .. } => {
                let commit_id = driver.resolve_commit(commit)?;
                // For squash, we combine messages. Use the accumulated squash message
                // if available (from a previous squash in a chain), otherwise fall
                // back to reading the last done commit's original message.
                let prev_msg = if let Some(ref accumulated) = self.accumulated_squash_message {
                    Some(accumulated.clone())
                } else {
                    self.last_done_commit_message(driver)?
                };
                let squash_msg = driver
                    .read_commit_message(commit_id)
                    .map_err(StepError::ReadCommitMessage)?;
                let mut combined = prev_msg.unwrap_or_default();
                if !combined.is_empty() && !combined.ends_with(b"\n") {
                    combined.push(b'\n');
                }
                combined.extend_from_slice(b"\n");
                combined.extend_from_slice(&squash_msg);
                let result = driver.cherry_pick(commit_id, Some(&combined))?;
                // Store the accumulated message for the next squash/fixup in the chain.
                self.accumulated_squash_message = Some(combined);
                Ok(StepOutcome::Applied {
                    new_commit: result.new_commit_id,
                })
            }
            Operation::Fixup { commit, .. } => {
                // Fixup is like squash but discards the fixup commit's message.
                // The resulting commit keeps the previous commit's (or accumulated) message.
                let commit_id = driver.resolve_commit(commit)?;
                let prev_msg = if let Some(ref accumulated) = self.accumulated_squash_message {
                    Some(accumulated.clone())
                } else {
                    self.last_done_commit_message(driver)?
                };
                let result = driver.cherry_pick(commit_id, prev_msg.as_deref())?;
                // Preserve the accumulated message for the next fixup/squash.
                if self.accumulated_squash_message.is_none() {
                    if let Some(msg) = prev_msg {
                        self.accumulated_squash_message = Some(msg);
                    }
                }
                Ok(StepOutcome::Applied {
                    new_commit: result.new_commit_id,
                })
            }
            Operation::Drop { .. } => {
                self.accumulated_squash_message = None;
                Ok(StepOutcome::Skipped)
            }
            Operation::Noop => Ok(StepOutcome::Skipped),
            Operation::Break => Ok(StepOutcome::Paused {
                commit_id: None,
                original_message: None,
            }),
            Operation::Exec { command, .. } => Err(StepError::ResolvePrefix(
                format!("exec operation not yet supported: {}", String::from_utf8_lossy(command)).into(),
            )),
            Operation::Label { .. }
            | Operation::Reset { .. }
            | Operation::UpdateRef { .. }
            | Operation::Revert { .. }
            | Operation::Merge { .. } => {
                // These operations are not yet supported by the rebase driver.
                // Return an error rather than silently skipping, as skipping could
                // corrupt repository state (e.g., missed Reset or Label operations).
                Err(StepError::ResolvePrefix(
                    format!("unsupported rebase operation: {:?}", op).into(),
                ))
            }
        }
    }

    /// Get the commit message of the last completed operation (for squash/fixup).
    ///
    /// Skips the last entry in `done` because it is the current operation being
    /// executed (pushed to `done` before execution in `step()`).
    fn last_done_commit_message(&self, driver: &dyn Driver) -> Result<Option<Vec<u8>>, StepError> {
        use gix_sequencer::todo::Operation;
        // Skip the last entry (the current operation, already pushed to done).
        for op in self.done.operations.iter().rev().skip(1) {
            match op {
                Operation::Pick { commit, .. }
                | Operation::Reword { commit, .. }
                | Operation::Edit { commit, .. }
                | Operation::Squash { commit, .. }
                | Operation::Fixup { commit, .. } => {
                    let commit_id = driver.resolve_commit(commit)?;
                    let msg = driver
                        .read_commit_message(commit_id)
                        .map_err(StepError::ReadCommitMessage)?;
                    return Ok(Some(msg));
                }
                _ => continue,
            }
        }
        Ok(None)
    }
}

fn read_file_trimmed(dir: &Path, name: &str) -> Result<Vec<u8>, ReadStateError> {
    let path = dir.join(name);
    let content = std::fs::read(&path).map_err(|source| ReadStateError::ReadFile {
        path: path.clone(),
        source,
    })?;
    Ok(content.trim_ascii().to_vec())
}

fn read_file_bytes(dir: &Path, name: &str) -> Result<Vec<u8>, ReadStateError> {
    let path = dir.join(name);
    std::fs::read(&path).map_err(|source| ReadStateError::ReadFile { path, source })
}

fn read_object_id(dir: &Path, name: &str, _hash_kind: gix_hash::Kind) -> Result<ObjectId, ReadStateError> {
    let content = read_file_trimmed(dir, name)?;
    let path = dir.join(name);
    let hex = content.to_str_lossy();
    let hex_trimmed = hex.trim();
    // C Git always writes full hex hashes in rebase state files.
    // Reject abbreviated hashes rather than silently zero-padding them,
    // which would produce an incorrect ObjectId.
    ObjectId::from_hex(hex_trimmed.as_bytes()).map_err(|source| ReadStateError::ParseObjectId {
        path,
        content: hex_trimmed.to_string(),
        source,
    })
}

fn read_number(dir: &Path, name: &str) -> Result<usize, ReadStateError> {
    let content = read_file_trimmed(dir, name)?;
    let path = dir.join(name);
    let s = String::from_utf8_lossy(&content);
    s.trim().parse::<usize>().map_err(|_| ReadStateError::ParseNumber {
        path,
        content: s.into_owned(),
    })
}

fn write_file(dir: &Path, name: &str, content: &[u8]) -> Result<(), WriteStateError> {
    let path = dir.join(name);
    std::fs::write(&path, content).map_err(|source| WriteStateError::WriteFile { path, source })
}

#[cfg(test)]
mod tests {
    use gix_hash::Kind;

    use super::*;

    #[test]
    fn roundtrip_state() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let todo = gix_sequencer::todo::TodoList {
            operations: [
                gix_sequencer::todo::Operation::Pick {
                    commit: ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                        .unwrap()
                        .into(),
                    summary: "First commit".into(),
                },
                gix_sequencer::todo::Operation::Pick {
                    commit: ObjectId::from_hex(b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
                        .unwrap()
                        .into(),
                    summary: "Second commit".into(),
                },
            ]
            .into(),
        };

        let state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: ObjectId::from_hex(b"cccccccccccccccccccccccccccccccccccccccc").unwrap(),
            orig_head: ObjectId::from_hex(b"dddddddddddddddddddddddddddddddddddddddd").unwrap(),
            interactive: true,
            todo,
            done: gix_sequencer::todo::TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 1,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        let read_back = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();

        assert_eq!(read_back.head_name, state.head_name);
        assert_eq!(read_back.onto, state.onto);
        assert_eq!(read_back.orig_head, state.orig_head);
        assert_eq!(read_back.interactive, state.interactive);
        assert_eq!(read_back.todo.operations.len(), 2);
        assert_eq!(read_back.done.operations.len(), 0);
        assert_eq!(read_back.current_step, 1);
        assert_eq!(read_back.total_steps, 2);
        assert!(read_back.stopped_sha.is_none());

        MergeState::remove(&rebase_dir).unwrap();
        assert!(!rebase_dir.exists());
    }

    #[test]
    fn abbreviated_stopped_sha_is_rejected() {
        // C Git always writes full 40-char hex hashes in state files like `stopped-sha`.
        // If we encounter an abbreviated hash, we should error rather than silently
        // zero-pad it into an incorrect ObjectId.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();

        // Write minimal valid state files
        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/feature").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"cccccccccccccccccccccccccccccccccccccccc").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"dddddddddddddddddddddddddddddddddddddddd",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();
        std::fs::write(rebase_dir.join("msgnum"), b"1").unwrap();
        std::fs::write(rebase_dir.join("end"), b"1").unwrap();

        // Write an abbreviated hash in stopped-sha (only 7 chars)
        std::fs::write(rebase_dir.join("stopped-sha"), b"abcdef1").unwrap();

        let result = MergeState::read_from(&rebase_dir, Kind::Sha1);
        // This MUST error -- zero-padding would produce the wrong ObjectId
        assert!(
            result.is_err(),
            "abbreviated hash in stopped-sha should produce an error, not be zero-padded"
        );
    }

    #[test]
    fn full_hash_in_state_files_works() {
        // Full 40-char hashes should be read correctly
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();

        let expected_oid = ObjectId::from_hex(b"abcdef1234567890abcdef1234567890abcdef12").unwrap();
        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/feature").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"cccccccccccccccccccccccccccccccccccccccc").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"dddddddddddddddddddddddddddddddddddddddd",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();
        std::fs::write(rebase_dir.join("msgnum"), b"1").unwrap();
        std::fs::write(rebase_dir.join("end"), b"1").unwrap();
        std::fs::write(
            rebase_dir.join("stopped-sha"),
            b"abcdef1234567890abcdef1234567890abcdef12",
        )
        .unwrap();

        let state = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(state.stopped_sha, Some(expected_oid));
    }
}
