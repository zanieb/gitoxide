//! Create git bundle files.
//!
//! A bundle is created by selecting references to include, optionally specifying
//! prerequisite commits, and writing the header followed by a packfile containing
//! all necessary objects.

use bstr::BString;
use gix_hash::ObjectId;

use crate::{Header, Prerequisite, Ref, Version};

/// Options for creating a bundle.
#[derive(Debug, Clone)]
pub struct Options {
    /// The bundle format version to produce.
    pub version: Version,
    /// The object hash algorithm in use.
    pub object_hash: gix_hash::Kind,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            version: Version::V2,
            object_hash: gix_hash::Kind::Sha1,
        }
    }
}

/// Errors that can occur when creating a bundle.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("no references specified for the bundle")]
    NoRefs,
    #[error(transparent)]
    Header(#[from] std::io::Error),
    #[error("failed to generate pack data")]
    PackGeneration(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// A builder for creating a git bundle file.
///
/// # Usage
///
/// ```no_run
/// use gix_bundle::{create::Builder, Version};
/// use gix_hash::ObjectId;
///
/// let mut builder = Builder::new(Version::V2, gix_hash::Kind::Sha1);
/// // Add references that this bundle provides.
/// builder.add_ref("refs/heads/main", ObjectId::null(gix_hash::Kind::Sha1));
/// ```
#[derive(Debug)]
pub struct Builder {
    header: Header,
    /// Tip object ids that the packfile must contain (i.e., the ref targets).
    tips: Vec<ObjectId>,
    /// Objects that the recipient is expected to already have (prerequisites).
    exclude: Vec<ObjectId>,
}

impl Builder {
    /// Create a new bundle builder for the given version and hash kind.
    pub fn new(version: Version, _object_hash: gix_hash::Kind) -> Self {
        Builder {
            header: Header {
                version,
                prerequisites: Vec::new(),
                refs: Vec::new(),
                capabilities: Vec::new(),
            },
            tips: Vec::new(),
            exclude: Vec::new(),
        }
    }

    /// Add a reference to the bundle.
    pub fn add_ref(&mut self, name: impl Into<BString>, id: ObjectId) -> &mut Self {
        self.tips.push(id);
        self.header.refs.push(Ref { id, name: name.into() });
        self
    }

    /// Add a prerequisite: an object that the target repository must already have.
    pub fn add_prerequisite(&mut self, id: ObjectId, comment: Option<BString>) -> &mut Self {
        self.exclude.push(id);
        self.header.prerequisites.push(Prerequisite { id, comment });
        self
    }

    /// Add a v3 capability.
    pub fn add_capability(&mut self, capability: impl Into<BString>) -> &mut Self {
        self.header.capabilities.push(capability.into());
        self
    }

    /// Return the object ids that should be tips of the packfile (ref targets).
    pub fn tips(&self) -> &[ObjectId] {
        &self.tips
    }

    /// Return the object ids that should be excluded from the packfile (prerequisites).
    pub fn exclude(&self) -> &[ObjectId] {
        &self.exclude
    }

    /// Write the bundle to the given writer.
    ///
    /// The `write_pack` closure receives the writer (positioned after the header),
    /// the tip object ids, and the exclusion set, and must write a complete packfile.
    /// It returns `Ok(true)` if a packfile was written, or `Ok(false)` if the pack is empty.
    pub fn write_to<E>(
        self,
        mut writer: impl std::io::Write,
        write_pack: impl FnOnce(&mut dyn std::io::Write, &[ObjectId], &[ObjectId]) -> Result<bool, E>,
    ) -> Result<(), Error>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        if self.header.refs.is_empty() {
            return Err(Error::NoRefs);
        }

        self.header.write_to(&mut writer).map_err(Error::Header)?;

        write_pack(&mut writer, &self.tips, &self.exclude).map_err(|e| Error::PackGeneration(Box::new(e)))?;

        Ok(())
    }
}
