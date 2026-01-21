use anyhow::bail;
use gix::bstr::BStr;

use crate::OutputFormat;

pub fn list(repo: gix::Repository, out: &mut dyn std::io::Write, format: OutputFormat) -> anyhow::Result<()> {
    if format != OutputFormat::Human {
        bail!("JSON output isn't implemented yet");
    }

    if let Some(worktree) = repo.worktree() {
        let locked_info = if worktree.is_locked() {
            let reason = worktree
                .lock_reason()
                .map(|r| format!(" ({})", r))
                .unwrap_or_default();
            format!(" locked{reason}")
        } else {
            String::new()
        };
        writeln!(
            out,
            "{base} [{branch}]{locked_info}",
            base = gix::path::realpath(worktree.base())?.display(),
            branch = repo
                .head_name()?
                .map_or("<detached>".into(), |name| name.shorten().to_owned()),
        )?;
    }
    for proxy in repo.worktrees()? {
        let locked_info = if proxy.is_locked() {
            let reason = proxy
                .lock_reason()
                .map(|r| format!(" ({})", r))
                .unwrap_or_default();
            format!(" locked{reason}")
        } else {
            String::new()
        };
        let base_display = proxy
            .base()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "<missing>".to_string());
        writeln!(
            out,
            "{base_display} [{name}]{locked_info}",
            name = proxy.id()
        )?;
    }
    Ok(())
}

/// Options for removing a worktree.
pub struct RemoveOptions {
    /// Force level for removal:
    /// - 0: Fail if dirty or locked
    /// - 1: Allow dirty, fail if locked
    /// - 2: Allow both dirty and locked
    pub force: u8,
}

/// Remove a worktree by its id.
pub fn remove(
    repo: gix::Repository,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
    id: &BStr,
    opts: RemoveOptions,
) -> anyhow::Result<()> {
    let options = gix::worktree::remove::Options { force: opts.force };

    repo.worktree_remove(id, options)?;

    writeln!(err, "Removed worktree '{}'", id)?;
    let _ = out; // currently unused, but available for future output
    Ok(())
}

/// Options for locking a worktree.
pub struct LockOptions {
    /// An optional reason for locking.
    pub reason: Option<String>,
}

/// Lock a worktree by its id.
pub fn lock(
    repo: gix::Repository,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
    id: &BStr,
    opts: LockOptions,
) -> anyhow::Result<()> {
    use gix::bstr::ByteSlice;

    let reason_bytes;
    let options = gix::worktree::lock::Options {
        reason: match &opts.reason {
            Some(r) => {
                reason_bytes = r.as_bytes();
                Some(reason_bytes.as_bstr())
            }
            None => None,
        },
    };

    repo.worktree_lock(id, options)?;

    if let Some(reason) = &opts.reason {
        writeln!(err, "Locked worktree '{}' with reason: {}", id, reason)?;
    } else {
        writeln!(err, "Locked worktree '{}'", id)?;
    }
    let _ = out;
    Ok(())
}

/// Unlock a worktree by its id.
pub fn unlock(
    repo: gix::Repository,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
    id: &BStr,
) -> anyhow::Result<()> {
    repo.worktree_unlock(id)?;

    writeln!(err, "Unlocked worktree '{}'", id)?;
    let _ = out;
    Ok(())
}

/// Options for pruning worktrees.
pub struct PruneOptions {
    /// Only report what would be pruned, don't actually delete.
    pub dry_run: bool,
}

/// Prune stale worktree entries.
pub fn prune(
    repo: gix::Repository,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
    opts: PruneOptions,
) -> anyhow::Result<()> {
    let options = gix::worktree::prune::Options {
        dry_run: opts.dry_run,
    };

    let pruned = repo.worktree_prune(options)?;

    if pruned.is_empty() {
        writeln!(err, "Nothing to prune")?;
    } else {
        for entry in &pruned {
            if opts.dry_run {
                writeln!(out, "Would prune '{}': {}", entry.id, entry.reason)?;
            } else {
                writeln!(out, "Pruned '{}': {}", entry.id, entry.reason)?;
            }
        }
        if opts.dry_run {
            writeln!(err, "Would prune {} worktree(s)", pruned.len())?;
        } else {
            writeln!(err, "Pruned {} worktree(s)", pruned.len())?;
        }
    }

    Ok(())
}

/// Options for moving a worktree.
pub struct MoveOptions {
    /// Force level for moving:
    /// - 0: Fail if locked
    /// - 2: Allow moving locked worktrees
    pub force: u8,
}

/// Move a worktree to a new location.
pub fn move_worktree(
    repo: gix::Repository,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
    id: &BStr,
    new_path: std::path::PathBuf,
    opts: MoveOptions,
) -> anyhow::Result<()> {
    let options = gix::worktree::r#move::Options { force: opts.force };

    repo.worktree_move_opts(id, &new_path, options)?;

    writeln!(err, "Moved worktree '{}' to '{}'", id, new_path.display())?;
    let _ = out;
    Ok(())
}

/// Repair worktree links.
pub fn repair(
    repo: gix::Repository,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
    paths: Vec<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let repairs = repo.worktree_repair(&paths)?;

    if repairs.is_empty() {
        writeln!(err, "Nothing to repair")?;
    } else {
        for repair in &repairs {
            writeln!(out, "Repaired '{}': {}", repair.id, repair.kind)?;
        }
        writeln!(err, "Repaired {} worktree(s)", repairs.len())?;
    }

    Ok(())
}

/// Options for adding a worktree.
#[derive(Default)]
pub struct AddOptions {
    /// The name of an existing branch to check out.
    pub branch: Option<String>,
    /// Create a new branch with this name.
    pub new_branch: Option<String>,
    /// Detach HEAD at the given commit-ish.
    pub detach: bool,
    /// Lock the worktree after creation.
    pub lock: bool,
    /// Skip the checkout step.
    pub no_checkout: bool,
}

/// Add a new worktree at the given path.
pub fn add(
    repo: gix::Repository,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
    path: std::path::PathBuf,
    commit_ish: Option<String>,
    opts: AddOptions,
) -> anyhow::Result<()> {
    use gix::bstr::ByteSlice;

    let branch_bytes;
    let new_branch_bytes;
    let start_point_bytes;

    let options = gix::worktree::add::Options {
        branch: match &opts.branch {
            Some(b) => {
                branch_bytes = b.as_bytes();
                Some(branch_bytes.as_bstr())
            }
            None => None,
        },
        new_branch: match &opts.new_branch {
            Some(b) => {
                new_branch_bytes = b.as_bytes();
                Some(new_branch_bytes.as_bstr())
            }
            None => None,
        },
        start_point: match &commit_ish {
            Some(s) => {
                start_point_bytes = s.as_bytes();
                Some(start_point_bytes.as_bstr())
            }
            None => None,
        },
        detach: opts.detach,
        lock: opts.lock,
        lock_reason: None,
        no_checkout: opts.no_checkout,
    };

    let proxy = repo.worktree_add(&path, options)?;

    writeln!(
        err,
        "Preparing worktree ({})",
        if opts.detach {
            "detached HEAD".to_string()
        } else if let Some(ref new_branch) = opts.new_branch {
            format!("new branch '{new_branch}'")
        } else if let Some(ref branch) = opts.branch {
            format!("checking out '{branch}'")
        } else {
            "checking out HEAD".to_string()
        }
    )?;

    let base = proxy.base()?;
    writeln!(out, "{}", base.display())?;
    Ok(())
}
