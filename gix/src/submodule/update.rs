//! Submodule update operations, modelled after `git submodule update`.
//!
//! The implementation follows the same logic as git's `builtin/submodule--helper.c`:
//!
//! 1. Determine the target commit from the superproject index
//! 2. Determine the update strategy (checkout/rebase/merge/none/command)
//! 3. If the submodule repo doesn't exist, clone it
//! 4. If the target commit isn't reachable, fetch
//! 5. Apply the update strategy (for checkout: detach HEAD to target and checkout tree)
//! 6. Optionally recurse into nested submodules

/// Options for controlling a submodule update operation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Options {
    /// If `true`, initialize the submodule (write url and update strategy to `.git/config`)
    /// before updating. Equivalent to `git submodule update --init`.
    pub init: bool,
    /// If `true`, recursively update nested submodules after updating this one.
    /// Equivalent to `git submodule update --recursive`.
    pub recursive: bool,
}

impl Options {
    /// Create new options with the given settings.
    pub fn new(init: bool, recursive: bool) -> Self {
        Options { init, recursive }
    }
}

impl Default for Options {
    fn default() -> Self {
        Options {
            init: false,
            recursive: false,
        }
    }
}

/// The outcome of a single submodule update.
#[derive(Debug)]
pub struct Outcome {
    /// The strategy that was used to update the submodule.
    pub strategy: gix_submodule::config::Update,
    /// The commit id that the submodule was updated to.
    pub target_commit: gix_hash::ObjectId,
    /// Whether the submodule was freshly cloned (as opposed to fetched/already present).
    pub freshly_cloned: bool,
    /// The checkout outcome, if the submodule worktree was checked out.
    #[cfg(feature = "worktree-mutation")]
    pub checkout: Option<gix_worktree_state::checkout::Outcome>,
}

/// Re-export the error type when the feature is available.
#[cfg(all(feature = "blocking-network-client", feature = "worktree-mutation"))]
pub use super::errors::update::Error;
