//! A library for performing object database integrity and connectivity checks
#![deny(unsafe_code, missing_docs)]

use std::{
    collections::VecDeque,
    fmt,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use gix_hash::ObjectId;
use gix_hashtable::HashSet;
use gix_object::{find::existing_object, tree::EntryKind, Data, Exists, Find, FindExt, Kind, ObjectRef};

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
            Error::Interrupted => out.write_str("connectivity check was interrupted"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Find(err) => Some(err),
            Error::Checksum(err) => Some(err),
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
            buf: Default::default(),
            secondary_buf: Default::default(),
        }
    }

    /// Run the connectivity check on the provided commit `oid`.
    ///
    /// ### Algorithm
    ///
    /// Walk the trees and blobs referenced by the commit and verify they exist in the ODB.
    /// Any objects previously encountered by this instance will be skipped silently.
    /// Any referenced blobs that are not present in the ODB will result in a call to the  `missing_cb`.
    /// Missing commits or trees will cause an error to be returned.
    ///     - TODO: consider how to handle a missing commit (invoke `missing_cb`, or possibly return a Result?)
    pub fn check_commit(&mut self, oid: &ObjectId) -> Result<(), existing_object::Error> {
        match self.check_commit_with_options(oid, Options::default()) {
            Ok(()) => Ok(()),
            Err(Error::Find(err)) => Err(err),
            Err(Error::Checksum(_)) => unreachable!("hash verification needs to be configured"),
            Err(Error::Interrupted) => unreachable!("interruptions need a configured interrupt flag"),
        }
    }

    /// Run the connectivity check on the provided commit `oid`, using `options`.
    ///
    /// This is like [`Connectivity::check_commit`], but can report progress and be interrupted.
    pub fn check_commit_with_options(&mut self, oid: &ObjectId, options: Options<'_>) -> Result<(), Error> {
        // Attempt to insert the commit ID in the set, and if already present, return immediately
        if !insert_seen(&mut self.seen, *oid, options)? {
            return Ok(());
        }
        // Obtain the commit's tree ID
        let tree_id = {
            let object = find_existing_object(&self.db, oid, &mut self.buf, Kind::Commit, options)?;
            match decode_object(object, oid)? {
                ObjectRef::Commit(commit) => commit.tree(),
                _ => unreachable!("find_existing_object validates the object kind"),
            }
        };

        self.check_tree_id(&tree_id, options)
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
            Err(Error::Interrupted) => unreachable!("interruptions need a configured interrupt flag"),
        }
    }

    /// Run the connectivity check on the provided annotated tag `oid`, using `options`.
    ///
    /// This is like [`Connectivity::check_tag`], but can report progress and be interrupted.
    pub fn check_tag_with_options(&mut self, oid: &ObjectId, options: Options<'_>) -> Result<(), Error> {
        if !insert_seen(&mut self.seen, *oid, options)? {
            return Ok(());
        }

        let (target, target_kind) = {
            let object = find_existing_object(&self.db, oid, &mut self.buf, Kind::Tag, options)?;
            match decode_object(object, oid)? {
                ObjectRef::Tag(tag) => (tag.target(), tag.target_kind),
                _ => unreachable!("find_existing_object validates the object kind"),
            }
        };

        self.check_referenced_object(&target, target_kind, options)
    }

    fn check_referenced_object(&mut self, oid: &ObjectId, kind: Kind, options: Options<'_>) -> Result<(), Error> {
        match kind {
            Kind::Blob => {
                if insert_seen(&mut self.seen, *oid, options)? {
                    check_blob(&self.db, oid, &mut self.buf, &mut self.missing_cb, options)?;
                }
                Ok(())
            }
            Kind::Tree => self.check_tree_id(oid, options),
            Kind::Commit => {
                if self.db.exists(oid) {
                    self.check_commit_with_options(oid, options)
                } else {
                    if insert_seen(&mut self.seen, *oid, options)? {
                        (self.missing_cb)(oid, Kind::Commit);
                    }
                    Ok(())
                }
            }
            Kind::Tag => {
                if self.db.exists(oid) {
                    self.check_tag_with_options(oid, options)
                } else {
                    if insert_seen(&mut self.seen, *oid, options)? {
                        (self.missing_cb)(oid, Kind::Tag);
                    }
                    Ok(())
                }
            }
        }
    }

    fn check_tree_id(&mut self, oid: &ObjectId, options: Options<'_>) -> Result<(), Error> {
        let mut tree_ids = VecDeque::from_iter(Some(*oid));
        while let Some(tree_id) = tree_ids.pop_front() {
            if insert_seen(&mut self.seen, tree_id, options)? {
                self.check_tree(&tree_id, &mut tree_ids, options)?;
            }
        }
        Ok(())
    }

    /// Blobs are checked right away, trees are stored in `tree_ids` for the parent to iterate them, and only
    /// if they have not been `seen` yet.
    fn check_tree(
        &mut self,
        oid: &ObjectId,
        tree_ids: &mut VecDeque<ObjectId>,
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

        for entry_ref in tree.entries.iter() {
            options.check_interrupted()?;
            match entry_ref.mode.kind() {
                EntryKind::Tree => {
                    let tree_id = entry_ref.oid.to_owned();
                    tree_ids.push_back(tree_id);
                }
                EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link => {
                    let blob_id = entry_ref.oid.to_owned();
                    if insert_seen(&mut self.seen, blob_id, options)? {
                        check_blob(
                            &self.db,
                            &blob_id,
                            &mut self.secondary_buf,
                            &mut self.missing_cb,
                            options,
                        )?;
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

fn insert_seen(seen: &mut HashSet, oid: ObjectId, options: Options<'_>) -> Result<bool, Error> {
    options.check_interrupted()?;
    let was_inserted = seen.insert(oid);
    if was_inserted {
        options.record_progress();
    }
    Ok(was_inserted)
}

fn check_blob<T, F>(
    db: &T,
    oid: &ObjectId,
    buf: &mut Vec<u8>,
    mut missing_cb: F,
    options: Options<'_>,
) -> Result<(), Error>
where
    T: FindExt + Exists,
    F: FnMut(&ObjectId, Kind),
{
    if options.verify_hashes {
        if find_optional_object(db, oid, buf, Kind::Blob, options)?.is_none() {
            missing_cb(oid, Kind::Blob);
        }
    } else if !db.exists(oid) {
        missing_cb(oid, Kind::Blob);
    }
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
        return Ok(Some(Data::new(expected, &[])));
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
    object.decode().map_err(|source| {
        existing_object::Error::Decode {
            oid: oid.as_ref().to_owned(),
            source,
        }
        .into()
    })
}
