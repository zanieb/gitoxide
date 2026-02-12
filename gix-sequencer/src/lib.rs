//! Parse and write git sequencer todo lists.
//!
//! The sequencer is used by `git cherry-pick`, `git revert`, and `git rebase --interactive`
//! to track which operations are pending. The todo list is stored in files like
//! `.git/sequencer/todo` or `.git/rebase-merge/git-rebase-todo`.
#![deny(rust_2018_idioms)]
#![deny(missing_docs)]
#![forbid(unsafe_code)]

/// A single operation in a sequencer todo list.
pub mod todo;
