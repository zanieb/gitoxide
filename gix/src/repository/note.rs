use gix_hash::ObjectId;

/// Notes-related errors.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("Could not find the notes reference '{reference}'")]
    FindReference {
        reference: String,
        source: crate::reference::find::existing::Error,
    },
    #[error("Could not peel notes reference to a commit")]
    PeelToCommit(#[from] crate::object::peel::to_kind::Error),
    #[error("Could not decode the notes tree")]
    DecodeTree(#[from] gix_object::decode::Error),
    #[error("Could not find object")]
    FindObject(#[from] crate::object::find::existing::Error),
    #[error("Object was not the expected kind")]
    ObjectKind(#[from] crate::object::try_into::Error),
    #[error("Could not list notes from tree")]
    ListNotes(#[from] gix_note::ListError<TreeLookupError>),
    #[error("Could not find note in tree")]
    FindNote(#[from] gix_note::FindError<TreeLookupError>),
    #[error("Could not write object")]
    WriteObject(#[from] crate::object::write::Error),
    #[error("Could not update reference")]
    EditReference(#[from] crate::reference::edit::Error),
    #[error("A note already exists for object {target}. Use force to overwrite.")]
    NoteAlreadyExists { target: ObjectId },
    #[error("No note found for object {target}")]
    NoteNotFound { target: ObjectId },
    #[error("Target object {target} does not exist")]
    TargetNotFound { target: ObjectId },
    #[error("Committer identity is not configured")]
    CommitterMissing,
    #[error("Could not look up notes subtree")]
    TreeLookup(#[source] TreeLookupError),
    #[error(transparent)]
    ReferenceNameValidation(#[from] gix_ref::name::Error),
    #[error(transparent)]
    ParseTime(#[from] crate::config::time::Error),
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

/// An error type for tree-lookup callbacks used in notes traversal.
///
/// Wraps the underlying errors from object lookup, type conversion, and decoding
/// to preserve the full error chain.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum TreeLookupError {
    #[error("Could not find object in notes tree")]
    FindObject(#[from] crate::object::find::existing::Error),
    #[error("Object was not a tree")]
    NotATree(#[from] crate::object::try_into::Error),
    #[error("Could not decode tree object")]
    Decode(#[from] gix_object::decode::Error),
}

impl crate::Repository {
    /// Return the effective notes reference name.
    ///
    /// If `reference` is `Some`, it is used as-is. Otherwise, the value of
    /// `core.notesRef` from the repository config is used, falling back to
    /// `refs/notes/commits` if not configured.
    fn effective_notes_ref<'a>(&self, reference: Option<&'a str>) -> String {
        if let Some(r) = reference {
            return r.to_owned();
        }
        self.config_snapshot()
            .string("core.notesRef")
            .map(|v| v.to_string())
            .unwrap_or_else(|| gix_note::DEFAULT_REF.to_owned())
    }

    /// List all notes from the given notes `reference` (e.g. `refs/notes/commits`).
    ///
    /// If `reference` is `None`, the default notes ref is determined by the
    /// `core.notesRef` config option, falling back to `refs/notes/commits`.
    ///
    /// Returns a list of note entries (target object + note blob OID).
    pub fn notes_list(&self, reference: Option<&str>) -> Result<Vec<gix_note::Entry>, Error> {
        let notes_ref = self.effective_notes_ref(reference);
        let tree = self.notes_tree(&notes_ref)?;
        let hash_kind = self.object_hash();

        let mut find_tree =
            |oid: &ObjectId| -> Result<gix_object::Tree, TreeLookupError> { self.lookup_notes_subtree(oid) };

        Ok(gix_note::list_notes(&tree, hash_kind, "", &mut find_tree)?)
    }

    /// Find the note for the given `target` object in the notes `reference`.
    ///
    /// If `reference` is `None`, the default notes ref is determined by the
    /// `core.notesRef` config option, falling back to `refs/notes/commits`.
    ///
    /// Returns the note blob OID if found, or `None`.
    pub fn note_find(&self, target: &ObjectId, reference: Option<&str>) -> Result<Option<ObjectId>, Error> {
        let notes_ref = self.effective_notes_ref(reference);
        let tree = self.notes_tree(&notes_ref)?;
        let hash_kind = self.object_hash();

        let mut find_tree =
            |oid: &ObjectId| -> Result<gix_object::Tree, TreeLookupError> { self.lookup_notes_subtree(oid) };

        Ok(gix_note::find_note(&tree, target, hash_kind, &mut find_tree)?)
    }

    /// Find and read the note for the given `target` object.
    ///
    /// If `reference` is `None`, the default notes ref is determined by the
    /// `core.notesRef` config option, falling back to `refs/notes/commits`.
    ///
    /// Returns the full `Note` with message content if found, or `None`.
    pub fn note_read(&self, target: &ObjectId, reference: Option<&str>) -> Result<Option<Note>, Error> {
        match self.note_find(target, reference)? {
            Some(note_id) => {
                let blob = self.find_object(note_id)?;
                Ok(Some(Note {
                    target: *target,
                    note_id,
                    message: blob.data.to_vec(),
                }))
            }
            None => Ok(None),
        }
    }

    /// Add or update a note on the given `target` object.
    ///
    /// The note content is stored as a blob, and the notes tree under `reference`
    /// (default: `refs/notes/commits`) is updated. A new commit is created on
    /// the notes ref.
    ///
    /// If `force` is false and a note already exists for `target`, an error is returned.
    /// If `force` is true, the existing note is overwritten.
    ///
    /// If `message` is empty, the note is removed instead (matching git's behavior
    /// of `git notes add -f -F /dev/null`).
    ///
    /// Matches the behavior of `git notes add [-f] -m <message> <object>`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TargetNotFound`] if the target object does not exist in the repository.
    /// Returns [`Error::NoteAlreadyExists`] if a note already exists for `target` and `force` is false.
    /// Returns [`Error::CommitterMissing`] if the committer identity is not configured.
    /// Other variants cover reference lookup, object write, tree decoding,
    /// and reference editing failures.
    pub fn note_add(
        &self,
        target: impl Into<ObjectId>,
        message: &[u8],
        reference: Option<&str>,
        force: bool,
    ) -> Result<ObjectId, Error> {
        let target = target.into();
        let notes_ref = self.effective_notes_ref(reference);

        // Verify the target object exists
        self.find_object(target).map_err(|_| Error::TargetNotFound { target })?;

        // Get the existing notes tree, or start with an empty tree
        let (existing_tree, parent_commit_id) = match self.notes_tree_and_commit(&notes_ref) {
            Ok((tree, commit_id)) => (tree, Some(commit_id)),
            Err(_) => (gix_object::Tree::empty(), None),
        };

        // Use fanout-aware lookup to check for existing notes
        let hash_kind = self.object_hash();
        let mut find_tree =
            |oid: &ObjectId| -> Result<gix_object::Tree, TreeLookupError> { self.lookup_notes_subtree(oid) };
        let has_existing = gix_note::find_note(&existing_tree, &target, hash_kind, &mut find_tree)?.is_some();

        if has_existing && !force {
            return Err(Error::NoteAlreadyExists { target });
        }

        let fanout = self.detect_fanout_depth(&existing_tree);

        // Handle empty message = removal
        if message.is_empty() {
            let new_tree = self.remove_note_from_tree(&existing_tree, &target, fanout)?;
            let tree_id = self.write_object(&new_tree)?;
            return self.commit_notes_tree(
                tree_id.detach(),
                parent_commit_id,
                &notes_ref,
                "Notes removed by 'git notes add'",
            );
        }

        // Write the note blob
        let blob_id = self.write_blob(message)?;

        // Insert the note at the correct fanout level
        let new_tree = self.insert_note_into_tree(&existing_tree, &target, blob_id.detach(), fanout)?;
        let tree_id = self.write_object(&new_tree)?;

        let msg = if has_existing {
            "Notes overwritten by 'git notes add'"
        } else {
            "Notes added by 'git notes add'"
        };
        self.commit_notes_tree(tree_id.detach(), parent_commit_id, &notes_ref, msg)
    }

    /// Remove the note for the given `target` object.
    ///
    /// If `reference` is `None`, the default notes ref is determined by the
    /// `core.notesRef` config option, falling back to `refs/notes/commits`.
    ///
    /// Returns an error if no note exists for `target`.
    ///
    /// Matches the behavior of `git notes remove <object>`.
    pub fn note_remove(&self, target: &ObjectId, reference: Option<&str>) -> Result<(), Error> {
        let notes_ref = self.effective_notes_ref(reference);

        let (existing_tree, parent_commit_id) = self
            .notes_tree_and_commit(&notes_ref)
            .map(|(tree, commit_id)| (tree, Some(commit_id)))
            .map_err(|_| Error::NoteNotFound { target: *target })?;

        // Use fanout-aware lookup to check if the note exists
        let hash_kind = self.object_hash();
        let mut find_tree =
            |oid: &ObjectId| -> Result<gix_object::Tree, TreeLookupError> { self.lookup_notes_subtree(oid) };
        let had_entry = gix_note::find_note(&existing_tree, target, hash_kind, &mut find_tree)?.is_some();

        if !had_entry {
            return Err(Error::NoteNotFound { target: *target });
        }

        let fanout = self.detect_fanout_depth(&existing_tree);
        let new_tree = self.remove_note_from_tree(&existing_tree, target, fanout)?;
        let tree_id = self.write_object(&new_tree)?;
        self.commit_notes_tree(
            tree_id.detach(),
            parent_commit_id,
            &notes_ref,
            "Notes removed by 'git notes remove'",
        )?;
        Ok(())
    }

    /// Create a commit for the notes tree and update the notes ref.
    fn commit_notes_tree(
        &self,
        tree_id: ObjectId,
        parent_commit_id: Option<ObjectId>,
        notes_ref: &str,
        message: &str,
    ) -> Result<ObjectId, Error> {
        use gix_ref::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};

        let committer = self.committer().ok_or(Error::CommitterMissing)??;
        let author = self.author().ok_or(Error::CommitterMissing)??;

        let parents: smallvec::SmallVec<[ObjectId; 1]> = parent_commit_id.into_iter().collect();

        let commit = gix_object::Commit {
            message: message.into(),
            tree: tree_id,
            author: author.into(),
            committer: committer.into(),
            encoding: None,
            parents: parents.clone(),
            extra_headers: Default::default(),
        };
        let commit_id = self.write_object(&commit)?;

        let ref_name: gix_ref::FullName = notes_ref.try_into()?;

        let expected = match parents.first() {
            Some(parent) => PreviousValue::ExistingMustMatch(gix_ref::Target::Object(*parent)),
            None => PreviousValue::MustNotExist,
        };

        self.edit_reference(RefEdit {
            change: Change::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    force_create_reflog: false,
                    message: format!("notes: {message}").into(),
                },
                expected,
                new: gix_ref::Target::Object(commit_id.detach()),
            },
            name: ref_name,
            deref: false,
        })?;

        Ok(commit_id.detach())
    }

    /// Resolve a notes reference to its tree and the commit OID.
    fn notes_tree_and_commit(&self, notes_ref: &str) -> Result<(gix_object::Tree, ObjectId), Error> {
        let reference = self.find_reference(notes_ref).map_err(|e| Error::FindReference {
            reference: notes_ref.to_owned(),
            source: e,
        })?;

        let commit_id = reference.id().detach();
        let commit_obj = reference.id().object()?.peel_to_kind(gix_object::Kind::Commit)?;
        let commit = commit_obj.try_into_commit()?;
        let tree_id = commit.tree_id()?;
        let tree_obj = tree_id.object()?;
        let tree = tree_obj.try_into_tree()?;
        let decoded = tree.decode()?;
        Ok((decoded.into(), commit_id))
    }

    /// Resolve a notes reference to its tree.
    fn notes_tree(&self, notes_ref: &str) -> Result<gix_object::Tree, Error> {
        let reference = self.find_reference(notes_ref).map_err(|e| Error::FindReference {
            reference: notes_ref.to_owned(),
            source: e,
        })?;

        let commit_obj = reference.id().object()?.peel_to_kind(gix_object::Kind::Commit)?;
        let commit = commit_obj.try_into_commit()?;
        let tree_id = commit.tree_id()?;
        let tree_obj = tree_id.object()?;
        let tree = tree_obj.try_into_tree()?;
        let decoded = tree.decode()?;
        Ok(decoded.into())
    }

    /// Look up a sub-tree by OID for notes tree traversal.
    fn lookup_notes_subtree(&self, oid: &ObjectId) -> Result<gix_object::Tree, TreeLookupError> {
        let obj = self.find_object(*oid)?;
        let tree = obj.try_into_tree()?;
        let decoded = tree.decode()?;
        Ok(decoded.into())
    }

    /// Detect the fanout depth of a notes tree.
    ///
    /// Returns 0 for flat trees, 1 for 2/38 fanout, 2 for 2/2/36, etc.
    /// Detection works by examining whether root entries are 2-char hex directory names.
    fn detect_fanout_depth(&self, tree: &gix_object::Tree) -> u8 {
        fn is_two_char_hex(name: &[u8]) -> bool {
            name.len() == 2 && name[0].is_ascii_hexdigit() && name[1].is_ascii_hexdigit()
        }

        // If the tree is empty, there is no fanout.
        if tree.entries.is_empty() {
            return 0;
        }

        // Check root entries: if we find tree entries with 2-char hex names, it is fanout.
        let has_fanout_dirs = tree
            .entries
            .iter()
            .any(|e| e.mode.is_tree() && is_two_char_hex(&e.filename));

        if !has_fanout_dirs {
            return 0;
        }

        // Depth >= 1. Check one level deeper to see if there is deeper fanout.
        let mut depth: u8 = 1;
        if let Some(dir_entry) = tree
            .entries
            .iter()
            .find(|e| e.mode.is_tree() && is_two_char_hex(&e.filename))
        {
            if let Ok(sub_tree) = self.lookup_notes_subtree(&dir_entry.oid) {
                let sub_has_fanout = sub_tree
                    .entries
                    .iter()
                    .any(|e| e.mode.is_tree() && is_two_char_hex(&e.filename));
                if sub_has_fanout {
                    depth = 2;
                }
            }
        }

        depth
    }

    /// Insert a note blob into the notes tree at the correct fanout level.
    ///
    /// For `fanout == 0`, inserts a blob entry with the full hex name at the root.
    /// For `fanout == 1`, creates or updates a subtree `ab/` containing `cdef0123...`.
    fn insert_note_into_tree(
        &self,
        tree: &gix_object::Tree,
        target: &ObjectId,
        blob_id: ObjectId,
        fanout: u8,
    ) -> Result<gix_object::Tree, Error> {
        let target_hex = target.to_hex().to_string();

        if fanout == 0 {
            // Flat: insert/replace blob at root level
            let mut entries: Vec<_> = tree
                .entries
                .iter()
                .filter(|e| e.filename.as_ref() as &[u8] != target_hex.as_bytes())
                .cloned()
                .collect();
            entries.push(gix_object::tree::Entry {
                mode: gix_object::tree::EntryKind::Blob.into(),
                filename: target_hex.into(),
                oid: blob_id,
            });
            entries.sort();
            return Ok(gix_object::Tree { entries });
        }

        // Fanout: split the hex into directory prefix and remaining filename.
        let dir_name = &target_hex[..2];
        let remaining = &target_hex[2..];

        // Find or create the subtree for this fanout prefix.
        let existing_sub = tree
            .entries
            .iter()
            .find(|e| e.mode.is_tree() && e.filename.as_ref() as &[u8] == dir_name.as_bytes());

        let mut sub_tree = if let Some(sub_entry) = existing_sub {
            self.lookup_notes_subtree(&sub_entry.oid).map_err(Error::TreeLookup)?
        } else {
            gix_object::Tree::empty()
        };

        if fanout >= 2 {
            // Recurse one level deeper
            sub_tree = self.insert_note_into_subtree(&sub_tree, remaining, blob_id, fanout - 1)?;
        } else {
            // fanout == 1: insert the blob directly into this subtree
            let new_entry = gix_object::tree::Entry {
                mode: gix_object::tree::EntryKind::Blob.into(),
                filename: remaining.into(),
                oid: blob_id,
            };
            let mut sub_entries: Vec<_> = sub_tree
                .entries
                .into_iter()
                .filter(|e| e.filename.as_ref() as &[u8] != remaining.as_bytes())
                .collect();
            sub_entries.push(new_entry);
            sub_entries.sort();
            sub_tree = gix_object::Tree { entries: sub_entries };
        }

        // Write the subtree and update the root tree.
        let sub_tree_id = self.write_object(&sub_tree)?;
        let dir_entry = gix_object::tree::Entry {
            mode: gix_object::tree::EntryKind::Tree.into(),
            filename: dir_name.into(),
            oid: sub_tree_id.detach(),
        };
        let mut root_entries: Vec<_> = tree
            .entries
            .iter()
            .filter(|e| !(e.mode.is_tree() && e.filename.as_ref() as &[u8] == dir_name.as_bytes()))
            .cloned()
            .collect();
        root_entries.push(dir_entry);
        root_entries.sort();
        Ok(gix_object::Tree { entries: root_entries })
    }

    /// Recursively insert a note into a subtree for fanout depth > 1.
    fn insert_note_into_subtree(
        &self,
        tree: &gix_object::Tree,
        remaining_hex: &str,
        blob_id: ObjectId,
        fanout: u8,
    ) -> Result<gix_object::Tree, Error> {
        if fanout == 0 {
            // Base case: insert the blob here.
            let new_entry = gix_object::tree::Entry {
                mode: gix_object::tree::EntryKind::Blob.into(),
                filename: remaining_hex.into(),
                oid: blob_id,
            };
            let mut entries: Vec<_> = tree
                .entries
                .iter()
                .filter(|e| e.filename.as_ref() as &[u8] != remaining_hex.as_bytes())
                .cloned()
                .collect();
            entries.push(new_entry);
            entries.sort();
            return Ok(gix_object::Tree { entries });
        }

        let dir_name = &remaining_hex[..2];
        let rest = &remaining_hex[2..];

        let existing_sub = tree
            .entries
            .iter()
            .find(|e| e.mode.is_tree() && e.filename.as_ref() as &[u8] == dir_name.as_bytes());

        let sub_tree = if let Some(sub_entry) = existing_sub {
            self.lookup_notes_subtree(&sub_entry.oid).map_err(Error::TreeLookup)?
        } else {
            gix_object::Tree::empty()
        };

        let new_sub = self.insert_note_into_subtree(&sub_tree, rest, blob_id, fanout - 1)?;
        let sub_tree_id = self.write_object(&new_sub)?;

        let dir_entry = gix_object::tree::Entry {
            mode: gix_object::tree::EntryKind::Tree.into(),
            filename: dir_name.into(),
            oid: sub_tree_id.detach(),
        };
        let mut entries: Vec<_> = tree
            .entries
            .iter()
            .filter(|e| !(e.mode.is_tree() && e.filename.as_ref() as &[u8] == dir_name.as_bytes()))
            .cloned()
            .collect();
        entries.push(dir_entry);
        entries.sort();
        Ok(gix_object::Tree { entries })
    }

    /// Remove a note from the notes tree, handling fanout structure.
    fn remove_note_from_tree(
        &self,
        tree: &gix_object::Tree,
        target: &ObjectId,
        fanout: u8,
    ) -> Result<gix_object::Tree, Error> {
        let target_hex = target.to_hex().to_string();

        if fanout == 0 {
            // Flat: remove the blob entry from the root.
            let entries: Vec<_> = tree
                .entries
                .iter()
                .filter(|e| e.filename.as_ref() as &[u8] != target_hex.as_bytes())
                .cloned()
                .collect();
            return Ok(gix_object::Tree { entries });
        }

        // Fanout: navigate into the correct subtree.
        let dir_name = &target_hex[..2];
        let remaining = &target_hex[2..];

        let sub_entry = tree
            .entries
            .iter()
            .find(|e| e.mode.is_tree() && e.filename.as_ref() as &[u8] == dir_name.as_bytes());

        let sub_entry = match sub_entry {
            Some(e) => e,
            None => return Ok(tree.clone()),
        };

        let sub_tree = self.lookup_notes_subtree(&sub_entry.oid).map_err(Error::TreeLookup)?;

        let new_sub = if fanout >= 2 {
            self.remove_note_from_subtree(&sub_tree, remaining, fanout - 1)?
        } else {
            // fanout == 1: remove the blob from this subtree
            let entries: Vec<_> = sub_tree
                .entries
                .into_iter()
                .filter(|e| e.filename.as_ref() as &[u8] != remaining.as_bytes())
                .collect();
            gix_object::Tree { entries }
        };

        // Build updated root, removing the directory entirely if it became empty.
        let mut root_entries: Vec<_> = tree
            .entries
            .iter()
            .filter(|e| !(e.mode.is_tree() && e.filename.as_ref() as &[u8] == dir_name.as_bytes()))
            .cloned()
            .collect();

        if !new_sub.entries.is_empty() {
            let sub_tree_id = self.write_object(&new_sub)?;
            root_entries.push(gix_object::tree::Entry {
                mode: gix_object::tree::EntryKind::Tree.into(),
                filename: dir_name.into(),
                oid: sub_tree_id.detach(),
            });
            root_entries.sort();
        }

        Ok(gix_object::Tree { entries: root_entries })
    }

    /// Recursively remove a note from a subtree for fanout depth > 1.
    fn remove_note_from_subtree(
        &self,
        tree: &gix_object::Tree,
        remaining_hex: &str,
        fanout: u8,
    ) -> Result<gix_object::Tree, Error> {
        if fanout == 0 {
            // Base case: remove the blob entry.
            let entries: Vec<_> = tree
                .entries
                .iter()
                .filter(|e| e.filename.as_ref() as &[u8] != remaining_hex.as_bytes())
                .cloned()
                .collect();
            return Ok(gix_object::Tree { entries });
        }

        let dir_name = &remaining_hex[..2];
        let rest = &remaining_hex[2..];

        let sub_entry = tree
            .entries
            .iter()
            .find(|e| e.mode.is_tree() && e.filename.as_ref() as &[u8] == dir_name.as_bytes());

        let sub_entry = match sub_entry {
            Some(e) => e,
            None => return Ok(tree.clone()),
        };

        let sub_tree = self.lookup_notes_subtree(&sub_entry.oid).map_err(Error::TreeLookup)?;
        let new_sub = self.remove_note_from_subtree(&sub_tree, rest, fanout - 1)?;

        let mut entries: Vec<_> = tree
            .entries
            .iter()
            .filter(|e| !(e.mode.is_tree() && e.filename.as_ref() as &[u8] == dir_name.as_bytes()))
            .cloned()
            .collect();

        if !new_sub.entries.is_empty() {
            let sub_tree_id = self.write_object(&new_sub)?;
            entries.push(gix_object::tree::Entry {
                mode: gix_object::tree::EntryKind::Tree.into(),
                filename: dir_name.into(),
                oid: sub_tree_id.detach(),
            });
            entries.sort();
        }

        Ok(gix_object::Tree { entries })
    }
}
