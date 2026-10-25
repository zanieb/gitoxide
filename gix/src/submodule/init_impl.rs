//! Implementation of `Submodule::init()`, modelled after git's `init_submodule()` in
//! `builtin/submodule--helper.c`.
//!
//! The init operation:
//! 1. Reads the submodule url from `.gitmodules`
//! 2. Writes `submodule.<name>.url` to `.git/config` (unless already set and `force` is false)
//! 3. Optionally writes `submodule.<name>.update` if configured in `.gitmodules`
//!    (but writes `none` for `!command` values, matching git's security behavior)

use crate::{Submodule, bstr::BStr};

/// Initialization
impl Submodule<'_> {
    /// Initialize this submodule by writing its `url` (and optionally `update` strategy) from
    /// `.gitmodules` into the superproject's `.git/config`, making it known to git.
    ///
    /// This is the equivalent of `git submodule init <path>`.
    ///
    /// If the submodule is already initialized (i.e. has a `submodule.<name>.url` in the local config),
    /// this is a no-op unless `force` is `true`.
    ///
    /// ### Security
    ///
    /// Following git's behavior, commands (`!cmd`) in the `update` field of `.gitmodules` cause
    /// `update = none` to be written to the local config (git prints a warning and falls back).
    pub fn init(&self, force: bool) -> Result<(), super::init::Error> {
        self.git_dir()?;

        // Validate submodule path for symlink attacks before any config writes.
        if let Some(worktree) = self.state.repo.workdir()
            && let Ok(sm_path_bstr) = self.path()
        {
            let sm_path = gix_path::from_bstr(sm_path_bstr)?;
            super::validate_submodule_path(&sm_path, worktree)?;
        }

        let name = self.name();

        // Read the local config file from disk FIRST, for both the "already initialized" check
        // and subsequent writes. This ensures we only check local config (not merged config which
        // may include .gitmodules values).
        let config_path = self.state.repo.common_dir().join("config");
        let mut local_config = gix_config::File::from_path_no_includes(config_path.clone(), gix_config::Source::Local)?;

        // Check if already initialized: look for submodule.<name>.url in LOCAL config only.
        let already_initialized = local_config
            .sections_by_name("submodule")
            .into_iter()
            .flatten()
            .any(|s| s.header().subsection_name() == Some(name) && s.value("url").is_some());
        if !force && already_initialized {
            return Ok(());
        }

        let (resolved_url, _) = self.url_for_config_and_transport()?;

        // Write the url — create or update the section
        {
            let mut section = local_config
                .section_mut_or_create_new("submodule", Some(name))
                .expect("'submodule' is a valid section name");
            // Remove existing url values before setting new one, to match git behavior
            while section.remove("url").is_some() {}
            section.push("url", Some(BStr::new(&resolved_url)))?;

            // Set active flag if not already set in local config.
            // Git's init_submodule() sets submodule.<name>.active = true.
            if section.value("active").is_none() {
                section.push("active", Some(BStr::new(b"true")))?;
            }
        }

        // Write the update strategy if configured in .gitmodules.
        // Git writes "none" for !command values (security fallback), matching init_submodule().
        if let Ok(Some(update_strategy)) = self.state.modules.update(name) {
            let update_value: &[u8] = match &update_strategy {
                gix_submodule::config::Update::Checkout => b"checkout",
                gix_submodule::config::Update::Rebase => b"rebase",
                gix_submodule::config::Update::Merge => b"merge",
                gix_submodule::config::Update::None => b"none",
                gix_submodule::config::Update::Command(_cmd) => {
                    // Security: git writes "none" and prints a warning for !command.
                    b"none"
                }
            };
            // Only write if not already set in local config, or if force is true.
            let has_update = local_config
                .sections_by_name("submodule")
                .into_iter()
                .flatten()
                .any(|s| s.header().subsection_name() == Some(name) && s.value("update").is_some());
            if force || !has_update {
                let mut section = local_config
                    .section_mut("submodule", Some(name))
                    .expect("just created above");
                while section.remove("update").is_some() {}
                section.push("update", Some(BStr::new(update_value)))?;
            }
        }

        // Write to disk
        let mut out = Vec::new();
        local_config.write_to(&mut out)?;
        std::fs::write(&config_path, out)?;
        Ok(())
    }
}
