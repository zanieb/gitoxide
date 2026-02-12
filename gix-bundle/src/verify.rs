//! Verify that a repository satisfies a bundle's prerequisites.

use gix_hash::ObjectId;

use crate::Header;

/// Errors that can occur when verifying a bundle's prerequisites.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("missing prerequisite object {id}")]
    MissingPrerequisite {
        /// The object id that was required but not found.
        id: ObjectId,
    },
}

/// The outcome of verifying a bundle.
#[derive(Debug)]
pub struct Outcome {
    /// The number of prerequisites that were checked.
    pub num_prerequisites: usize,
}

impl Header {
    /// Verify that all prerequisites exist by calling `contains` for each prerequisite object id.
    ///
    /// `contains` should return `true` if the object exists in the target repository.
    ///
    /// A bundle with no prerequisites always verifies successfully.
    pub fn verify_prerequisites(&self, contains: impl Fn(&ObjectId) -> bool) -> Result<Outcome, Error> {
        for prereq in &self.prerequisites {
            if !contains(&prereq.id) {
                return Err(Error::MissingPrerequisite { id: prereq.id });
            }
        }
        Ok(Outcome {
            num_prerequisites: self.prerequisites.len(),
        })
    }
}
