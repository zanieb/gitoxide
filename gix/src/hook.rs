//! Discover and execute Git hooks.
//!
//! Git hooks are scripts located in the hooks directory (`.git/hooks/` by default, or the path
//! specified by `core.hooksPath`) that are executed at specific points in Git's workflow.
//!
//! A hook must be executable and returns exit code 0 on success. A non-zero exit code
//! typically causes the operation to abort.
#![allow(clippy::empty_docs)]

use std::path::PathBuf;

/// The name of a well-known Git hook.
///
/// This identifies the hook to run and determines which file to look for in the hooks directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Name {
    /// Called before a commit is created. No arguments.
    /// A non-zero exit code aborts the commit.
    PreCommit,
    /// Called to prepare the default commit message, before the editor is launched.
    /// Arguments: message file path, message source (optional), commit SHA (optional).
    PrepareCommitMsg,
    /// Called after the commit message is entered, with the message file path as argument.
    /// Can be used to validate or modify the message. A non-zero exit code aborts the commit.
    CommitMsg,
    /// Called after a commit is successfully created. No arguments. Informational only.
    PostCommit,
    /// Called before a push, with the remote name and URL as arguments.
    /// Receives lines of `<local ref> <local sha> <remote ref> <remote sha>` on stdin.
    /// A non-zero exit code aborts the push.
    PrePush,
    /// Called on the server before updating refs. Receives lines of
    /// `<old-value> <new-value> <ref-name>` on stdin.
    PreReceive,
    /// Called on the server after all refs are updated. Receives the same stdin as `pre-receive`.
    PostReceive,
    /// Called on the server for each ref being updated.
    /// Arguments: ref name, old SHA, new SHA.
    Update,
    /// Called before applying a patch (from `git am`). No arguments.
    PreApplyPatch,
    /// Called after applying a patch (from `git am`). No arguments.
    PostApplyPatch,
    /// Called after a successful merge. No arguments. Informational only.
    PostMerge,
    /// Called before a rebase. Arguments: upstream, rebased branch.
    PreRebase,
    /// Called after a checkout or switch. Arguments: previous HEAD ref, new HEAD ref, branch flag.
    PostCheckout,
    /// Called during `git rewrite` operations like `git rebase` or `git filter-branch`.
    PostRewrite,
    /// Called before a merge commit is created (similar to `pre-commit` but for merges).
    PreMergeCommit,
    /// Called by `git push` on the remote after the default `update` hook.
    PostUpdate,
    /// Called to validate the ref transaction.
    ReferenceTransaction,
}

impl Name {
    /// Return the file name of this hook as it appears in the hooks directory.
    pub fn as_str(&self) -> &'static str {
        match self {
            Name::PreCommit => "pre-commit",
            Name::PrepareCommitMsg => "prepare-commit-msg",
            Name::CommitMsg => "commit-msg",
            Name::PostCommit => "post-commit",
            Name::PrePush => "pre-push",
            Name::PreReceive => "pre-receive",
            Name::PostReceive => "post-receive",
            Name::Update => "update",
            Name::PreApplyPatch => "pre-applypatch",
            Name::PostApplyPatch => "post-applypatch",
            Name::PostMerge => "post-merge",
            Name::PreRebase => "pre-rebase",
            Name::PostCheckout => "post-checkout",
            Name::PostRewrite => "post-rewrite",
            Name::PreMergeCommit => "pre-merge-commit",
            Name::PostUpdate => "post-update",
            Name::ReferenceTransaction => "reference-transaction",
        }
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The outcome of running a hook.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The exit status of the hook process.
    pub status: std::process::ExitStatus,
    /// The captured stdout output, if stdout was piped.
    pub stdout: Option<Vec<u8>>,
    /// The captured stderr output, if stderr was piped.
    pub stderr: Option<Vec<u8>>,
}

impl Outcome {
    /// Returns `true` if the hook exited successfully (exit code 0).
    pub fn success(&self) -> bool {
        self.status.success()
    }
}

/// A prepared hook that has been discovered and is ready to be executed.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The path to the hook script.
    pub path: PathBuf,
    /// The name of the hook.
    pub name: Name,
}

///
pub mod run {
    use std::path::PathBuf;

    /// The error returned by [`Repository::run_hook()`](crate::Repository::run_hook()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error("Failed to spawn hook at '{path}'")]
        Spawn { path: PathBuf, source: std::io::Error },
        #[error("Failed to write to hook stdin")]
        WriteStdin(#[source] std::io::Error),
        #[error("Failed to wait for hook to complete")]
        Wait(#[source] std::io::Error),
        #[error("Failed to read hook stdout")]
        ReadStdout(#[source] std::io::Error),
        #[error("Failed to read hook stderr")]
        ReadStderr(#[source] std::io::Error),
        #[error(transparent)]
        CommandContext(#[from] crate::config::command_context::Error),
        #[error("Could not interpolate the hooks path from configuration")]
        HooksPathInterpolation(#[source] gix_config::path::interpolate::Error),
        #[error("The hook at '{path}' exited with code {code}")]
        HookFailed {
            /// The path of the hook that failed.
            path: PathBuf,
            /// The exit code, or -1 if terminated by signal.
            code: i32,
        },
    }
}

///
pub mod find {
    /// The error returned by [`Repository::find_hook()`](crate::Repository::find_hook()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error("Could not interpolate the hooks path from configuration")]
        InterpolatePath(#[from] gix_config::path::interpolate::Error),
    }
}
