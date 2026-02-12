#![allow(clippy::result_large_err, clippy::unnecessary_debug_formatting)]
mod util;
use util::*;

#[cfg(all(feature = "merge", feature = "worktree-mutation"))]
mod cherry_pick;
mod clone;
mod commit;
mod config;
#[cfg(feature = "blob-diff")]
mod diff;
mod head;
mod id;
mod init;
#[cfg(feature = "note")]
mod note;
mod object;
#[cfg(feature = "blocking-network-client")]
mod push;
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
#[cfg(feature = "attributes")]
mod submodule_update;
