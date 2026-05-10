#![allow(clippy::result_large_err)]
use std::borrow::Cow;

use crate::{
    Remote,
    bstr::{BStr, BString, ByteSlice},
    config, remote,
    remote::find,
};

impl crate::Repository {
    /// Create a new remote available at the given `url`.
    ///
    /// It's configured to fetch included tags by default, similar to git.
    /// See [`with_fetch_tags(…)`][Remote::with_fetch_tags()] for a way to change it.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # mod doctest { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/doctest.rs")); }
    /// # let repo = doctest::open_repo(doctest::basic_repo_dir()?)?;
    /// let remote = repo.remote_at("https://github.com/byron/gitoxide")?;
    ///
    /// assert_eq!(remote.name(), None);
    /// assert_eq!(
    ///     remote
    ///         .url(gix::remote::Direction::Fetch)
    ///         .expect("fetch URL")
    ///         .to_bstring(),
    ///     "https://github.com/byron/gitoxide"
    /// );
    /// # Ok(()) }
    /// ```
    pub fn remote_at<Url, E>(&self, url: Url) -> Result<Remote<'_>, remote::init::Error>
    where
        Url: TryInto<gix_url::Url, Error = E>,
        gix_url::parse::Error: From<E>,
    {
        Remote::from_fetch_url(url, true, self)
    }

    /// Create a new remote available at the given `url` similarly to [`remote_at()`][crate::Repository::remote_at()],
    /// but don't rewrite the url according to rewrite rules.
    /// This eliminates a failure mode in case the rewritten URL is faulty, allowing to selectively [apply rewrite
    /// rules][Remote::rewrite_urls()] later and do so non-destructively.
    pub fn remote_at_without_url_rewrite<Url, E>(&self, url: Url) -> Result<Remote<'_>, remote::init::Error>
    where
        Url: TryInto<gix_url::Url, Error = E>,
        gix_url::parse::Error: From<E>,
    {
        Remote::from_fetch_url(url, false, self)
    }

    /// Find the configured remote with the given `name_or_url` or report an error,
    /// similar to [`try_find_remote(…)`][Self::try_find_remote()].
    ///
    /// Note that we will obtain remotes only if we deem them [trustworthy][crate::open::Options::filter_config_section()].
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # mod doctest { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/doctest.rs")); }
    /// # let repo = doctest::open_repo(doctest::remote_repo_dir("clone")?)?;
    /// let remote = repo.find_remote("origin")?;
    ///
    /// assert_eq!(remote.name().expect("configured").as_bstr(), "origin");
    /// assert_eq!(remote.refspecs(gix::remote::Direction::Fetch).len(), 1);
    /// # Ok(()) }
    /// ```
    pub fn find_remote<'a>(&self, name_or_url: impl Into<&'a BStr>) -> Result<Remote<'_>, find::existing::Error> {
        let name_or_url = name_or_url.into();
        Ok(self
            .try_find_remote(name_or_url)
            .ok_or_else(|| find::existing::Error::NotFound {
                name: name_or_url.into(),
            })??)
    }

    /// Find the default remote as configured, or `None` if no such configuration could be found.
    ///
    /// See [`remote_default_name()`](Self::remote_default_name()) for more information on the `direction` parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # mod doctest { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/doctest.rs")); }
    /// # let repo = doctest::open_repo(doctest::remote_repo_dir("clone")?)?;
    /// let remote = repo
    ///     .find_default_remote(gix::remote::Direction::Fetch)
    ///     .expect("configured")?;
    ///
    /// assert_eq!(remote.name().expect("named").as_bstr(), "origin");
    /// # Ok(()) }
    /// ```
    pub fn find_default_remote(
        &self,
        direction: remote::Direction,
    ) -> Option<Result<Remote<'_>, find::existing::Error>> {
        self.remote_default_name(direction)
            .map(|name| self.find_remote(name.as_ref()))
    }

    /// Find the configured remote with the given `name_or_url` or return `None` if it doesn't exist,
    /// for the purpose of fetching or pushing data.
    ///
    /// There are various error kinds related to partial information or incorrectly formatted URLs or ref-specs.
    /// Also note that the created `Remote` may have neither fetch nor push ref-specs set at all.
    ///
    /// Note that ref-specs are de-duplicated right away which may change their order. This doesn't affect matching in any way
    /// as negations/excludes are applied after includes.
    ///
    /// We will only include information if we deem it [trustworthy][crate::open::Options::filter_config_section()].
    pub fn try_find_remote<'a>(&self, name_or_url: impl Into<&'a BStr>) -> Option<Result<Remote<'_>, find::Error>> {
        self.try_find_remote_inner(name_or_url.into(), true)
    }

    /// This method emulate what `git fetch <remote>` does in order to obtain a remote to fetch from.
    ///
    /// As such, with `name_or_url` being `Some`, it will:
    ///
    /// * use `name_or_url` verbatim if it is a URL, creating a new remote in memory as needed.
    /// * find the named remote if `name_or_url` is a remote name
    ///
    /// If `name_or_url` is `None`:
    ///
    /// * use the current `HEAD` branch to find a configured remote
    /// * fall back to either a generally configured remote or the only configured remote.
    ///
    /// Fail if no remote could be found despite all of the above.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # mod doctest { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/doctest.rs")); }
    /// # let repo = doctest::open_repo(doctest::remote_repo_dir("clone")?)?;
    /// let remote_names: Vec<_> = repo.remote_names().into_iter().map(|name| name.to_string()).collect();
    /// assert_eq!(remote_names, vec!["myself".to_owned(), "origin".to_owned()]);
    ///
    /// let remote = repo.find_fetch_remote(None)?;
    /// assert_eq!(remote.name().expect("configured").as_bstr(), "origin");
    /// # Ok(()) }
    /// ```
    pub fn find_fetch_remote(&self, name_or_url: Option<&BStr>) -> Result<Remote<'_>, find::for_fetch::Error> {
        Ok(match name_or_url {
            Some(name) => match self.try_find_remote(name).and_then(Result::ok) {
                Some(remote) => remote,
                None => self.remote_at(gix_url::parse(name)?)?,
            },
            None => self
                .head()?
                .into_remote(remote::Direction::Fetch)
                .transpose()?
                .map(Ok)
                .or_else(|| self.find_default_remote(remote::Direction::Fetch))
                .ok_or(find::for_fetch::Error::ExactlyOneRemoteNotAvailable)??,
        })
    }

    /// Similar to [`try_find_remote()`][Self::try_find_remote()], but removes a failure mode if rewritten URLs turn out to be invalid
    /// as it skips rewriting them.
    /// Use this in conjunction with [`Remote::rewrite_urls()`] to non-destructively apply the rules and keep the failed urls unchanged.
    pub fn try_find_remote_without_url_rewrite<'a>(
        &self,
        name_or_url: impl Into<&'a BStr>,
    ) -> Option<Result<Remote<'_>, find::Error>> {
        self.try_find_remote_inner(name_or_url.into(), false)
    }

    fn try_find_remote_inner<'a>(
        &self,
        name_or_url: impl Into<&'a BStr>,
        rewrite_urls: bool,
    ) -> Option<Result<Remote<'_>, find::Error>> {
        let mut filter = self.filter_config_section();
        let name_or_url = name_or_url.into();
        let mut config_url = |key: &'static config::tree::keys::Url, kind: &'static str| {
            self.config
                .resolved
                .string_filter(&format!("remote.{}.{}", name_or_url, key.name), &mut filter)
                .map(|url| {
                    key.try_into_url(url).map_err(|err| find::Error::Url {
                        kind,
                        remote_name: name_or_url.into(),
                        source: err,
                    })
                })
        };
        let url = config_url(&config::tree::Remote::URL, "fetch");
        let push_url = config_url(&config::tree::Remote::PUSH_URL, "push");
        let config = &self.config.resolved;

        let fetch_specs = config
            .strings_filter(&format!("remote.{}.{}", name_or_url, "fetch"), &mut filter)
            .map(|specs| {
                config_spec(
                    specs,
                    name_or_url,
                    &config::tree::Remote::FETCH,
                    gix_refspec::parse::Operation::Fetch,
                )
            });
        let push_specs = config
            .strings_filter(&format!("remote.{}.{}", name_or_url, "push"), &mut filter)
            .map(|specs| {
                config_spec(
                    specs,
                    name_or_url,
                    &config::tree::Remote::PUSH,
                    gix_refspec::parse::Operation::Push,
                )
            });
        let fetch_tags = config
            .string_filter(&format!("remote.{}.{}", name_or_url, "tagOpt"), &mut filter)
            .map(|value| {
                config::tree::Remote::TAG_OPT
                    .try_into_tag_opt(value)
                    .map_err(Into::into)
            });
        let fetch_tags = match fetch_tags {
            Some(Ok(v)) => v,
            Some(Err(err)) => return Some(Err(err)),
            None => Default::default(),
        };

        match (url, fetch_specs, push_url, push_specs) {
            (None, None, None, None) => self.try_find_legacy_remote(name_or_url, rewrite_urls),
            (None, _, None, _) => Some(Err(find::Error::UrlMissing)),
            (url, fetch_specs, push_url, push_specs) => {
                let url = match url {
                    Some(Ok(v)) => Some(v),
                    Some(Err(err)) => return Some(Err(err)),
                    None => None,
                };
                let push_url = match push_url {
                    Some(Ok(v)) => Some(v),
                    Some(Err(err)) => return Some(Err(err)),
                    None => None,
                };
                let fetch_specs = match fetch_specs {
                    Some(Ok(v)) => v,
                    Some(Err(err)) => return Some(Err(err)),
                    None => Vec::new(),
                };
                let push_specs = match push_specs {
                    Some(Ok(v)) => v,
                    Some(Err(err)) => return Some(Err(err)),
                    None => Vec::new(),
                };

                Some(
                    Remote::from_preparsed_config(
                        Some(name_or_url.to_owned()),
                        url,
                        push_url,
                        fetch_specs,
                        push_specs,
                        rewrite_urls,
                        fetch_tags,
                        self,
                    )
                    .map_err(Into::into),
                )
            }
        }
    }

    fn try_find_legacy_remote(&self, name: &BStr, rewrite_urls: bool) -> Option<Result<Remote<'_>, find::Error>> {
        if !is_valid_legacy_remote_name(name) {
            return None;
        }
        let name_path = gix_path::try_from_bstr(name).ok()?;
        let remotes_file = self.common_dir().join("remotes").join(name_path.as_ref());
        match std::fs::read(&remotes_file) {
            Ok(data) => {
                if let Some(remote) = self.remote_from_legacy_remotes_file(name, data.as_bstr(), rewrite_urls) {
                    return Some(remote);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Some(Err(find::Error::LegacyRemoteFileIo {
                    path: remotes_file,
                    source: err,
                }));
            }
        }

        let branches_file = self.common_dir().join("branches").join(name_path.as_ref());
        match std::fs::read(&branches_file) {
            Ok(data) => self.remote_from_legacy_branches_file(name, data.as_bstr(), rewrite_urls),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => Some(Err(find::Error::LegacyRemoteFileIo {
                path: branches_file,
                source: err,
            })),
        }
    }

    fn remote_from_legacy_remotes_file(
        &self,
        name: &BStr,
        data: &BStr,
        rewrite_urls: bool,
    ) -> Option<Result<Remote<'_>, find::Error>> {
        let mut url = None;
        let mut fetch_specs = Vec::new();
        let mut push_specs = Vec::new();
        for line in data.lines().map(ByteSlice::trim_end) {
            if line.starts_with(b"URL:") {
                let value = line[4..].trim_start();
                url = (!value.is_empty()).then(|| value.as_bstr().to_owned());
            } else if line.starts_with(b"Pull:") {
                fetch_specs.push(Cow::Owned(line[5..].trim_start().as_bstr().to_owned()));
            } else if line.starts_with(b"Push:") {
                push_specs.push(Cow::Owned(line[5..].trim_start().as_bstr().to_owned()));
            }
        }

        Some(parse_legacy_url(url?, name, "fetch").and_then(|url| {
            let fetch_specs = config_spec(
                fetch_specs,
                name,
                &config::tree::Remote::FETCH,
                gix_refspec::parse::Operation::Fetch,
            )?;
            let push_specs = config_spec(
                push_specs,
                name,
                &config::tree::Remote::PUSH,
                gix_refspec::parse::Operation::Push,
            )?;

            Remote::from_preparsed_config(
                Some(name.to_owned()),
                Some(url),
                None,
                fetch_specs,
                push_specs,
                rewrite_urls,
                Default::default(),
                self,
            )
            .map_err(Into::into)
        }))
    }

    fn remote_from_legacy_branches_file(
        &self,
        name: &BStr,
        data: &BStr,
        rewrite_urls: bool,
    ) -> Option<Result<Remote<'_>, find::Error>> {
        let line = data.lines().next()?.trim();
        if line.is_empty() {
            return None;
        }
        let (url, branch) = match line.find_byte(b'#') {
            Some(pos) => (&line[..pos], Cow::Borrowed(line[pos + 1..].as_bstr())),
            None => {
                let branch = self
                    .config
                    .resolved
                    .string(config::tree::Init::DEFAULT_BRANCH)
                    .unwrap_or_else(|| Cow::Borrowed(crate::init::DEFAULT_BRANCH_NAME.into()));
                (line, branch)
            }
        };
        if url.is_empty() {
            return None;
        }

        Some(
            parse_legacy_url(url.as_bstr().to_owned(), name, "fetch").and_then(|url| {
                let fetch_specs = config_spec(
                    [legacy_branches_fetch_spec(branch.as_ref(), name)],
                    name,
                    &config::tree::Remote::FETCH,
                    gix_refspec::parse::Operation::Fetch,
                )?;
                let push_specs = config_spec(
                    [legacy_branches_push_spec(branch.as_ref())],
                    name,
                    &config::tree::Remote::PUSH,
                    gix_refspec::parse::Operation::Push,
                )?;
                Remote::from_preparsed_config(
                    Some(name.to_owned()),
                    Some(url),
                    None,
                    fetch_specs,
                    push_specs,
                    rewrite_urls,
                    Default::default(),
                    self,
                )
                .map_err(Into::into)
            }),
        )
    }
}

fn config_spec<'a, T, Specs>(
    specs: Specs,
    name_or_url: &BStr,
    key: &'static config::tree::keys::Any<T>,
    op: gix_refspec::parse::Operation,
) -> Result<Vec<gix_refspec::RefSpec>, find::Error>
where
    T: config::tree::keys::Validate,
    Specs: IntoIterator<Item = Cow<'a, BStr>>,
{
    let kind = key.name;
    specs
        .into_iter()
        .map(|spec| {
            key.try_into_refspec(spec, op).map_err(|err| find::Error::RefSpec {
                remote_name: name_or_url.into(),
                kind,
                source: err,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|mut specs| {
            specs.sort();
            specs.dedup();
            specs
        })
}

fn parse_legacy_url(url: impl Into<BString>, name: &BStr, kind: &'static str) -> Result<gix_url::Url, find::Error> {
    config::tree::Remote::URL
        .try_into_url(Cow::Owned(url.into()))
        .map_err(|err| find::Error::Url {
            remote_name: name.into(),
            kind,
            source: err,
        })
}

fn legacy_branches_fetch_spec(branch: &BStr, name: &BStr) -> Cow<'static, BStr> {
    let mut spec = BString::from("refs/heads/");
    spec.extend_from_slice(branch);
    spec.extend_from_slice(b":refs/heads/");
    spec.extend_from_slice(name);
    Cow::Owned(spec)
}

fn legacy_branches_push_spec(branch: &BStr) -> Cow<'static, BStr> {
    let mut spec = BString::from("HEAD:refs/heads/");
    spec.extend_from_slice(branch);
    Cow::Owned(spec)
}

fn is_valid_legacy_remote_name(name: &BStr) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.to_str().is_ok()
        && !name.iter().any(|b| matches!(*b, b'\0' | b'/' | b'\\'))
}
