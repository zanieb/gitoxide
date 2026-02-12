/// A module providing the send-pack protocol implementation for pushing changes to a remote.
///
/// ### Protocol Overview (V1/V2 `receive-pack`)
///
/// The push protocol works as follows:
///
/// 1. The client connects to the server's `receive-pack` service
/// 2. The server advertises its references and capabilities
/// 3. The client sends reference update commands (old-oid new-oid refname)
/// 4. The client sends a packfile containing the objects the server needs
/// 5. The server responds with the status of each reference update
mod types;
pub use types::*;

///
pub mod response;

#[cfg(any(feature = "blocking-client", feature = "async-client"))]
#[cfg(feature = "push")]
mod error;
#[cfg(any(feature = "blocking-client", feature = "async-client"))]
#[cfg(feature = "push")]
pub use error::Error;

#[cfg(any(feature = "blocking-client", feature = "async-client"))]
#[cfg(feature = "push")]
pub(crate) mod function;
