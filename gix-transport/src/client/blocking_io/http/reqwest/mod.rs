use gix_error::{ErrorExt, Result};

/// Apply Git's configured proxy and its host exclusions to a blocking HTTP client.
/// An empty proxy or a `*` exclusion disables proxying. Digest, Negotiate, NTLM and
/// credential-helper lookup for proxy passwords return an unsupported error.
pub fn configure_proxy(
    mut builder: reqwest::blocking::ClientBuilder,
    config: &super::Options,
) -> Result<reqwest::blocking::ClientBuilder> {
    let no_proxy = config
        .no_proxy
        .clone()
        .or_else(|| std::env::var("no_proxy").or_else(|_| std::env::var("NO_PROXY")).ok());
    if config.proxy.as_deref() == Some("") || no_proxy.as_deref().is_some_and(|hosts| hosts.trim() == "*") {
        builder = builder.no_proxy();
    } else if let Some(url) = &config.proxy {
        if !matches!(
            config.proxy_auth_method,
            super::options::ProxyAuthMethod::AnyAuth | super::options::ProxyAuthMethod::Basic
        ) {
            return Err(
                gix_error::unsupported("The reqwest transport supports only Basic HTTP proxy authentication").raise(),
            );
        }
        if config.proxy_authenticate.is_some()
            && reqwest::Url::parse(url)
                .ok()
                .is_none_or(|proxy| proxy.password().is_none())
        {
            return Err(gix_error::unsupported(
                "The reqwest transport does not support Git credential helpers for HTTP proxies",
            )
            .raise());
        }
        let proxy = reqwest::Proxy::all(url.as_str())
            .map_err(|_| gix_error::message("Invalid Git HTTP proxy URL").raise())?
            .no_proxy(no_proxy.as_deref().and_then(reqwest::NoProxy::from_string));
        builder = builder.proxy(proxy);
    }
    Ok(builder)
}

/// An implementation for HTTP requests via `reqwest`.
pub struct Remote {
    /// A worker thread which performs the actual request.
    handle: Option<std::thread::JoinHandle<Result>>,
    /// A channel to send requests (work) to the worker thread.
    request: std::sync::mpsc::SyncSender<remote::Request>,
    /// A channel to receive the result of the prior request.
    response: std::sync::mpsc::Receiver<remote::Response>,
    /// A mechanism for configuring the remote.
    config: crate::client::blocking_io::http::Options,
    /// The effective base URL after an accepted redirect.
    redirected_base_url: std::sync::Arc<parking_lot::Mutex<Option<String>>>,
}

/// A function to configure a single request prior to sending it, support most complex configuration beyond what's possible with
/// basic `git` http configuration.
pub type ConfigureRequestFn = dyn FnMut(&mut reqwest::blocking::Request) -> Result + Send + Sync + 'static;

/// A function to configure the HTTP client before it is built, for example to provide a TLS configuration.
pub type ConfigureClientFn =
    dyn FnMut(reqwest::blocking::ClientBuilder) -> Result<reqwest::blocking::ClientBuilder> + Send + Sync + 'static;

/// Options to configure the reqwest HTTP handler.
#[derive(Default)]
pub struct Options {
    /// Configure the client on the first request, before building and reusing it for subsequent requests.
    ///
    /// The callback runs again if the worker is restarted after a failure. Changes to this callback after
    /// initialization do not affect an existing client. The transport installs its redirect policy after
    /// this callback; use [`Self::configure_request`] for request-specific headers.
    pub configure_client: Option<Box<ConfigureClientFn>>,
    /// A function to configure the request that is about to be made.
    pub configure_request: Option<Box<ConfigureRequestFn>>,
}

///
pub mod remote;
