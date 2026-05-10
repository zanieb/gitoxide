//! A library for performing object database integrity and connectivity checks
#![deny(unsafe_code, missing_docs)]

use std::{
    collections::VecDeque,
    fmt,
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use gix_hash::ObjectId;
use gix_hashtable::{HashMap, HashSet};
use gix_object::{
    bstr::{BStr, BString},
    find::{existing, existing_object},
    tree::{EntryKind, EntryMode, EntryRef},
    Data, Exists, Find, FindExt, Kind, ObjectRef,
};

/// Options to use while performing a connectivity check.
#[derive(Default, Clone, Copy)]
pub struct Options<'a> {
    /// A flag to poll before advancing the traversal.
    ///
    /// If it is set, the traversal returns [`Error::Interrupted`] as soon as possible.
    pub should_interrupt: Option<&'a AtomicBool>,
    /// A counter to increment for each previously unseen object considered by the traversal.
    pub progress: Option<&'a AtomicUsize>,
    /// If true, validate the checksum of each visited object against its object id.
    pub verify_hashes: bool,
    /// Objects to skip entirely as they are known to be broken.
    pub skip_objects: Option<&'a HashSet>,
    /// If true, report tree entries with group-writable modes.
    pub strict: bool,
    /// If true, report tree entries that are not sorted according to Git's tree order,
    /// along with duplicate entry names.
    pub validate_tree_order: bool,
    /// If true, report tree entries whose modes are not canonical Git tree modes.
    pub validate_tree_modes: bool,
    /// If set, commit parents are not traversed when this predicate returns true for the commit id.
    ///
    /// Use this for shallow boundary commits, whose parents are intentionally absent.
    pub skip_commit_parents: Option<&'a dyn Fn(&ObjectId) -> bool>,
}

impl Options<'_> {
    fn check_interrupted(self) -> Result<(), Error> {
        if self
            .should_interrupt
            .map(|should_interrupt| should_interrupt.load(Ordering::Relaxed))
            .unwrap_or_default()
        {
            Err(Error::Interrupted)
        } else {
            Ok(())
        }
    }

    fn record_progress(self) {
        if let Some(progress) = self.progress {
            progress.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The error returned when checking connectivity with [`Options`].
#[derive(Debug)]
pub enum Error {
    /// The object database failed to provide a required object.
    Find(existing_object::Error),
    /// An object was found, but its content does not match its object id.
    Checksum(gix_object::data::verify::Error),
    /// Strict mode rejected a group-writable tree entry.
    StrictMode {
        /// The tree containing the offending entry.
        tree_id: ObjectId,
        /// The entry name.
        filename: BString,
        /// The offending entry mode.
        mode: EntryMode,
    },
    /// A tree contains duplicate entry names.
    DuplicateTreeEntry {
        /// The tree containing the duplicate entry.
        tree_id: ObjectId,
        /// The duplicate entry name.
        filename: BString,
    },
    /// A tree contains entries that are not sorted according to Git's tree order.
    TreeNotSorted {
        /// The tree containing the out-of-order entries.
        tree_id: ObjectId,
        /// The previous entry name.
        previous: BString,
        /// The current entry name.
        current: BString,
    },
    /// A tree entry uses a non-canonical mode.
    InvalidTreeEntryMode {
        /// The tree containing the invalid entry.
        tree_id: ObjectId,
        /// The entry name.
        filename: BString,
        /// The invalid entry mode.
        mode: EntryMode,
    },
    /// The traversal observed the configured interruption flag.
    Interrupted,
}

impl From<existing_object::Error> for Error {
    fn from(err: existing_object::Error) -> Self {
        Error::Find(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Find(err) => err.fmt(out),
            Error::Checksum(err) => err.fmt(out),
            Error::StrictMode {
                tree_id,
                filename,
                mode,
            } => write!(
                out,
                "Tree {tree_id} contains group-writable entry {filename:?} with mode {mode:o}"
            ),
            Error::DuplicateTreeEntry { tree_id, filename } => {
                write!(out, "Tree {tree_id} contains duplicate entry {filename:?}")
            }
            Error::TreeNotSorted {
                tree_id,
                previous,
                current,
            } => write!(
                out,
                "Tree {tree_id} contains out-of-order entries {previous:?} and {current:?}"
            ),
            Error::InvalidTreeEntryMode {
                tree_id,
                filename,
                mode,
            } => write!(
                out,
                "Tree {tree_id} contains entry {filename:?} with invalid mode {mode:o}"
            ),
            Error::Interrupted => out.write_str("connectivity check was interrupted"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Find(err) => Some(err),
            Error::Checksum(err) => Some(err),
            Error::StrictMode { .. } => None,
            Error::DuplicateTreeEntry { .. } | Error::TreeNotSorted { .. } | Error::InvalidTreeEntryMode { .. } => None,
            Error::Interrupted => None,
        }
    }
}

/// Perform a connectivity check.
pub struct Connectivity<T, F>
where
    T: FindExt + Exists,
    F: FnMut(&ObjectId, Kind),
{
    /// ODB handle to use for the check
    db: T,
    /// Closure to invoke when a missing object is encountered
    missing_cb: F,
    /// Set of Object IDs already (or about to be) scanned during the check
    seen: HashSet,
    /// The first path through which each seen object became reachable.
    reachable_paths: HashMap<ObjectId, BString>,
    /// A buffer to keep a single object at a time.
    buf: Vec<u8>,
    /// A buffer for objects that have to be read while the main buffer is borrowed.
    secondary_buf: Vec<u8>,
}

impl<T, F> Connectivity<T, F>
where
    T: FindExt + Exists,
    F: FnMut(&ObjectId, Kind),
{
    /// Instantiate a connectivity check.
    pub fn new(db: T, missing_cb: F) -> Connectivity<T, F> {
        Connectivity {
            db,
            missing_cb,
            seen: HashSet::default(),
            reachable_paths: HashMap::default(),
            buf: Default::default(),
            secondary_buf: Default::default(),
        }
    }

    /// Run the connectivity check on the provided commit `oid`.
    ///
    /// ### Algorithm
    ///
    /// Walk the parents, trees, and blobs referenced by the commit and verify they exist in the ODB.
    /// Any objects previously encountered by this instance will be skipped silently.
    /// Any referenced blobs or parent commits that are not present in the ODB will result in a call to the `missing_cb`.
    /// Missing root commits, malformed objects, or referenced objects with the wrong kind cause an error to be returned.
    pub fn check_commit(&mut self, oid: &ObjectId) -> Result<(), existing_object::Error> {
        match self.check_commit_with_options(oid, Options::default()) {
            Ok(()) => Ok(()),
            Err(Error::Find(err)) => Err(err),
            Err(Error::Checksum(_)) => unreachable!("hash verification needs to be configured"),
            Err(Error::StrictMode { .. }) => unreachable!("strict mode needs to be configured"),
            Err(
                Error::DuplicateTreeEntry { .. } | Error::TreeNotSorted { .. } | Error::InvalidTreeEntryMode { .. },
            ) => unreachable!("tree entry validation needs to be configured"),
            Err(Error::Interrupted) => unreachable!("interruptions need a configured interrupt flag"),
        }
    }

    /// Run the connectivity check on the provided commit `oid`, using `options`.
    ///
    /// This is like [`Connectivity::check_commit`], but can report progress and be interrupted.
    pub fn check_commit_with_options(&mut self, oid: &ObjectId, options: Options<'_>) -> Result<(), Error> {
        self.check_commit_at(oid, options, root_path("commit", oid))
    }

    fn check_commit_at(&mut self, oid: &ObjectId, options: Options<'_>, path: BString) -> Result<(), Error> {
        // Previously reached objects don't need traversal again, but they must still match this use-site's kind.
        if !insert_seen_at(&mut self.seen, &mut self.reachable_paths, *oid, options, path.clone())? {
            if !is_skipped_object(oid, options) {
                find_existing_object(&self.db, oid, &mut self.buf, Kind::Commit, options)?;
            }
            return Ok(());
        }
        // Obtain the commit's tree ID and parent IDs.
        let skip_parents = options
            .skip_commit_parents
            .map(|skip_commit_parents| skip_commit_parents(oid))
            .unwrap_or_default();
        let (tree_id, parent_ids) = {
            let object = find_existing_object(&self.db, oid, &mut self.buf, Kind::Commit, options)?;
            match decode_object(object, oid)? {
                ObjectRef::Commit(commit) => (commit.tree(), commit.parents().collect::<Vec<_>>()),
                _ => unreachable!("find_existing_object validates the object kind"),
            }
        };

        self.check_tree_id_at(&tree_id, options, child_path(&path, "tree", &tree_id))?;

        if !skip_parents {
            for parent_id in parent_ids {
                self.check_referenced_object_at(
                    &parent_id,
                    Kind::Commit,
                    options,
                    child_path(&path, "parent", &parent_id),
                )?;
            }
        }
        Ok(())
    }

    /// Run the connectivity check on the provided annotated tag `oid`.
    ///
    /// Tags may point to any object kind, including another tag. Missing objects
    /// referenced by the tag are reported through the missing-object callback.
    pub fn check_tag(&mut self, oid: &ObjectId) -> Result<(), existing_object::Error> {
        match self.check_tag_with_options(oid, Options::default()) {
            Ok(()) => Ok(()),
            Err(Error::Find(err)) => Err(err),
            Err(Error::Checksum(_)) => unreachable!("hash verification needs to be configured"),
            Err(Error::StrictMode { .. }) => unreachable!("strict mode needs to be configured"),
            Err(
                Error::DuplicateTreeEntry { .. } | Error::TreeNotSorted { .. } | Error::InvalidTreeEntryMode { .. },
            ) => unreachable!("tree entry validation needs to be configured"),
            Err(Error::Interrupted) => unreachable!("interruptions need a configured interrupt flag"),
        }
    }

    /// Run the connectivity check on the provided annotated tag `oid`, using `options`.
    ///
    /// This is like [`Connectivity::check_tag`], but can report progress and be interrupted.
    pub fn check_tag_with_options(&mut self, oid: &ObjectId, options: Options<'_>) -> Result<(), Error> {
        self.check_tag_at(oid, options, root_path("tag", oid))
    }

    fn check_tag_at(&mut self, oid: &ObjectId, options: Options<'_>, path: BString) -> Result<(), Error> {
        if !insert_seen_at(&mut self.seen, &mut self.reachable_paths, *oid, options, path.clone())? {
            if !is_skipped_object(oid, options) {
                find_existing_object(&self.db, oid, &mut self.buf, Kind::Tag, options)?;
            }
            return Ok(());
        }

        let (target, target_kind) = {
            let object = find_existing_object(&self.db, oid, &mut self.buf, Kind::Tag, options)?;
            match decode_object(object, oid)? {
                ObjectRef::Tag(tag) => (tag.target(), tag.target_kind),
                _ => unreachable!("find_existing_object validates the object kind"),
            }
        };

        self.check_referenced_object_at(
            &target,
            target_kind,
            options,
            child_path(&path, kind_label(target_kind), &target),
        )
    }

    /// Run the connectivity check on the provided object `oid`, detecting its kind from the object database.
    pub fn check_object(&mut self, oid: &ObjectId) -> Result<(), existing_object::Error> {
        match self.check_object_with_options(oid, Options::default()) {
            Ok(()) => Ok(()),
            Err(Error::Find(err)) => Err(err),
            Err(Error::Checksum(_)) => unreachable!("hash verification needs to be configured"),
            Err(Error::StrictMode { .. }) => unreachable!("strict mode needs to be configured"),
            Err(
                Error::DuplicateTreeEntry { .. } | Error::TreeNotSorted { .. } | Error::InvalidTreeEntryMode { .. },
            ) => unreachable!("tree entry validation needs to be configured"),
            Err(Error::Interrupted) => unreachable!("interruptions need a configured interrupt flag"),
        }
    }

    /// Run the connectivity check on the provided object `oid`, detecting its kind from the object database and using `options`.
    ///
    /// This is useful for roots whose kind is not known ahead of time, such as object ids collected from reflog entries.
    pub fn check_object_with_options(&mut self, oid: &ObjectId, options: Options<'_>) -> Result<(), Error> {
        self.check_object_at(oid, options, root_path("object", oid))
    }

    fn check_object_at(&mut self, oid: &ObjectId, options: Options<'_>, path: BString) -> Result<(), Error> {
        if !insert_seen_at(&mut self.seen, &mut self.reachable_paths, *oid, options, path.clone())? {
            if !is_skipped_object(oid, options) {
                find_existing_any_object(&self.db, oid, &mut self.buf, options)?;
            }
            return Ok(());
        }

        enum Root {
            Blob,
            Tree,
            Commit(ObjectId),
            Tag(ObjectId, Kind),
        }

        let root = {
            let object = find_existing_any_object(&self.db, oid, &mut self.buf, options)?;
            match object.kind {
                Kind::Blob => Root::Blob,
                Kind::Tree => Root::Tree,
                Kind::Commit => {
                    let tree_id = match decode_object(object, oid)? {
                        ObjectRef::Commit(commit) => commit.tree(),
                        _ => unreachable!("object kind was matched before decoding"),
                    };
                    Root::Commit(tree_id)
                }
                Kind::Tag => {
                    let (target, target_kind) = match decode_object(object, oid)? {
                        ObjectRef::Tag(tag) => (tag.target(), tag.target_kind),
                        _ => unreachable!("object kind was matched before decoding"),
                    };
                    Root::Tag(target, target_kind)
                }
            }
        };

        match root {
            Root::Blob => Ok(()),
            Root::Tree => {
                let mut tree_ids = VecDeque::new();
                self.check_tree(oid, &path, &mut tree_ids, options)?;
                while let Some((tree_id, tree_path)) = tree_ids.pop_front() {
                    if insert_seen_at(
                        &mut self.seen,
                        &mut self.reachable_paths,
                        tree_id,
                        options,
                        tree_path.clone(),
                    )? {
                        self.check_tree(&tree_id, &tree_path, &mut tree_ids, options)?;
                    } else if !is_skipped_object(&tree_id, options) {
                        ensure_optional_object_kind(&self.db, &tree_id, &mut self.buf, Kind::Tree, options)?;
                    }
                }
                Ok(())
            }
            Root::Commit(tree_id) => self.check_tree_id_at(&tree_id, options, child_path(&path, "tree", &tree_id)),
            Root::Tag(target, target_kind) => self.check_referenced_object_at(
                &target,
                target_kind,
                options,
                child_path(&path, kind_label(target_kind), &target),
            ),
        }
    }

    /// Run connectivity checks on all non-null old and new ids in reflog entries.
    ///
    /// Each item is `(old_id, new_id)`. Null ids are skipped, as they represent creation or deletion boundaries.
    pub fn check_reflog_entries(
        &mut self,
        entries: impl IntoIterator<Item = (ObjectId, ObjectId)>,
    ) -> Result<(), existing_object::Error> {
        match self.check_reflog_entries_with_options(entries, Options::default()) {
            Ok(()) => Ok(()),
            Err(Error::Find(err)) => Err(err),
            Err(Error::Checksum(_)) => unreachable!("hash verification needs to be configured"),
            Err(Error::StrictMode { .. }) => unreachable!("strict mode needs to be configured"),
            Err(
                Error::DuplicateTreeEntry { .. } | Error::TreeNotSorted { .. } | Error::InvalidTreeEntryMode { .. },
            ) => unreachable!("tree entry validation needs to be configured"),
            Err(Error::Interrupted) => unreachable!("interruptions need a configured interrupt flag"),
        }
    }

    /// Run connectivity checks on all non-null old and new ids in reflog entries, using `options`.
    ///
    /// Each item is `(old_id, new_id)`. Null ids are skipped, as they represent creation or deletion boundaries.
    pub fn check_reflog_entries_with_options(
        &mut self,
        entries: impl IntoIterator<Item = (ObjectId, ObjectId)>,
        options: Options<'_>,
    ) -> Result<(), Error> {
        for (old_id, new_id) in entries {
            if !old_id.is_null() {
                self.check_object_at(&old_id, options, root_path("reflog-old", &old_id))?;
            }
            if !new_id.is_null() {
                self.check_object_at(&new_id, options, root_path("reflog-new", &new_id))?;
            }
        }
        Ok(())
    }

    /// Run connectivity checks on all non-null ids from index entries.
    ///
    /// Blob, executable, and symlink entries are checked as blobs. Tree entries, as used by sparse indices,
    /// are traversed as trees. Commit entries represent submodules and are skipped because they belong to a different repository.
    pub fn check_index_entries(
        &mut self,
        entries: impl IntoIterator<Item = (ObjectId, EntryKind)>,
    ) -> Result<(), existing_object::Error> {
        match self.check_index_entries_with_options(entries, Options::default()) {
            Ok(()) => Ok(()),
            Err(Error::Find(err)) => Err(err),
            Err(Error::Checksum(_)) => unreachable!("hash verification needs to be configured"),
            Err(Error::StrictMode { .. }) => unreachable!("strict mode needs to be configured"),
            Err(
                Error::DuplicateTreeEntry { .. } | Error::TreeNotSorted { .. } | Error::InvalidTreeEntryMode { .. },
            ) => unreachable!("tree entry validation needs to be configured"),
            Err(Error::Interrupted) => unreachable!("interruptions need a configured interrupt flag"),
        }
    }

    /// Run connectivity checks on all non-null ids from index entries, using `options`.
    ///
    /// See [`Connectivity::check_index_entries`] for the mapping from index entry kind to object traversal.
    pub fn check_index_entries_with_options(
        &mut self,
        entries: impl IntoIterator<Item = (ObjectId, EntryKind)>,
        options: Options<'_>,
    ) -> Result<(), Error> {
        for (oid, kind) in entries {
            if !oid.is_null() {
                self.check_index_entry_with_options(&oid, kind, options, root_path("index-entry", &oid))?;
            }
        }
        Ok(())
    }

    /// Run connectivity checks on all non-null tree ids from an index tree-cache extension.
    pub fn check_index_tree_cache(
        &mut self,
        tree_ids: impl IntoIterator<Item = ObjectId>,
    ) -> Result<(), existing_object::Error> {
        match self.check_index_tree_cache_with_options(tree_ids, Options::default()) {
            Ok(()) => Ok(()),
            Err(Error::Find(err)) => Err(err),
            Err(Error::Checksum(_)) => unreachable!("hash verification needs to be configured"),
            Err(Error::StrictMode { .. }) => unreachable!("strict mode needs to be configured"),
            Err(
                Error::DuplicateTreeEntry { .. } | Error::TreeNotSorted { .. } | Error::InvalidTreeEntryMode { .. },
            ) => unreachable!("tree entry validation needs to be configured"),
            Err(Error::Interrupted) => unreachable!("interruptions need a configured interrupt flag"),
        }
    }

    /// Run connectivity checks on all non-null tree ids from an index tree-cache extension, using `options`.
    pub fn check_index_tree_cache_with_options(
        &mut self,
        tree_ids: impl IntoIterator<Item = ObjectId>,
        options: Options<'_>,
    ) -> Result<(), Error> {
        for tree_id in tree_ids {
            if !tree_id.is_null() {
                self.check_tree_id_at(&tree_id, options, root_path("index-tree-cache", &tree_id))?;
            }
        }
        Ok(())
    }

    /// Return all objects from `object_ids` that were not reached by previous connectivity checks.
    ///
    /// The input should be the object ids known to exist in the object database. Missing objects reported while
    /// traversing are remembered as seen, but they won't affect this result unless the caller includes them here.
    pub fn unreachable<'a>(&self, object_ids: impl IntoIterator<Item = &'a ObjectId>) -> Vec<ObjectId> {
        object_ids
            .into_iter()
            .filter(|oid| !self.seen.contains(*oid))
            .copied()
            .collect()
    }

    /// Return the first recorded reachability path for `oid`.
    pub fn path_to(&self, oid: &ObjectId) -> Option<&BString> {
        self.reachable_paths.get(oid)
    }

    /// Iterate over all seen object ids and their first recorded reachability paths.
    pub fn reachable_paths(&self) -> impl Iterator<Item = (&ObjectId, &BString)> {
        self.reachable_paths.iter()
    }

    fn check_index_entry_with_options(
        &mut self,
        oid: &ObjectId,
        kind: EntryKind,
        options: Options<'_>,
        path: BString,
    ) -> Result<(), Error> {
        match kind {
            EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link => {
                if insert_seen_at(&mut self.seen, &mut self.reachable_paths, *oid, options, path)? {
                    check_blob(&self.db, oid, &mut self.buf, &mut self.missing_cb, options)?;
                } else if !is_skipped_object(oid, options) {
                    ensure_optional_object_kind(&self.db, oid, &mut self.buf, Kind::Blob, options)?;
                }
                Ok(())
            }
            EntryKind::Tree => self.check_tree_id_at(oid, options, path),
            EntryKind::Commit => Ok(()),
        }
    }

    fn check_referenced_object_at(
        &mut self,
        oid: &ObjectId,
        kind: Kind,
        options: Options<'_>,
        path: BString,
    ) -> Result<(), Error> {
        match kind {
            Kind::Blob => {
                if insert_seen_at(&mut self.seen, &mut self.reachable_paths, *oid, options, path)? {
                    check_blob(&self.db, oid, &mut self.buf, &mut self.missing_cb, options)?;
                } else if !is_skipped_object(oid, options) {
                    ensure_optional_object_kind(&self.db, oid, &mut self.buf, Kind::Blob, options)?;
                }
                Ok(())
            }
            Kind::Tree => self.check_tree_id_at(oid, options, path),
            Kind::Commit => {
                if self.db.exists(oid) {
                    self.check_commit_at(oid, options, path)
                } else {
                    if insert_seen_at(&mut self.seen, &mut self.reachable_paths, *oid, options, path)? {
                        (self.missing_cb)(oid, Kind::Commit);
                    }
                    Ok(())
                }
            }
            Kind::Tag => {
                if self.db.exists(oid) {
                    self.check_tag_at(oid, options, path)
                } else {
                    if insert_seen_at(&mut self.seen, &mut self.reachable_paths, *oid, options, path)? {
                        (self.missing_cb)(oid, Kind::Tag);
                    }
                    Ok(())
                }
            }
        }
    }

    fn check_tree_id_at(&mut self, oid: &ObjectId, options: Options<'_>, path: BString) -> Result<(), Error> {
        let mut tree_ids = VecDeque::from_iter(Some((*oid, path)));
        while let Some((tree_id, tree_path)) = tree_ids.pop_front() {
            if insert_seen_at(
                &mut self.seen,
                &mut self.reachable_paths,
                tree_id,
                options,
                tree_path.clone(),
            )? {
                self.check_tree(&tree_id, &tree_path, &mut tree_ids, options)?;
            } else if !is_skipped_object(&tree_id, options) {
                ensure_optional_object_kind(&self.db, &tree_id, &mut self.buf, Kind::Tree, options)?;
            }
        }
        Ok(())
    }

    /// Blobs are checked right away, trees are stored in `tree_ids` for the parent to iterate them, and only
    /// if they have not been `seen` yet.
    fn check_tree(
        &mut self,
        oid: &ObjectId,
        path: &BString,
        tree_ids: &mut VecDeque<(ObjectId, BString)>,
        options: Options<'_>,
    ) -> Result<(), Error> {
        let Some(object) = find_optional_object(&self.db, oid, &mut self.buf, Kind::Tree, options)? else {
            (self.missing_cb)(oid, Kind::Tree);
            return Ok(());
        };
        let tree = match decode_object(object, oid)? {
            ObjectRef::Tree(tree) => tree,
            _ => unreachable!("find_optional_object validates the object kind"),
        };

        let mut previous_entry: Option<EntryRef<'_>> = None;
        for entry_ref in tree.entries.iter() {
            options.check_interrupted()?;
            if options.validate_tree_order {
                if let Some(previous) = previous_entry {
                    if previous.filename == entry_ref.filename {
                        return Err(Error::DuplicateTreeEntry {
                            tree_id: *oid,
                            filename: entry_ref.filename.into(),
                        });
                    }
                    if previous.cmp(entry_ref) != std::cmp::Ordering::Less {
                        return Err(Error::TreeNotSorted {
                            tree_id: *oid,
                            previous: previous.filename.into(),
                            current: entry_ref.filename.into(),
                        });
                    }
                }
                previous_entry = Some(*entry_ref);
            }
            if options.validate_tree_modes {
                let canonical_mode: EntryMode = entry_ref.mode.kind().into();
                if entry_ref.mode != canonical_mode {
                    return Err(Error::InvalidTreeEntryMode {
                        tree_id: *oid,
                        filename: entry_ref.filename.into(),
                        mode: entry_ref.mode,
                    });
                }
            }
            if options.strict && entry_ref.mode.value() & 0o020 != 0 {
                return Err(Error::StrictMode {
                    tree_id: *oid,
                    filename: entry_ref.filename.into(),
                    mode: entry_ref.mode,
                });
            }
            match entry_ref.mode.kind() {
                EntryKind::Tree => {
                    let tree_id = entry_ref.oid.to_owned();
                    tree_ids.push_back((tree_id, tree_entry_path(path, entry_ref.filename)));
                }
                EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link => {
                    let blob_id = entry_ref.oid.to_owned();
                    if insert_seen_at(
                        &mut self.seen,
                        &mut self.reachable_paths,
                        blob_id,
                        options,
                        tree_entry_path(path, entry_ref.filename),
                    )? {
                        check_blob(
                            &self.db,
                            &blob_id,
                            &mut self.secondary_buf,
                            &mut self.missing_cb,
                            options,
                        )?;
                    } else if !is_skipped_object(&blob_id, options) {
                        ensure_optional_object_kind(&self.db, &blob_id, &mut self.secondary_buf, Kind::Blob, options)?;
                    }
                }
                EntryKind::Commit => {
                    // Skip submodules as they wouldn't be in this repository!
                }
            }
        }
        Ok(())
    }
}

/// Write `object_ids` into the `lost-found` directories below `git_dir`.
///
/// Commit objects are written to `lost-found/commit/<object-id>`, all other object kinds to
/// `lost-found/other/<object-id>`. Existing files are left unchanged.
pub fn write_lost_found<'a, T>(
    git_dir: impl AsRef<Path>,
    objects: &T,
    object_ids: impl IntoIterator<Item = &'a ObjectId>,
) -> Result<usize, lost_found::Error>
where
    T: Find,
{
    let git_dir = git_dir.as_ref();
    let mut buf = Vec::new();
    let mut written = 0;
    for oid in object_ids {
        let object = objects
            .try_find(oid, &mut buf)
            .map_err(existing::Error::Find)?
            .ok_or_else(|| existing::Error::NotFound { oid: *oid })?;
        let dir = git_dir
            .join("lost-found")
            .join(if object.kind == Kind::Commit { "commit" } else { "other" });
        std::fs::create_dir_all(&dir).map_err(|source| lost_found::Error::CreateDirectory {
            source,
            directory: dir.clone(),
        })?;
        let path = dir.join(oid.to_string());
        let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(lost_found::Error::CreateFile { source, path }),
        };
        file.write_all(object.data)
            .map_err(|source| lost_found::Error::Write { source, path })?;
        written += 1;
    }
    Ok(written)
}

/// Errors and support types for writing dangling objects into `lost-found`.
pub mod lost_found {
    use std::{fmt, path::PathBuf};

    /// The error returned by [`crate::write_lost_found()`].
    #[derive(Debug)]
    pub enum Error {
        /// The object database failed to provide a dangling object.
        Find(gix_object::find::existing::Error),
        /// A directory in the `lost-found` hierarchy could not be created.
        CreateDirectory {
            /// The underlying IO error.
            source: std::io::Error,
            /// The directory that could not be created.
            directory: PathBuf,
        },
        /// A file in the `lost-found` hierarchy could not be created.
        CreateFile {
            /// The underlying IO error.
            source: std::io::Error,
            /// The file that could not be created.
            path: PathBuf,
        },
        /// The dangling object data could not be written.
        Write {
            /// The underlying IO error.
            source: std::io::Error,
            /// The file that could not be written.
            path: PathBuf,
        },
    }

    impl From<gix_object::find::existing::Error> for Error {
        fn from(err: gix_object::find::existing::Error) -> Self {
            Error::Find(err)
        }
    }

    impl fmt::Display for Error {
        fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Error::Find(err) => err.fmt(out),
                Error::CreateDirectory { directory, .. } => {
                    write!(out, "Could not create lost-found directory {}", directory.display())
                }
                Error::CreateFile { path, .. } => write!(out, "Could not create lost-found file {}", path.display()),
                Error::Write { path, .. } => write!(out, "Could not write lost-found file {}", path.display()),
            }
        }
    }

    impl std::error::Error for Error {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Error::Find(err) => Some(err),
                Error::CreateDirectory { source, .. }
                | Error::CreateFile { source, .. }
                | Error::Write { source, .. } => Some(source),
            }
        }
    }
}

fn is_skipped_object(oid: &ObjectId, options: Options<'_>) -> bool {
    options
        .skip_objects
        .map(|skip_objects| skip_objects.contains(oid))
        .unwrap_or_default()
}

fn insert_seen(seen: &mut HashSet, oid: ObjectId, options: Options<'_>) -> Result<bool, Error> {
    options.check_interrupted()?;
    if is_skipped_object(&oid, options) {
        return Ok(false);
    }
    let was_inserted = seen.insert(oid);
    if was_inserted {
        options.record_progress();
    }
    Ok(was_inserted)
}

fn insert_seen_at(
    seen: &mut HashSet,
    reachable_paths: &mut HashMap<ObjectId, BString>,
    oid: ObjectId,
    options: Options<'_>,
    path: BString,
) -> Result<bool, Error> {
    let inserted = insert_seen(seen, oid, options)?;
    if inserted {
        reachable_paths.insert(oid, path);
    }
    Ok(inserted)
}

fn root_path(label: &str, oid: &ObjectId) -> BString {
    format!("{label} {oid}").into()
}

fn child_path(parent: &BString, label: &str, oid: &ObjectId) -> BString {
    let mut path: Vec<u8> = parent.clone().into();
    path.extend_from_slice(b" -> ");
    path.extend_from_slice(label.as_bytes());
    path.push(b' ');
    path.extend_from_slice(oid.to_string().as_bytes());
    path.into()
}

fn tree_entry_path(parent: &BString, filename: &BStr) -> BString {
    let mut path: Vec<u8> = parent.clone().into();
    path.extend_from_slice(b" -> ");
    path.extend_from_slice(filename);
    path.into()
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Tree => "tree",
        Kind::Blob => "blob",
        Kind::Commit => "commit",
        Kind::Tag => "tag",
    }
}

fn check_blob<T, F>(
    db: &T,
    oid: &ObjectId,
    buf: &mut Vec<u8>,
    mut missing_cb: F,
    options: Options<'_>,
) -> Result<(), Error>
where
    T: Find,
    F: FnMut(&ObjectId, Kind),
{
    if find_optional_object(db, oid, buf, Kind::Blob, options)?.is_none() {
        missing_cb(oid, Kind::Blob);
    }
    Ok(())
}

fn ensure_optional_object_kind<T>(
    db: &T,
    oid: &ObjectId,
    buf: &mut Vec<u8>,
    expected: Kind,
    options: Options<'_>,
) -> Result<(), Error>
where
    T: Find,
{
    find_optional_object(db, oid, buf, expected, options)?;
    Ok(())
}

fn find_existing_object<'a, T>(
    db: &T,
    oid: &ObjectId,
    buf: &'a mut Vec<u8>,
    expected: Kind,
    options: Options<'_>,
) -> Result<Data<'a>, Error>
where
    T: Find,
{
    find_optional_object(db, oid, buf, expected, options)?.ok_or_else(|| {
        existing_object::Error::NotFound {
            oid: oid.as_ref().to_owned(),
        }
        .into()
    })
}

fn find_existing_any_object<'a, T>(
    db: &T,
    oid: &ObjectId,
    buf: &'a mut Vec<u8>,
    options: Options<'_>,
) -> Result<Data<'a>, Error>
where
    T: Find,
{
    find_optional_any_object(db, oid, buf, options)?.ok_or_else(|| {
        existing_object::Error::NotFound {
            oid: oid.as_ref().to_owned(),
        }
        .into()
    })
}

fn find_optional_any_object<'a, T>(
    db: &T,
    oid: &ObjectId,
    buf: &'a mut Vec<u8>,
    options: Options<'_>,
) -> Result<Option<Data<'a>>, Error>
where
    T: Find,
{
    if *oid == ObjectId::empty_tree(oid.kind()) {
        return Ok(Some(Data::new_with_hash(Kind::Tree, &[], oid.kind())));
    }
    if *oid == ObjectId::empty_blob(oid.kind()) {
        return Ok(Some(Data::new_with_hash(Kind::Blob, &[], oid.kind())));
    }

    let Some(object) = db.try_find(oid, buf).map_err(existing_object::Error::Find)? else {
        return Ok(None);
    };
    if options.verify_hashes {
        object.verify_checksum(oid).map_err(Error::Checksum)?;
    }
    Ok(Some(object))
}

fn find_optional_object<'a, T>(
    db: &T,
    oid: &ObjectId,
    buf: &'a mut Vec<u8>,
    expected: Kind,
    options: Options<'_>,
) -> Result<Option<Data<'a>>, Error>
where
    T: Find,
{
    let expected_empty_id = match expected {
        Kind::Tree => Some(ObjectId::empty_tree(oid.kind())),
        Kind::Blob => Some(ObjectId::empty_blob(oid.kind())),
        Kind::Commit | Kind::Tag => None,
    };
    if expected_empty_id.as_ref().is_some_and(|empty_id| empty_id == oid) {
        return Ok(Some(Data::new_with_hash(expected, &[], oid.kind())));
    }

    let Some(object) = db.try_find(oid, buf).map_err(existing_object::Error::Find)? else {
        return Ok(None);
    };
    if options.verify_hashes {
        object.verify_checksum(oid).map_err(Error::Checksum)?;
    }
    if object.kind != expected {
        return Err(existing_object::Error::ObjectKind {
            oid: oid.as_ref().to_owned(),
            actual: object.kind,
            expected,
        }
        .into());
    }
    Ok(Some(object))
}

fn decode_object<'a>(object: Data<'a>, oid: &ObjectId) -> Result<ObjectRef<'a>, Error> {
    object.decode_with_hash(oid.kind()).map_err(|source| {
        existing_object::Error::Decode {
            oid: oid.as_ref().to_owned(),
            source,
        }
        .into()
    })
}
