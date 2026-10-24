//!
#![allow(clippy::empty_docs)]

/// The kind of Git repository, focussing on the repository data itself, i.e. what's in `.git`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Kind {
    /// An ordinary Git repository.
    Common,
    /// A submodule worktree, whose `git` repository lives in `.git/modules/**/<name>` of the parent repository.
    ///
    /// Note that 'old-form' submodules (with a nested `.git` directory) are represented as [`Kind::Common`].
    Submodule,
    /// A worktree, whose `git` repository lives in `.git/worktrees/**/<name>` of the parent repository.
    LinkedWorkTree,
}

/// A supported repository format version, as selected by `core.repositoryFormatVersion`.
///
/// See [Git's repository format specification](https://git-scm.com/docs/repository-version).
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum FormatVersion {
    /// The original SHA-1 repository format, used when no version is configured.
    ///
    /// Objects use SHA-1 identifiers, and references use loose files and `packed-refs`.
    /// Git honors these historical `extensions.*` keys even in version 0:
    ///
    /// - `noop`: changes no behavior; used for compatibility testing.
    /// - `preciousObjects`: forbids deleting objects, including during pruning and repacking.
    /// - `partialClone`: names the promisor remote that can supply intentionally omitted objects.
    /// - `worktreeConfig`: enables per-worktree `config.worktree` files in addition to the shared configuration.
    ///
    /// Unknown extension keys are ignored, but known version-1-only extensions such as `objectFormat` and
    /// `relativeWorktrees` are rejected. Enabling those requires [`V1`][Self::V1].
    #[default]
    V0,
    /// Version 0's base format with explicit extension negotiation.
    ///
    /// The version number alone does not change storage or enable features. Git requires readers to understand
    /// every configured `extensions.*` key and its value before operating on a version-1 repository.
    /// Examples of version-1-only extensions include:
    ///
    /// - `objectFormat`: selects SHA-1 or SHA-256 objects; without it, SHA-1 remains the default.
    ///   `gix` needs the corresponding `sha1` or `sha256` Cargo feature.
    /// - `relativeWorktrees`: records that worktrees may use relative links to their administrative Git directories.
    ///   Creating relative links is controlled separately by `worktree.useRelativePaths`.
    /// - `refStorage`: selects Git's reference storage backend, including reftable.
    ///
    /// The historical version-0 extensions remain valid. Recognizing version 1 does not imply that `gix` supports
    /// every extension. With no extensions configured, prefer [`V0`][Self::V0] for compatibility with older readers.
    V1,
}

impl FormatVersion {
    /// Validate that upgrading this repository format to version 1 will not activate unsupported extensions.
    ///
    /// For version 0, permit only Git's grandfathered `noop`, `preciousObjects`, `partialClone`, and `worktreeConfig`
    /// extensions, without subsections. Unknown extensions that version 0 ignores become significant in version 1.
    /// Version 1 needs no upgrade and is accepted without checking its extensions.
    ///
    /// This only checks extension names; it neither validates values nor modifies `config`.
    pub fn validate_upgrade_to_v1(self, config: &gix_config::File) -> crate::Result<()> {
        use gix_error::bail;

        if self == Self::V1 {
            return Ok(());
        }
        for section in config.sections_by_name("extensions").into_iter().flatten() {
            for name in section.value_names() {
                if section.header().subsection_name().is_some()
                    || !["noop", "preciousobjects", "partialclone", "worktreeconfig"]
                        .iter()
                        .any(|known| name.eq_ignore_ascii_case(known))
                {
                    let mut error =
                        gix_error::unsupported("Cannot upgrade repository format with unsupported extension")
                            .with("extension", name);
                    if let Some(subsection) = section.header().subsection_name() {
                        error = error.validation().with("subsection", subsection);
                    }
                    bail!(error);
                }
            }
        }
        Ok(())
    }
}

#[cfg(any(feature = "attributes", feature = "excludes"))]
pub mod attributes;
///
#[cfg(feature = "blame")]
mod blame;
/// Local branch operations.
pub mod branch;
mod cache;
#[cfg(feature = "worktree-mutation")]
mod checkout;
mod config;

///
#[cfg(all(feature = "merge", feature = "worktree-mutation"))]
pub mod cherry_pick;
///
#[cfg(feature = "blob-diff")]
mod diff;
///
#[cfg(feature = "dirwalk")]
mod dirwalk;
///
#[cfg(feature = "attributes")]
pub mod filter;
///
pub mod freelist;
mod graph;
#[cfg(feature = "hooks")]
mod hook;
pub(crate) mod identity;
mod impls;
#[cfg(feature = "index")]
mod index;
#[cfg(feature = "dirwalk")]
mod index_add;
pub(crate) mod init;
mod location;
#[cfg(feature = "mailmap")]
mod mailmap;
///
#[cfg(feature = "merge")]
mod merge;
/// Operations on a single notes reference.
#[cfg(feature = "notes")]
pub mod note;
mod object;
#[cfg(feature = "attributes")]
mod pathspec;
mod reference;
mod remote;
mod revision;
mod shallow;
/// Support for signing commits and tags.
pub mod signing;
mod state;
#[cfg(feature = "attributes")]
mod submodule;
mod thread_safe;
pub(crate) mod worktree;

///
#[cfg(feature = "blame")]
pub mod blame_file {
    /// Options to be passed to [Repository::blame_file()](crate::Repository::blame_file()).
    #[derive(Default, Debug, Clone)]
    pub struct Options {
        /// The algorithm to use for diffing. If `None`, `diff.algorithm` will be used.
        pub diff_algorithm: Option<gix_diff::blob::Algorithm>,
        /// The ranges to blame in the file.
        pub ranges: gix_blame::BlameRanges,
        /// Don't consider commits before the given date.
        pub since: Option<gix_date::Time>,
        /// Determine if rename tracking should be performed, and how.
        pub rewrites: Option<gix_diff::Rewrites>,
        /// A set of commits to ignore when blaming.
        ///
        /// Same-sized replacements made by ignored single-parent commits are passed through to
        /// their parents instead of being attributed to them. More complex replacement hunks that
        /// need C Git's fuzzy ignore-rev matching remain attributed to the ignored commit.
        pub ignore_revs: Vec<gix_hash::ObjectId>,
        /// If set, the blame operation checks this flag periodically and aborts
        /// if it has been set to `true`. This allows the caller to implement
        /// cancellation/interrupt support.
        ///
        /// If `None`, a private flag is used that cannot be interrupted externally.
        pub should_interrupt: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        /// If set, include uncommitted worktree changes in the blame output.
        ///
        /// The provided bytes should be the raw content of the file as it exists in the worktree.
        /// Lines that differ between the worktree version and the HEAD version will be attributed
        /// to a virtual "uncommitted changes" entry using [`gix_hash::ObjectId::null()`] as the
        /// commit id.
        pub worktree_blob: Option<Vec<u8>>,
        /// If set, stop traversal when reaching this commit and mark remaining entries as boundary.
        ///
        /// This is the OID-based equivalent of [`since`](Self::since). When a parent commit
        /// matches `oldest_commit`, it is not enqueued for further traversal.
        pub oldest_commit: Option<gix_hash::ObjectId>,
    }
}

///
#[cfg(feature = "worktree-mutation")]
pub mod reset;

///
#[cfg(feature = "worktree-mutation")]
pub mod stash;

#[cfg(feature = "worktree-mutation")]
pub(crate) mod worktree_ops;

/// Add files from the worktree to the index.
#[cfg(feature = "dirwalk")]
pub mod add_to_index {
    /// Options for [`Repository::add_to_index()`](crate::Repository::add_to_index()).
    #[derive(Default, Debug, Clone, Copy)]
    pub struct Options {
        /// Stage ignored files as well.
        pub force_ignored: bool,
    }

    /// The outcome of [`Repository::add_to_index()`](crate::Repository::add_to_index()).
    #[derive(Default, Debug, Clone)]
    pub struct Outcome {
        /// The number of paths added or refreshed in the index.
        pub added_entries: usize,
        /// The number of tracked paths removed from the index because they are missing in the worktree.
        pub removed_entries: usize,
        /// Ignored paths that matched the walk. These are skipped unless [`Options::force_ignored`] is set.
        pub ignored_entries: Vec<crate::bstr::BString>,
    }

    /// The error returned by [`Repository::add_to_index()`](crate::Repository::add_to_index()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        Operation(#[from] gix_error::Error),
        #[error("A working tree is required to add files to the index")]
        MissingWorktree,
        #[error(transparent)]
        FilterPipeline(gix_error::Error),
        #[error(transparent)]
        Pathspec(gix_error::Error),
        #[error(transparent)]
        DirwalkOptions(gix_error::Error),
        #[error(transparent)]
        Dirwalk(gix_error::Error),
        #[error(transparent)]
        WorktreeFile(gix_error::Error),
        #[error("Could not read metadata for '{path}'")]
        Metadata {
            path: std::path::PathBuf,
            source: std::io::Error,
        },
        #[error("Could not convert filesystem metadata for '{path}' to index stat information")]
        Stat {
            path: std::path::PathBuf,
            source: std::time::SystemTimeError,
        },
        #[error("Pathspecs did not match any files: {pathspecs:?}")]
        PathspecsDidNotMatch { pathspecs: Vec<crate::bstr::BString> },
        #[error("The following paths are ignored by one of your exclude files: {paths:?}")]
        Ignored { paths: Vec<crate::bstr::BString> },
        #[error(transparent)]
        WriteIndex(gix_error::Error),
    }
}
