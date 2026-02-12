mod util;
use util::*;

mod clone;
mod clone_refs;
mod commit;
mod config;
#[cfg(feature = "blob-diff")]
mod diff;
mod error;
mod head;
mod id;
mod init;
#[cfg(feature = "note")]
mod note;
mod object;
mod reference;
mod remote;
mod repository;
#[cfg(feature = "worktree-mutation")]
mod reset;
#[cfg(feature = "revision")]
mod revision;
mod signing;
#[cfg(feature = "worktree-mutation")]
mod stash;
#[cfg(feature = "status")]
mod status;
#[cfg(feature = "attributes")]
mod submodule;
