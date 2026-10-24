use std::{path::Path, sync::atomic::AtomicBool};

use gix::{
    NestedProgress, Result,
    bstr::{BStr, BString, ByteSlice},
    error::{OptionExt, ResultExt, bail, message},
    prelude::ObjectIdExt,
};
use unicode_width::UnicodeWidthStr;

use crate::OutputFormat;

pub struct AddOptions {
    pub new_branch: Option<BString>,
    pub commit_ish: Option<BString>,
    pub detach: bool,
    pub reset_branch: Option<BString>,
    pub lock: bool,
    pub no_checkout: bool,
    pub format: OutputFormat,
}

pub fn add<P>(
    repo: gix::Repository,
    destination: &Path,
    out: &mut dyn std::io::Write,
    progress: P,
    should_interrupt: &AtomicBool,
    options: AddOptions,
) -> Result<()>
where
    P: NestedProgress,
    P::SubProgress: NestedProgress + 'static,
{
    if options.format != OutputFormat::Human {
        bail!(gix::error::unsupported("JSON output isn't implemented yet"));
    }
    let created = repo
        .worktree_add_with_progress(
            destination,
            gix::worktree::add::Options {
                new_branch: options.new_branch.as_ref().map(|name| name.as_bstr()),
                reset_branch: options.reset_branch.as_ref().map(|name| name.as_bstr()),
                start_point: options.commit_ish.as_ref().map(|name| name.as_bstr()),
                detach: options.detach,
                lock: options.lock,
                no_checkout: options.no_checkout,
                ..Default::default()
            },
            progress,
            should_interrupt,
        )?
        .into_repo()?;
    let info = create_worktree_info(
        &created,
        gix::path::realpath(
            created
                .workdir()
                .ok_or_raise(|| message("The new worktree has no checkout directory"))?,
        )?,
    )?;
    info.write(out, UnicodeWidthStr::width(info.base.as_str())).or_error()?;
    Ok(())
}

pub fn remove<P>(repo: gix::Repository, worktree: &Path, force: u8, progress: P, format: OutputFormat) -> Result<()>
where
    P: NestedProgress,
    P::SubProgress: NestedProgress + 'static,
{
    use gix::worktree::remove::Force;

    if format != OutputFormat::Human {
        bail!(gix::error::unsupported("JSON output isn't implemented yet"));
    }
    repo.remove_worktree(
        worktree,
        match force {
            0 => Force::Never,
            1 => Force::DiscardChanges,
            _ => Force::OverrideLock,
        },
        progress,
    )?;
    Ok(())
}

pub fn list(repo: gix::Repository, out: &mut dyn std::io::Write, format: OutputFormat) -> Result<()> {
    if format != OutputFormat::Human {
        bail!(gix::error::unsupported("JSON output isn't implemented yet"));
    }
    let main_repo = repo.main_repo()?;
    let mut worktrees = Vec::new();

    if let Some(worktree) = main_repo.worktree() {
        worktrees.push(create_worktree_info(&main_repo, gix::path::realpath(worktree.base())?)?);
    }

    for proxy in main_repo.worktrees()? {
        let base = gix::path::realpath(proxy.base()?)?;

        match proxy.into_repo() {
            Ok(worktree_repo) => {
                worktrees.push(create_worktree_info(&worktree_repo, base)?);
            }
            Err(_) => {
                worktrees.push(create_inaccessible_worktree_info(&repo, base));
            }
        }
    }

    let path_width = worktrees
        .iter()
        .map(|worktree| UnicodeWidthStr::width(worktree.base.as_str()))
        .max()
        .unwrap_or(0);

    for worktree in worktrees {
        worktree.write(out, path_width).or_error()?;
    }

    Ok(())
}

struct WorktreeInfo {
    base: String,
    head: String,
    branch: String,
}

impl WorktreeInfo {
    fn write(&self, out: &mut dyn std::io::Write, path_width: usize) -> std::io::Result<()> {
        writeln!(
            out,
            "{}{} {} [{}]",
            self.base,
            " ".repeat(path_width.saturating_sub(UnicodeWidthStr::width(self.base.as_str()))),
            self.head,
            self.branch,
        )
    }
}

fn create_worktree_info(repo: &gix::Repository, base: std::path::PathBuf) -> Result<WorktreeInfo> {
    let head = repo
        .head_id()
        .map_or_else(
            |_| repo.object_hash().null().attach(repo).shorten_or_id(),
            |id| id.shorten_or_id(),
        )
        .to_string();

    let branch = repo.head_name()?.map_or_else(
        || "<detached>".to_string(),
        |name| name.shorten().to_owned().to_string(),
    );

    Ok(WorktreeInfo {
        base: base.display().to_string(),
        head,
        branch,
    })
}

fn create_inaccessible_worktree_info(repo: &gix::Repository, base: std::path::PathBuf) -> WorktreeInfo {
    WorktreeInfo {
        base: base.display().to_string(),
        head: repo.object_hash().null().attach(repo).shorten_or_id().to_string(),
        branch: "<unknown>".to_string(),
    }
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
) -> Result<()> {
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

    repo.worktree_lock(id, options).or_error()?;

    if let Some(reason) = &opts.reason {
        writeln!(err, "Locked worktree '{id}' with reason: {reason}").or_error()?;
    } else {
        writeln!(err, "Locked worktree '{id}'").or_error()?;
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
) -> Result<()> {
    repo.worktree_unlock(id).or_error()?;

    writeln!(err, "Unlocked worktree '{id}'").or_error()?;
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
) -> Result<()> {
    let options = gix::worktree::prune::Options { dry_run: opts.dry_run };

    let pruned = repo.worktree_prune(options).or_error()?;

    if pruned.is_empty() {
        writeln!(err, "Nothing to prune").or_error()?;
    } else {
        for entry in &pruned {
            if opts.dry_run {
                writeln!(out, "Would prune '{}': {}", entry.id, entry.reason).or_error()?;
            } else {
                writeln!(out, "Pruned '{}': {}", entry.id, entry.reason).or_error()?;
            }
        }
        if opts.dry_run {
            writeln!(err, "Would prune {} worktree(s)", pruned.len()).or_error()?;
        } else {
            writeln!(err, "Pruned {} worktree(s)", pruned.len()).or_error()?;
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
) -> Result<()> {
    let options = gix::worktree::r#move::Options { force: opts.force };

    repo.worktree_move_opts(id, &new_path, options).or_error()?;

    writeln!(err, "Moved worktree '{}' to '{}'", id, new_path.display()).or_error()?;
    let _ = out;
    Ok(())
}

/// Repair worktree links.
pub fn repair(
    repo: gix::Repository,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
    paths: Vec<std::path::PathBuf>,
) -> Result<()> {
    let repairs = repo.worktree_repair(&paths).or_error()?;

    if repairs.is_empty() {
        writeln!(err, "Nothing to repair").or_error()?;
    } else {
        for repair in &repairs {
            writeln!(out, "Repaired '{}': {}", repair.id, repair.kind).or_error()?;
        }
        writeln!(err, "Repaired {} worktree(s)", repairs.len()).or_error()?;
    }

    Ok(())
}
