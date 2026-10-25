///
pub mod negotiate {
    #[cfg(feature = "credentials")]
    pub use gix_negotiate::Algorithm;
}

#[cfg(any(feature = "blocking-network-client", feature = "async-network-client"))]
pub use super::connection::fetch::{Outcome, Prepare, ProgressId, RefLogMessage, Status, outcome, refs};

/// If `Yes`, don't really make changes but do as much as possible to get an idea of what would be done.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg(any(feature = "blocking-network-client", feature = "async-network-client"))]
pub(crate) enum DryRun {
    /// Enable dry-run mode and don't actually change the underlying repository in any way.
    Yes,
    /// Run the operation like normal, making changes to the underlying repository.
    No,
}

/// How to deal with refs when cloning or fetching.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg(any(feature = "blocking-network-client", feature = "async-network-client"))]
pub(crate) enum WritePackedRefs {
    /// Normal operation, i.e. don't use packed-refs at all for writing.
    Never,
    /// Put ref updates straight into the `packed-refs` file, without creating loose refs first or dealing with them in any way.
    Only,
}

#[cfg(any(feature = "blocking-network-client", feature = "async-network-client"))]
pub use gix_protocol::fetch::{RefMap, refmap};
pub use gix_protocol::fetch::{Shallow, Tags};

/// Limit the objects sent by a remote that supports partial clones.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Filter {
    /// Omit blob contents.
    BlobNone,
    /// Omit trees and blobs at or below this depth. Zero requests only commits and tags.
    TreeDepth(u32),
}

impl std::fmt::Display for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlobNone => f.write_str("blob:none"),
            Self::TreeDepth(depth) => write!(f, "tree:{depth}"),
        }
    }
}
