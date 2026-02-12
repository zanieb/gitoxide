use crate::bstr::BString;

/// The error returned by [`PreparePush::send()`][super::PreparePush::send()].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error(transparent)]
    Protocol(#[from] gix_protocol::push::Error),
    #[error("No mapping of local references to remote references was produced, so nothing could be pushed")]
    NoMapping {
        /// The refspecs that were used for the push, for the user to see.
        refspecs: Vec<gix_refspec::RefSpec>,
        /// The number of remote refs that were advertised.
        num_remote_refs: usize,
    },
    #[error("The local object hash {local} does not match the remote hash {remote}")]
    IncompatibleObjectHash {
        local: gix_hash::Kind,
        remote: gix_hash::Kind,
    },
    #[error("Could not determine local reference for push mapping: {name}")]
    FindLocalRef {
        name: BString,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Could not find object {oid} in local repository")]
    FindObject {
        oid: gix_hash::ObjectId,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Could not generate pack data for push")]
    PackGeneration(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl gix_protocol::transport::IsSpuriousError for Error {
    fn is_spurious(&self) -> bool {
        match self {
            Error::Protocol(err) => err.is_spurious(),
            _ => false,
        }
    }
}
