use std::path::PathBuf;

use crate::{
    config::tree::Core,
    hook::{self, Name, Outcome, Prepared},
    Repository,
};

impl Repository {
    /// Discover the hooks directory, respecting the `core.hooksPath` configuration.
    ///
    /// If `core.hooksPath` is set, it is used (relative paths are resolved relative to the working
    /// directory, or the git directory if bare).
    /// Otherwise, the default `<git-dir>/hooks` directory is used.
    pub fn hooks_dir(&self) -> Result<PathBuf, hook::find::Error> {
        let config = &self.config.resolved;
        let mut filter = self.filter_config_section();
        if let Some(path) = config.path_filter(Core::HOOKS_PATH, &mut filter) {
            let install_dir = crate::path::install_dir().ok();
            let home = crate::config::cache::access::home_dir(self.options.permissions.env);
            let ctx = crate::config::cache::interpolate_context(install_dir.as_deref(), home.as_deref());
            let resolved = path.interpolate(ctx)?;
            let resolved = resolved.into_owned();
            if resolved.is_relative() {
                if let Some(workdir) = self.workdir() {
                    return Ok(workdir.join(resolved));
                }
                return Ok(self.git_dir().join(resolved));
            }
            Ok(resolved)
        } else {
            Ok(self.git_dir().join("hooks"))
        }
    }

    /// Find a specific hook by name.
    ///
    /// Returns `Some(Prepared)` if the hook file exists and is executable (on Unix)
    /// or exists (on Windows). Returns `None` if the hook doesn't exist.
    pub fn find_hook(&self, name: Name) -> Result<Option<Prepared>, hook::find::Error> {
        let hooks_dir = self.hooks_dir()?;
        let path = hooks_dir.join(name.as_str());

        if !path.is_file() {
            return Ok(None);
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = path.metadata() {
                if metadata.permissions().mode() & 0o111 == 0 {
                    return Ok(None);
                }
            }
        }

        Ok(Some(Prepared { path, name }))
    }

    /// Run a hook with the given arguments and optional stdin data.
    ///
    /// If the hook is not found, returns `Ok(None)`.
    /// If the hook is found and executes, returns `Ok(Some(Outcome))`.
    ///
    /// The hook is executed with the repository's command context (GIT_DIR, GIT_WORK_TREE, etc.).
    /// Hook scripts are always executed through a shell to support both scripts and binaries.
    pub fn run_hook(
        &self,
        name: Name,
        args: &[&str],
        stdin_data: Option<&[u8]>,
    ) -> Result<Option<Outcome>, hook::run::Error> {
        let prepared = match self.find_hook(name)? {
            Some(prepared) => prepared,
            None => return Ok(None),
        };

        self.run_prepared_hook(&prepared, args, stdin_data).map(Some)
    }

    /// Run a hook that was previously found with [`find_hook()`](Self::find_hook()), with the given
    /// arguments and optional stdin data.
    ///
    /// The hook is executed with the repository's command context (GIT_DIR, GIT_WORK_TREE, etc.).
    pub fn run_prepared_hook(
        &self,
        prepared: &Prepared,
        args: &[&str],
        stdin_data: Option<&[u8]>,
    ) -> Result<Outcome, hook::run::Error> {
        let ctx = self.command_context()?;

        let stdin_mode = if stdin_data.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        };

        let mut cmd: std::process::Command = gix_command::prepare(prepared.path.as_os_str())
            .command_may_be_shell_script()
            .with_context(ctx)
            .stdin(stdin_mode)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .args(args.iter().copied())
            .into();

        // Git runs hooks with the working directory set to the worktree root (or the git
        // directory for bare repositories).  Match this behavior so that hooks relying on
        // relative paths work correctly.
        if let Some(workdir) = self.workdir() {
            cmd.current_dir(workdir);
        } else {
            cmd.current_dir(self.git_dir());
        }

        let mut child = cmd.spawn().map_err(|e| hook::run::Error::Spawn {
            path: prepared.path.clone(),
            source: e,
        })?;

        if let Some(data) = stdin_data {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(data).map_err(hook::run::Error::WriteStdin)?;
                // Drop stdin to close it, signaling EOF to the child.
            }
        }

        let output = child.wait_with_output().map_err(hook::run::Error::Wait)?;

        Ok(Outcome {
            status: output.status,
            stdout: if output.stdout.is_empty() {
                None
            } else {
                Some(output.stdout)
            },
            stderr: if output.stderr.is_empty() {
                None
            } else {
                Some(output.stderr)
            },
        })
    }

    /// Run a hook and return an error if it fails (non-zero exit code).
    ///
    /// If the hook is not found, this is not considered an error and `Ok(None)` is returned.
    /// If the hook succeeds, `Ok(Some(Outcome))` is returned.
    ///
    /// This is useful for hooks like `pre-commit` where a non-zero exit code means
    /// the operation should be aborted.
    pub fn run_hook_or_fail(
        &self,
        name: Name,
        args: &[&str],
        stdin_data: Option<&[u8]>,
    ) -> Result<Option<Outcome>, hook::run::Error> {
        let prepared = match self.find_hook(name)? {
            Some(prepared) => prepared,
            None => return Ok(None),
        };

        let path = prepared.path.clone();
        let outcome = self.run_prepared_hook(&prepared, args, stdin_data)?;

        if !outcome.success() {
            let code = outcome.status.code().unwrap_or(-1);
            return Err(hook::run::Error::HookFailed { path, code });
        }

        Ok(Some(outcome))
    }
}

impl From<hook::find::Error> for hook::run::Error {
    fn from(err: hook::find::Error) -> Self {
        match err {
            hook::find::Error::InterpolatePath(e) => hook::run::Error::HooksPathInterpolation(e),
        }
    }
}
