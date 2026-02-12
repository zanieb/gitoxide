use std::collections::VecDeque;

use bstr::BString;
use gix_hash::Prefix;

/// An operation in a sequencer todo list.
///
/// This corresponds to a single line in a `git-rebase-todo` or `sequencer/todo` file.
///
/// Commit references are stored as [`Prefix`] values, which correctly handle both full
/// and abbreviated hashes. Callers must resolve abbreviated prefixes via the ODB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// `pick <commit>` -- apply the given commit.
    Pick {
        /// The commit to cherry-pick (may be abbreviated).
        commit: Prefix,
        /// The commit summary (subject line), used for display only.
        summary: BString,
    },
    /// `reword <commit>` -- like pick, but pause to edit the commit message.
    Reword {
        /// The commit to apply (may be abbreviated).
        commit: Prefix,
        /// The commit summary.
        summary: BString,
    },
    /// `edit <commit>` -- like pick, but pause after applying.
    Edit {
        /// The commit to apply (may be abbreviated).
        commit: Prefix,
        /// The commit summary.
        summary: BString,
    },
    /// `squash <commit>` -- like pick, but merge into the previous commit and edit the message.
    Squash {
        /// The commit to squash (may be abbreviated).
        commit: Prefix,
        /// The commit summary.
        summary: BString,
    },
    /// `fixup <commit>` -- like squash, but discard this commit's message.
    Fixup {
        /// The commit to fixup (may be abbreviated).
        commit: Prefix,
        /// The commit summary.
        summary: BString,
        /// If `true`, open the editor to amend the previous message (fixup -C / fixup -c).
        amend_message: AmendMessage,
    },
    /// `exec <command>` -- run a shell command.
    Exec {
        /// The command to execute.
        command: BString,
    },
    /// `break` -- pause the rebase at this point.
    Break,
    /// `drop <commit>` -- remove this commit (same as deleting the line).
    Drop {
        /// The commit to drop (may be abbreviated).
        commit: Prefix,
        /// The commit summary.
        summary: BString,
    },
    /// `label <name>` -- label the current HEAD for later reference.
    Label {
        /// The label name.
        name: BString,
    },
    /// `reset <name>` -- reset HEAD to the named label.
    Reset {
        /// The label name (or commit) to reset to.
        name: BString,
    },
    /// `update-ref <ref>` -- track a ref to be updated at the end of the rebase.
    UpdateRef {
        /// The ref to update.
        reference: BString,
    },
    /// `noop` -- do nothing, used as a placeholder when the todo list would otherwise be empty.
    Noop,
    /// `revert <commit>` -- revert the given commit (used by `git revert` sequencer).
    Revert {
        /// The commit to revert (may be abbreviated).
        commit: Prefix,
        /// The commit summary.
        summary: BString,
    },
    /// `merge [-C|-c <commit>] <label> [# <oneline>]` -- create a merge commit.
    ///
    /// Used by `git rebase --rebase-merges` to recreate merge commits.
    Merge {
        /// If set, the commit whose message to use (`-C` to keep, `-c` to edit).
        commit: Option<(Prefix, AmendMessage)>,
        /// The label to merge (previously created with `label`).
        label: BString,
        /// An optional one-line description following `#`.
        oneline: BString,
    },
}

/// Whether and how the commit message should be amended during a fixup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmendMessage {
    /// Don't amend; discard this commit's message (plain `fixup`).
    No,
    /// Replace the previous message with this commit's message (`fixup -C`).
    Replace,
    /// Open the editor to combine the messages (`fixup -c`).
    Edit,
}

/// A parsed todo list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoList {
    /// The operations to perform, in order.
    ///
    /// Stored as a [`VecDeque`] so that removing the front element (the next
    /// operation to execute) is O(1) instead of O(n).
    pub operations: VecDeque<Operation>,
}

///
pub mod parse;

///
pub mod write;
