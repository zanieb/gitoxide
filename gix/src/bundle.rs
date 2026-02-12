//! Create, verify, and unbundle git bundle files.
//!
//! Git bundles are portable files that contain refs and a packfile. They can be
//! used to transfer repository data without a network connection.

pub use gix_bundle::{create, header, verify, Header, Prerequisite, Ref, Version};

/// The error returned by bundle operations on a [`Repository`](crate::Repository).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error(transparent)]
    Header(#[from] gix_bundle::header::Error),
    #[error(transparent)]
    Verify(#[from] gix_bundle::verify::Error),
    #[error(transparent)]
    Create(#[from] gix_bundle::create::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    FindObject(#[from] crate::object::find::existing::Error),
    #[error(transparent)]
    FindReference(#[from] crate::reference::find::Error),
}

impl crate::Repository {
    /// Parse and verify a bundle file, checking that all prerequisites exist in this repository.
    ///
    /// Returns the bundle header on success.
    pub fn bundle_verify(&self, path: impl AsRef<std::path::Path>) -> Result<Header, Error> {
        let (header, _reader) = header::from_path(path, self.object_hash())?;
        header.verify_prerequisites(|id| self.has_object(id))?;
        Ok(header)
    }

    /// Read the header of a bundle file without verifying prerequisites.
    pub fn bundle_header(&self, path: impl AsRef<std::path::Path>) -> Result<Header, gix_bundle::header::Error> {
        let (header, _reader) = header::from_path(path, self.object_hash())?;
        Ok(header)
    }
}
