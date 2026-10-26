use crate::{Result, config::Snapshot};

impl Snapshot<'_> {
    /// Returns the configuration for all git-credential helpers from trusted configuration that apply
    /// to the given `url` along with an action preconfigured to invoke the cascade with.
    /// For details, please see [this function](function::credential_helpers).
    pub fn credential_helpers(
        &self,
        url: gix_url::Url,
    ) -> Result<(
        gix_credentials::helper::Cascade,
        gix_credentials::helper::Action,
        gix_prompt::Options,
    )> {
        let repo = self.repo;
        function::credential_helpers(
            url,
            &repo.config.resolved,
            repo.config.lenient_config,
            &mut repo.filter_config_section(),
            repo.config.environment,
            false,
        )
    }
}

pub(super) mod function {
    use gix_error::ResultExt;

    use crate::{
        Result,
        bstr::{ByteSlice, ByteVec},
        config::{
            cache::util::ApplyLeniency,
            tree::{Core, Credential, credential, gitoxide::Credentials},
        },
    };

    /// Returns the configuration for all git-credential helpers from trusted configuration that apply
    /// to the given `url` along with an action preconfigured to invoke the cascade with to retrieve it.
    /// This includes `url` which may be altered to contain a user-name as configured.
    ///
    /// These can be invoked to obtain credentials. Note that the `url` is expected to be the one used
    /// to connect to a remote, and thus should already have passed the url-rewrite engine.
    ///
    /// * `config`
    ///     - the configuration to obtain credential helper configuration from.
    /// * `is_lenient_config`
    ///     - if `true`, minor configuration errors will be ignored.
    /// * `filter`
    ///     - A way to choose which sections in `config` can be trusted. This is important as we will execute programs
    ///       from the paths contained within.
    /// * `environment`
    ///     - Determines how environment variables can be used.
    ///     - Actually used are `GIT_*` and `SSH_*` environment variables to configure git prompting capabilities.
    /// * `use_http_path`
    ///     - Typically, this should be false to let the `url` configuration decide if the value should be enabled.
    ///     - If `false`, credentials are effectively per host.
    ///
    /// # Deviation
    ///
    /// - Invalid urls can't be used to obtain credential helpers as they are rejected early when creating a valid `url` here.
    /// - Parsed urls will automatically drop the port if it's the default, i.e. `http://host:80` becomes `http://host` when parsed.
    ///   This affects the prompt provided to the user, so that git will use the verbatim url, whereas we use `http://host`.
    /// - Upper-case scheme and host will be lower-cased automatically when parsing into a url, so prompts differ compared to git.
    /// - A **difference in prompt might affect the matching of getting existing stored credentials**, and it's a question of this being
    ///   a feature or a bug.
    // TODO: when dealing with `http.*.*` configuration, generalize this algorithm as needed and support precedence.
    pub fn credential_helpers(
        mut url: gix_url::Url,
        config: &gix_config::File,
        is_lenient_config: bool,
        mut filter: impl FnMut(&gix_config::file::Metadata) -> bool,
        environment: crate::open::permissions::Environment,
        mut use_http_path: bool,
    ) -> Result<(
        gix_credentials::helper::Cascade,
        gix_credentials::helper::Action,
        gix_prompt::Options,
    )> {
        let mut programs = Vec::new();
        let mut context_options = gix_credentials::protocol::ContextOptions::default();
        let url_had_user_initially = url.user().is_some();
        crate::config::url_match::normalize(&mut url);

        if let Some(credential_sections) = config.sections_by_name_and_filter("credential", &mut filter) {
            for section in credential_sections {
                let section = match section.header().subsection_name() {
                    Some(pattern) => crate::config::url_match::specificity(pattern, &url).map(|_| {
                        (
                            section,
                            &credential::UrlParameter::HELPER,
                            &credential::UrlParameter::USERNAME,
                            &credential::UrlParameter::USE_HTTP_PATH,
                            &credential::UrlParameter::PROTECT_PROTOCOL,
                        )
                    }),
                    None => Some((
                        section,
                        &Credential::HELPER,
                        &Credential::USERNAME,
                        &Credential::USE_HTTP_PATH,
                        &Credential::PROTECT_PROTOCOL,
                    )),
                };
                if let Some((section, helper_key, username_key, use_http_path_key, protect_protocol_key)) = section {
                    for value in section.values(helper_key.name) {
                        if value.trim().is_empty() {
                            programs.clear();
                        } else {
                            programs.push(gix_credentials::Program::from_custom_definition(value)?);
                        }
                    }
                    if let Some(Some(user)) = (!url_had_user_initially).then(|| {
                        section
                            .value(username_key.name)
                            .filter(|n| !n.trim().is_empty())
                            .and_then(|n| {
                                let n: Vec<_> = n.into();
                                n.into_string().ok()
                            })
                    }) {
                        url.set_user(Some(user));
                    }
                    if let Some(toggle) = section
                        .value(use_http_path_key.name)
                        .map(|val| {
                            gix_config::Boolean::try_from(val)
                                .or_raise(|| {
                                    gix_error::validation(format!(
                                        "Could not parse 'useHttpPath' key in section {}",
                                        section.header().to_bstring()
                                    ))
                                })
                                .map(|b| b.0)
                        })
                        .transpose()?
                    {
                        use_http_path = toggle;
                    }
                    if let Some(toggle) = section
                        .value(protect_protocol_key.name)
                        .map(|value| {
                            protect_protocol_key
                                .enrich_error(gix_config::Boolean::try_from(value).map(|value| Some(value.0)))
                        })
                        .transpose()?
                        .flatten()
                    {
                        context_options.protect_protocol = toggle;
                    }
                }
            }
        }

        let allow_git_env = environment.git_prefix.is_allowed();
        let allow_ssh_env = environment.ssh_prefix.is_allowed();
        let prompt_options = gix_prompt::Options {
            askpass: crate::config::cache::access::trusted_file_path(
                config,
                Core::ASKPASS,
                &mut filter,
                true, // An empty core.askpass is allowed even with strict configuration.
                environment,
            )
            .or_raise(|| gix_error::message("core.askpass could not be read"))?,
            mode: Credentials::TERMINAL_PROMPT
                .enrich_error(config.boolean(Credentials::TERMINAL_PROMPT))
                .with_leniency(is_lenient_config)?
                .and_then(|val| (!val).then_some(gix_prompt::Mode::Disable))
                .unwrap_or_default(),
        }
        .apply_environment(allow_git_env, allow_ssh_env, false /* terminal prompt */);
        let action = gix_credentials::helper::Action::Get(gix_credentials::protocol::Context::from_url(
            url.to_bstring(),
            context_options,
        ));
        Ok((
            gix_credentials::helper::Cascade {
                programs,
                use_http_path,
                context_options,
                // The default ssh implementation uses binaries that do their own auth, so our passwords aren't used.
                query_user_only: url.scheme == gix_url::Scheme::Ssh,
                stderr: Credentials::HELPER_STDERR
                    .enrich_error(config.boolean(Credentials::HELPER_STDERR))
                    .with_leniency(is_lenient_config)?
                    .unwrap_or(true),
            },
            action,
            prompt_options,
        ))
    }
}
