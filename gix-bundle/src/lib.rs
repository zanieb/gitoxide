//! Read, write, and verify [git bundle](https://git-scm.com/docs/bundle-format) files.
//!
//! A git bundle is a portable file format that packages a set of git objects (as a packfile)
//! together with a header listing references and optional prerequisites. This allows
//! transporting repository data without a live connection to a remote.
//!
//! ## Format
//!
//! The bundle format (v2 and v3) consists of:
//! 1. A signature line (`# v2 git bundle` or `# v3 git bundle`)
//! 2. Optional capabilities (v3 only)
//! 3. Zero or more prerequisite lines (`-<oid> <comment>`)
//! 4. One or more reference lines (`<oid> <refname>`)
//! 5. A blank line separator
//! 6. A packfile containing all necessary objects
#![deny(rust_2018_idioms, missing_docs)]
#![forbid(unsafe_code)]

use bstr::BString;
use gix_hash::ObjectId;

/// The version of the bundle format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// The v2 bundle format. No capabilities.
    V2,
    /// The v3 bundle format. Supports capabilities like `filter`.
    V3,
}

/// A prerequisite that the target repository must satisfy before the bundle can be applied.
///
/// The target must already contain the object referenced by `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prerequisite {
    /// The object id that must exist in the target repository.
    pub id: ObjectId,
    /// An optional human-readable comment (typically a one-line commit message).
    pub comment: Option<BString>,
}

/// A reference contained in the bundle, mapping a name to an object id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    /// The object id this reference points to.
    pub id: ObjectId,
    /// The full reference name, e.g. `refs/heads/main`.
    pub name: BString,
}

/// The parsed header of a git bundle file, containing all metadata except the packfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The bundle format version.
    pub version: Version,
    /// Prerequisites that must exist in the target repository.
    pub prerequisites: Vec<Prerequisite>,
    /// References included in this bundle.
    pub refs: Vec<Ref>,
    /// Capabilities (v3 only). Each entry is a `key=value` or just `key`.
    pub capabilities: Vec<BString>,
}

///
pub mod header;

///
pub mod create;

///
pub mod verify;
