#[cfg(feature = "async-network-client")]
use gix_transport::client::async_io::Transport;
#[cfg(feature = "blocking-network-client")]
use gix_transport::client::blocking_io::Transport;

use crate::{
    remote,
    remote::{fetch::RefMap, ref_map, Connection, Direction},
    Progress,
};

mod error;
pub use error::Error;

/// The status of a single reference update.
pub use gix_protocol::push::response::StatusV1 as RefUpdateStatus;

/// The outcome of a push operation.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The result of the initial mapping of references, the prerequisite for any push.
    pub ref_map: RefMap,
    /// The outcome of the handshake with the server.
    pub handshake: gix_protocol::Handshake,
    /// The per-reference status of the push.
    pub updates: Vec<RefUpdateStatus>,
    /// If `true`, the server reported success for the overall unpack operation.
    pub unpack_ok: bool,
}

///
pub mod prepare {
    /// The error returned by [`prepare_push()`][super::Connection::prepare_push()].
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error("Cannot perform a meaningful push operation without any configured ref-specs")]
        MissingRefSpecs,
        #[error(transparent)]
        RefMap(#[from] crate::remote::ref_map::Error),
    }

    impl gix_protocol::transport::IsSpuriousError for Error {
        fn is_spurious(&self) -> bool {
            match self {
                Error::RefMap(err) => err.is_spurious(),
                _ => false,
            }
        }
    }
}

impl<'remote, 'repo, T> Connection<'remote, 'repo, T>
where
    T: Transport,
{
    /// Perform a handshake with the remote and obtain a ref-map with `options`.
    /// From there, the push operation can be further configured before being performed.
    ///
    /// Note that at this point, the `transport` should already be configured using the
    /// [`transport_mut()`][Self::transport_mut()] method, as it will be consumed here.
    ///
    /// # Note
    ///
    /// The `options` are used to configure the ref-map, which determines which references
    /// will be considered for the push. By default, the remote's push refspecs are used.
    #[allow(clippy::result_large_err)]
    #[gix_protocol::maybe_async::maybe_async]
    pub async fn prepare_push(
        mut self,
        progress: impl Progress,
        options: ref_map::Options,
    ) -> Result<PreparePush<'remote, 'repo, T>, prepare::Error> {
        if self.remote.refspecs(remote::Direction::Push).is_empty() && options.extra_refspecs.is_empty() {
            return Err(prepare::Error::MissingRefSpecs);
        }
        let ref_map = self.ref_map_for_push(progress, options).await?;
        Ok(PreparePush {
            con: Some(self),
            ref_map,
            dry_run: false,
            atomic: false,
            expected_old_ids: None,
        })
    }

    #[allow(clippy::result_large_err)]
    #[gix_protocol::maybe_async::maybe_async]
    async fn ref_map_for_push(
        &mut self,
        mut progress: impl Progress,
        ref_map::Options {
            prefix_from_spec_as_filter_on_remote,
            handshake_parameters,
            extra_refspecs,
        }: ref_map::Options,
    ) -> Result<RefMap, ref_map::Error> {
        let _span = gix_trace::coarse!("remote::Connection::ref_map_for_push()");
        let mut credentials_storage;
        let url = self.transport.inner.to_url();
        let authenticate = match self.authenticate.as_mut() {
            Some(f) => f,
            None => {
                let url = self.remote.url(Direction::Push).map_or_else(
                    || gix_url::parse(url.as_ref()).expect("valid URL to be provided by transport"),
                    ToOwned::to_owned,
                );
                credentials_storage = self.configured_credentials(url)?;
                &mut credentials_storage
            }
        };

        let repo = self.remote.repo;
        if self.transport_options.is_none() {
            self.transport_options = repo
                .transport_options(url.as_ref(), self.remote.name().map(crate::remote::Name::as_bstr))
                .map_err(|err| ref_map::Error::GatherTransportConfig {
                    source: err,
                    url: url.into_owned(),
                })?;
        }
        if let Some(config) = self.transport_options.as_ref() {
            self.transport.inner.configure(&**config)?;
        }

        // For push, we connect to the ReceivePack service.
        let mut handshake = gix_protocol::handshake(
            &mut self.transport.inner,
            gix_transport::Service::ReceivePack,
            authenticate,
            handshake_parameters,
            &mut progress,
        )
        .await?;

        let context = gix_protocol::fetch::refmap::init::Context {
            fetch_refspecs: self.remote.push_specs.clone(),
            extra_refspecs,
        };

        let fetch_refmap = handshake.prepare_lsrefs_or_extract_refmap(
            self.remote.repo.config.user_agent_tuple(),
            prefix_from_spec_as_filter_on_remote,
            context,
        )?;

        #[cfg(feature = "async-network-client")]
        let ref_map = fetch_refmap
            .fetch_async(progress, &mut self.transport.inner, self.trace)
            .await?;

        #[cfg(feature = "blocking-network-client")]
        let ref_map = fetch_refmap.fetch_blocking(progress, &mut self.transport.inner, self.trace)?;

        self.handshake = Some(handshake);
        Ok(ref_map)
    }
}

/// A structure to hold the result of the handshake with the remote and configure the upcoming push operation.
pub struct PreparePush<'remote, 'repo, T>
where
    T: Transport,
{
    con: Option<Connection<'remote, 'repo, T>>,
    ref_map: RefMap,
    dry_run: bool,
    atomic: bool,
    /// Optional force-with-lease overrides: map from remote ref name to expected old OID.
    /// When set, the push will use these values for compare-and-swap instead of the
    /// remote's reported ref values. If the remote's actual value differs from the
    /// expected value, the push for that ref will be rejected.
    pub(crate) expected_old_ids: Option<std::collections::HashMap<crate::bstr::BString, gix_hash::ObjectId>>,
}

impl<T> PreparePush<'_, '_, T>
where
    T: Transport,
{
    /// Return the `ref_map` (that includes the server handshake) which was part of listing refs prior to pushing.
    pub fn ref_map(&self) -> &RefMap {
        &self.ref_map
    }
}

/// Builder
impl<T> PreparePush<'_, '_, T>
where
    T: Transport,
{
    /// If dry run is enabled, no change to the remote repository will be made.
    pub fn with_dry_run(mut self, enabled: bool) -> Self {
        self.dry_run = enabled;
        self
    }

    /// If enabled, request atomic push so that all ref updates succeed or all fail together.
    /// Only available if the server advertises `atomic` capability.
    pub fn with_atomic(mut self, enabled: bool) -> Self {
        self.atomic = enabled;
        self
    }

    /// Set expected old OID values for force-with-lease semantics.
    ///
    /// The map keys are fully-qualified remote ref names (e.g., `refs/heads/main`),
    /// and values are the expected current OID on the remote. If the remote's
    /// actual ref value differs from the expected value, that ref update will
    /// be rejected locally (the push command won't be sent for that ref).
    ///
    /// A null OID value means the ref is expected to not exist on the remote.
    pub fn with_expected_old_ids(
        mut self,
        expected: std::collections::HashMap<crate::bstr::BString, gix_hash::ObjectId>,
    ) -> Self {
        self.expected_old_ids = Some(expected);
        self
    }
}

mod send_pack;
