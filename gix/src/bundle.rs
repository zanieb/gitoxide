//! Create, verify, and unbundle git bundle files.
//!
//! Git bundles are portable files that contain refs and a packfile. They can be
//! used to transfer repository data without a network connection.

use std::sync::atomic::AtomicBool;

use crate::bstr::BString;
pub use gix_bundle::{create, header, verify, Header, Prerequisite, Ref, Version};
use gix_ref::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};

/// The outcome of unbundling a git bundle into a repository.
#[derive(Debug)]
pub struct UnbundleOutcome {
    /// The parsed bundle header.
    pub header: Header,
    /// The result of writing and indexing the contained pack.
    pub write_pack: gix_pack::bundle::write::Outcome,
    /// Reference edits that were applied for non-pseudo refs advertised by the bundle.
    pub reference_edits: Vec<RefEdit>,
    /// Bundle refs that were intentionally not written, such as `HEAD`.
    pub skipped_refs: Vec<Ref>,
}

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
    #[error(transparent)]
    PackWrite(#[from] gix_pack::bundle::write::Error),
    #[error("Invalid bundled reference name {name:?}")]
    InvalidRefName {
        name: BString,
        #[source]
        source: gix_validate::reference::name::Error,
    },
    #[error("Bundled reference {name:?} points to missing object {id}")]
    MissingRefObject { name: BString, id: gix_hash::ObjectId },
    #[error(transparent)]
    EditReference(#[from] crate::reference::edit::Error),
    #[error("Could not remove pack keep file at '{}'", path.display())]
    RemovePackKeepFile {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("Could not remove imported pack file at '{}'", path.display())]
    RemoveImportedPackFile {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
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

    /// Unbundle `path` into this repository.
    ///
    /// This verifies that all prerequisites are present, writes and indexes the embedded pack in the object database, then
    /// updates all non-pseudo refs advertised by the bundle. Pseudo refs such as `HEAD` are returned in
    /// [`UnbundleOutcome::skipped_refs`] to avoid detaching or otherwise changing the receiving repository's current `HEAD`.
    ///
    /// The pack `.keep` file is removed once at least one reference has been updated, matching fetch's garbage-collection
    /// safety behavior.
    pub fn bundle_unbundle(&self, path: impl AsRef<std::path::Path>) -> Result<UnbundleOutcome, Error> {
        let mut progress = crate::progress::Discard;
        let should_interrupt = AtomicBool::new(false);
        self.bundle_unbundle_with_progress(path, &mut progress, &should_interrupt)
    }

    /// Like [`bundle_unbundle()`](Self::bundle_unbundle()), but reports progress and can be interrupted.
    pub fn bundle_unbundle_with_progress(
        &self,
        path: impl AsRef<std::path::Path>,
        progress: &mut dyn crate::progress::DynNestedProgress,
        should_interrupt: &AtomicBool,
    ) -> Result<UnbundleOutcome, Error> {
        let (header, mut pack_reader) = header::from_path(path, self.object_hash())?;
        header.verify_prerequisites(|id| self.has_object(id))?;

        let mut skipped_refs = Vec::new();
        let mut ref_targets = Vec::new();
        for bundle_ref in &header.refs {
            let name =
                gix_ref::FullName::try_from(bundle_ref.name.clone()).map_err(|source| Error::InvalidRefName {
                    name: bundle_ref.name.clone(),
                    source,
                })?;
            if matches!(
                name.category(),
                Some(
                    gix_ref::Category::PseudoRef
                        | gix_ref::Category::MainPseudoRef
                        | gix_ref::Category::LinkedPseudoRef { .. }
                )
            ) {
                skipped_refs.push(bundle_ref.clone());
            } else {
                ref_targets.push((name, bundle_ref.id));
            }
        }

        let mut write_pack = gix_pack::Bundle::write_to_directory(
            &mut pack_reader,
            Some(&self.objects.store_ref().path().join("pack")),
            progress,
            should_interrupt,
            Some(Box::new({
                let repo = self.clone();
                repo.objects
            })),
            gix_pack::bundle::write::Options {
                object_hash: self.object_hash(),
                ..Default::default()
            },
        )?;

        for bundle_ref in &header.refs {
            if !self.has_object(bundle_ref.id) {
                remove_imported_pack_files(&mut write_pack)?;
                return Err(Error::MissingRefObject {
                    name: bundle_ref.name.clone(),
                    id: bundle_ref.id,
                });
            }
        }

        let mut ref_edits = Vec::new();
        for (name, id) in ref_targets {
            ref_edits.push(RefEdit {
                change: Change::Update {
                    log: LogChange {
                        mode: RefLog::AndReference,
                        force_create_reflog: false,
                        message: "bundle: unbundle".into(),
                    },
                    expected: PreviousValue::Any,
                    new: gix_ref::Target::Object(id),
                },
                name,
                deref: false,
            });
        }

        let reference_edits = self.edit_references(ref_edits)?;
        if !reference_edits.is_empty() || write_pack.index.num_objects == 0 {
            if let Some(path) = write_pack.keep_path.take() {
                std::fs::remove_file(&path).map_err(|source| Error::RemovePackKeepFile { path, source })?;
            }
        }

        Ok(UnbundleOutcome {
            header,
            write_pack,
            reference_edits,
            skipped_refs,
        })
    }
}

fn remove_imported_pack_files(write_pack: &mut gix_pack::bundle::write::Outcome) -> Result<(), Error> {
    let Some(keep_path) = write_pack.keep_path.take() else {
        return Ok(());
    };
    remove_pack_file(keep_path)?;

    if let Some(index_path) = write_pack.index_path.take() {
        remove_pack_file(index_path)?;
    }
    if let Some(data_path) = write_pack.data_path.take() {
        remove_pack_file(data_path)?;
    }
    Ok(())
}

fn remove_pack_file(path: std::path::PathBuf) -> Result<(), Error> {
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::RemoveImportedPackFile { path, source }),
    }
}
