use crate::{Result, note};

impl crate::Repository {
    /// Return a platform for repeated Git notes queries and mutations.
    ///
    /// Selected notes references do not have to exist. Missing references are treated
    /// as containing no notes rather than causing queries to fail.
    pub fn notes(&self) -> Result<note::Platform<'_>> {
        note::Platform::new(self)
    }
}

fn count_first_nibble(counts: &mut [u8; 16], target: &ObjectId) {
    let bucket = usize::from(target.as_bytes()[0] >> 4);
    counts[bucket] = counts[bucket].saturating_add(1).min(2);
}

fn is_flat_note_entry(entry: &gix_object::tree::Entry, hash_kind: gix_hash::Kind) -> bool {
    entry.mode.is_blob() && gix_note::target_from_path(entry.filename.as_ref(), hash_kind).is_ok()
}
