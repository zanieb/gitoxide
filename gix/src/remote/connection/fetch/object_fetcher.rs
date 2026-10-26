use std::sync::atomic::AtomicBool;

use gix_error::ResultExt;

use super::{Connection, ConnectionDetached, Outcome, PrepareDetached, RefMap, Status, Transport};
use crate::bstr::ByteSlice;
use crate::{Result, remote::fetch, types::RemoteDetached};

/// A negotiated transport for fetching additional objects from the same remote.
pub struct ObjectFetcher<T: Transport> {
    pub(super) transport: gix_protocol::SendFlushOnDrop<T>,
    pub(super) remote: RemoteDetached,
    pub(super) handshake: gix_protocol::Handshake,
    pub(super) trace: bool,
}

impl<T: Transport> ObjectFetcher<T> {
    /// Fetch the requested objects into `repo` without repeating discovery. The negotiated
    /// transport and its current authentication state are reused.
    ///
    /// The object verification and garbage collection rules of [`Connection::fetch_objects()`]
    /// apply. The server may refuse an object-ID request.
    #[gix_protocol::bisync::bisync]
    pub async fn fetch<P>(
        &mut self,
        repo: &crate::Repository,
        objects: &[gix_hash::ObjectId],
        progress: P,
        should_interrupt: &AtomicBool,
    ) -> Result<()>
    where
        P: gix_utils::progress::NestedProgress,
        P::SubProgress: 'static,
    {
        if objects.is_empty() {
            return Ok(());
        }
        let mut remote = self.remote.clone();
        set_object_refspecs(&mut remote, objects)?;
        let map = RefMap::from_refs(
            Vec::new(),
            &self.handshake.capabilities,
            fetch::refmap::init::Context {
                fetch_refspecs: remote.fetch_specs.clone(),
                extra_refspecs: Vec::new(),
            },
        )?;
        let repo = without_haves(repo)?;
        let con = ConnectionDetached {
            remote,
            authenticate: None,
            transport_options: None,
            transport: gix_protocol::SendFlushOnDrop::new(&mut self.transport.inner, self.trace),
            handshake: Some(self.handshake.clone()),
            trace: self.trace,
        };
        let outcome = PrepareDetached::new(con, map)
            .receive(&repo, progress, should_interrupt, None)
            .await?;
        finish(&repo, objects, outcome)
    }
}

impl<T: Transport> Connection<'_, '_, '_, T> {
    /// Fetch fully specified object IDs without updating refs. The server may refuse a
    /// request by object ID. An unfiltered pack can contain additional reachable objects.
    ///
    /// Before returning success, verify every requested object is present and release the
    /// pack's `.keep` marker. Callers must keep the objects reachable to prevent their removal
    /// by garbage collection.
    #[gix_protocol::bisync::bisync]
    pub async fn fetch_objects<P>(
        self,
        objects: &[gix_hash::ObjectId],
        mut progress: P,
        should_interrupt: &AtomicBool,
    ) -> Result<()>
    where
        P: gix_utils::progress::NestedProgress,
        P::SubProgress: 'static,
    {
        if objects.is_empty() {
            return Ok(());
        }
        let repo = without_haves(self.remote.repo)?;
        let mut con = self.into_detached();
        set_object_refspecs(&mut con.remote, objects)?;
        let outcome = con
            .prepare_fetch(&repo, &mut progress, Default::default())
            .await?
            .receive(&repo, progress, should_interrupt, None)
            .await?;
        finish(&repo, objects, outcome)
    }
}

fn set_object_refspecs(remote: &mut RemoteDetached, objects: &[gix_hash::ObjectId]) -> Result<()> {
    remote.fetch_tags = fetch::Tags::None;
    let mut hex = gix_hash::Kind::hex_buf();
    remote.fetch_specs = objects
        .iter()
        .map(|id| {
            gix_refspec::parse(
                id.hex_to_buf(&mut hex).as_bytes().as_bstr(),
                gix_refspec::parse::Operation::Fetch,
            )
            .map(|s| s.to_owned())
        })
        .collect::<std::result::Result<_, _>>()?;
    Ok(())
}

fn without_haves(repo: &crate::Repository) -> Result<crate::Repository> {
    // A local commit in a partial clone can still be missing objects beneath it.
    let mut repo = repo.clone();
    let mut config = repo.config_snapshot_mut();
    config.set_value(&crate::config::tree::Fetch::NEGOTIATION_ALGORITHM, "noop")?;
    config.commit()?;
    Ok(repo)
}

fn finish(repo: &crate::Repository, objects: &[gix_hash::ObjectId], outcome: Outcome) -> Result<()> {
    let missing = objects.iter().find(|id| !repo.has_object(id));
    if let Status::Change { write_pack_bundle, .. } = outcome.status
        && let Some(path) = write_pack_bundle.keep_path
    {
        std::fs::remove_file(&path)
            .or_raise(|| gix_error::message!("Failed to remove .keep file at {:?}", path.display()))?;
    }
    if let Some(id) = missing {
        gix_error::bail!(gix_error::validation(format!(
            "The server did not supply requested object {id}"
        )));
    }
    Ok(())
}
