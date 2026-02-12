use super::response;
use bstr::BString;

/// A command to update a reference on the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The name of the reference to update, e.g. `refs/heads/main`.
    pub ref_name: BString,
    /// The old object id the server is expected to have. Use a null id for creating new refs.
    pub old_id: gix_hash::ObjectId,
    /// The new object id to set. Use a null id for deleting refs.
    pub new_id: gix_hash::ObjectId,
}

impl Command {
    /// Create a new command to update `ref_name` from `old_id` to `new_id`.
    pub fn new(ref_name: impl Into<BString>, old_id: gix_hash::ObjectId, new_id: gix_hash::ObjectId) -> Self {
        Command {
            ref_name: ref_name.into(),
            old_id,
            new_id,
        }
    }

    /// Returns `true` if this is a delete command (new_id is null).
    pub fn is_delete(&self) -> bool {
        self.new_id.is_null()
    }

    /// Returns `true` if this is a create command (old_id is null).
    pub fn is_create(&self) -> bool {
        self.old_id.is_null()
    }
}

/// Options for the push operation.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// If `true`, the server should only check if the push would succeed without actually updating refs.
    /// This requires the server to support the `atomic` or `report-status` capability.
    pub dry_run: bool,
    /// If `true`, request atomic push so that all ref updates succeed or all fail together.
    /// Only available if the server advertises `atomic` capability.
    pub atomic: bool,
}

/// The outcome of a push operation.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The per-reference status of the push.
    pub ref_updates: Vec<response::StatusV1>,
    /// The status of the unpack operation on the server side.
    pub unpack_status: response::UnpackStatus,
}

/// The progress ids used during the push operation.
#[derive(Debug, Copy, Clone)]
pub enum ProgressId {
    /// The progress name is defined by the remote and the progress messages it sets.
    RemoteProgress,
    /// Progress of sending the packfile data.
    SendingPack,
    /// Progress of counting objects to push.
    CountingObjects,
}

impl From<ProgressId> for gix_features::progress::Id {
    fn from(v: ProgressId) -> Self {
        match v {
            ProgressId::RemoteProgress => *b"PURP",
            ProgressId::SendingPack => *b"PUSP",
            ProgressId::CountingObjects => *b"PUCO",
        }
    }
}
