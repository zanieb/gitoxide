use anyhow::bail;
use gix::{commit::describe::SelectRef, prelude::ObjectIdExt, Repository, Submodule};

use crate::OutputFormat;

pub fn list(
    repo: Repository,
    mut out: impl std::io::Write,
    format: OutputFormat,
    dirty_suffix: Option<String>,
) -> anyhow::Result<()> {
    if format != OutputFormat::Human {
        bail!("Only human output is supported for now")
    }

    let Some(submodules) = repo.submodules()? else {
        return Ok(());
    };
    for sm in submodules {
        print_sm(sm, dirty_suffix.as_deref(), &mut out)?;
    }
    Ok(())
}

fn print_sm(sm: Submodule<'_>, dirty_suffix: Option<&str>, out: &mut impl std::io::Write) -> anyhow::Result<()> {
    let _span = gix::trace::coarse!("print_sm", path = ?sm.path());
    let state = sm.state()?;
    let mut sm_repo = sm.open()?;
    if let Some(repo) = sm_repo.as_mut() {
        repo.object_cache_size_if_unset(4 * 1024 * 1024);
    }
    writeln!(
        out,
        " {is_active} {path:?} {config} head:{head_id} index:{index_id} ({worktree}) [{url}]",
        is_active = if !sm.is_active()? || !state.repository_exists {
            "ⅹ"
        } else {
            "✓"
        },
        path = sm.path()?,
        config = if state.superproject_configuration {
            "config:yes"
        } else {
            "config:no"
        },
        head_id = submodule_short_hash(sm.head_id()?, sm_repo.as_ref()),
        index_id = submodule_short_hash(sm.index_id()?, sm_repo.as_ref()),
        worktree = match sm_repo {
            Some(repo) => {
                // TODO(name-revision): this is the simple version, `git` gives it
                //                      multiple tries https://github.com/git/git/blob/fac96dfbb1c24369ba7d37a5affd8adfe6c650fd/builtin/submodule--helper.c#L161
                //                      and even uses `git name-rev`/`git describe --contains` which we can't do yet.
                repo.head_commit()?
                    .describe()
                    .names(SelectRef::AllRefs)
                    .id_as_fallback(true)
                    .try_resolve()?
                    .expect("resolution present if ID can be used as fallback")
                    .format_with_dirty_suffix(dirty_suffix.map(ToOwned::to_owned))?
                    .to_string()
            }
            None => {
                "no worktree".into()
            }
        },
        url = sm.url()?.to_bstring()
    )?;
    Ok(())
}

fn submodule_short_hash(id: Option<gix::ObjectId>, repo: Option<&Repository>) -> String {
    id.map_or_else(
        || "none".to_string(),
        |id| repo.map_or_else(|| id.to_string(), |repo| id.attach(repo).shorten_or_id().to_string()),
    )
}

/// Options for `gix submodule update`.
pub struct UpdateOptions {
    /// Initialize submodules before updating.
    pub init: bool,
    /// Recursively update nested submodules.
    pub recursive: bool,
}

/// Update submodules to the commit recorded in the superproject's index.
#[cfg(feature = "blocking-client")]
pub fn update(
    repo: Repository,
    mut out: impl std::io::Write,
    _progress: impl gix::NestedProgress + 'static,
    format: OutputFormat,
    options: UpdateOptions,
) -> anyhow::Result<()> {
    if format != OutputFormat::Human {
        bail!("Only human output is supported for now")
    }

    let Some(submodules) = repo.submodules()? else {
        writeln!(out, "No submodules configured")?;
        return Ok(());
    };

    let should_interrupt = std::sync::atomic::AtomicBool::new(false);
    let update_opts = gix::submodule::update::Options::new(options.init, options.recursive);

    for sm in submodules {
        let name = sm.name().to_owned();
        let path = sm.path()?.into_owned();
        match sm.update_submodule(gix::progress::Discard, &should_interrupt, &update_opts) {
            Ok(Some(outcome)) => {
                let action = if outcome.freshly_cloned {
                    "cloned and checked out"
                } else {
                    "checked out"
                };
                writeln!(out, "Submodule path '{path}': {action} '{}'", outcome.target_commit)?;
            }
            Ok(None) => {
                // Submodule was skipped (not initialized, no index entry, etc.)
            }
            Err(err) => {
                writeln!(out, "warning: failed to update submodule '{name}': {err}")?;
            }
        }
    }
    Ok(())
}
