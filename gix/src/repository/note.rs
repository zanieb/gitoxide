use crate::{Result, note};
use gix_error::{OptionExt, ResultExt};
use gix_hash::ObjectId;

impl crate::Repository {
    /// Return a platform for repeated Git notes queries and mutations.
    ///
    /// Selected notes references do not have to exist. Missing references are treated
    /// as containing no notes rather than causing queries to fail.
    pub fn notes(&self) -> Result<note::Platform<'_>> {
        note::Platform::new(self)
    }
}

/// A note attached to a git object, with its content already loaded.
#[derive(Debug, Clone)]
pub struct Note {
    /// The object this note annotates.
    pub target: ObjectId,
    /// The OID of the blob containing the note text.
    pub note_id: ObjectId,
    /// The note message content.
    pub message: Vec<u8>,
}

impl crate::Repository {
    fn selected_notes_ref(&self, reference: Option<&str>) -> Result<gix_ref::FullName> {
        match reference {
            Some(reference) => gix_ref::FullName::try_from(reference).or_error(),
            None => self
                .notes()?
                .default_ref()
                .map(ToOwned::to_owned)
                .ok_or_raise(|| gix_error::validation("No default notes reference is configured")),
        }
    }

    /// List the mappings in the selected notes reference.
    pub fn notes_list(&self, reference: Option<&str>) -> Result<Vec<gix_note::Entry>> {
        let reference = self.selected_notes_ref(reference)?;
        let tree = self.find_reference(reference.as_ref())?.peel_to_commit()?.tree()?;
        gix_note::list_notes(&tree.decode()?.into(), self.object_hash(), "", &mut |id| {
            self.find_tree(*id)?.decode().map(gix_object::Tree::from)
        })
        .or_error()
    }

    /// Return the blob ID of the note attached to `target` in the selected reference.
    pub fn note_find(&self, target: &ObjectId, reference: Option<&str>) -> Result<Option<ObjectId>> {
        let name = self.selected_notes_ref(reference)?;
        let tree = self.find_reference(name.as_ref())?.peel_to_commit()?.tree()?;
        gix_note::State::new(tree.id, self)?.get(target, self)
    }

    /// Read the note attached to `target` in the selected reference.
    pub fn note_read(&self, target: &ObjectId, reference: Option<&str>) -> Result<Option<Note>> {
        self.note_find(target, reference)?
            .map(|note_id| {
                let blob = self.find_blob(note_id)?;
                Ok(Note {
                    target: *target,
                    note_id,
                    message: blob.data.clone(),
                })
            })
            .transpose()
    }

    /// Add a note or replace it if `force` is set. An empty message removes the note.
    /// Return the new notes commit if the mapping changed.
    pub fn note_add(
        &self,
        target: impl Into<ObjectId>,
        message: &[u8],
        reference: Option<&str>,
        force: bool,
    ) -> Result<Option<ObjectId>> {
        let target = target.into();
        self.find_object(target)?;
        let name = self.selected_notes_ref(reference)?;
        let mut platform = self.notes()?.with_refs([name.as_bstr().to_owned()])?;
        let exists = !platform.get(target)?.is_empty();
        if exists && !force {
            gix_error::bail!(gix_error::message!("A note already exists for {target}").conflict());
        }
        if message.is_empty() {
            if !exists {
                return Ok(None);
            }
            platform
                .with_commit_message("Notes removed by 'git notes add'")
                .remove(name.as_bstr().to_owned(), target)?;
        } else {
            let log = if exists {
                "Notes overwritten by 'git notes add'"
            } else {
                "Notes added by 'git notes add'"
            };
            platform
                .with_commit_message(log)
                .replace_at_ref(name.as_ref(), target, message)?;
        }
        Ok(Some(self.find_reference(name.as_ref())?.follow_to_object()?.detach()))
    }

    /// Remove the note attached to `target`, returning an error if there was no note.
    pub fn note_remove(&self, target: &ObjectId, reference: Option<&str>) -> Result<()> {
        let name = self.selected_notes_ref(reference)?;
        self.notes()?
            .with_commit_message("Notes removed by 'git notes remove'")
            .remove(name.as_bstr().to_owned(), *target)?
            .ok_or_raise(|| gix_error::message!("No note exists for {target}").not_found())?;
        Ok(())
    }
}
