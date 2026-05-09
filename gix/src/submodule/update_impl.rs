//! Implementation of `Submodule::update_submodule()`, modelled after git's `update_submodule()` and
//! `run_update_procedure()` in `builtin/submodule--helper.c`.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use crate::{bstr::ByteSlice, Repository, Submodule};

/// Update operations
impl Submodule<'_> {
    /// Update this submodule to the commit recorded in the superproject's index.
    ///
    /// This performs the equivalent of `git submodule update [--init] [--recursive]`:
    ///
    /// 1. Optionally initialize the submodule (if `options.init` is set)
    /// 2. Clone the submodule repository if it doesn't exist, or fetch if the target commit is missing
    /// 3. Apply the configured update strategy
    /// 4. Optionally recurse into nested submodules
    ///
    /// Returns `Ok(None)` if the submodule has no commit recorded in the superproject index,
    /// if the submodule is not initialized (and `--init` was not passed), or if the update
    /// strategy is `none`.
    ///
    /// # Important behavioral notes (matching git)
    ///
    /// - **Just-cloned submodules always use checkout**, even if configured for rebase/merge/none.
    ///   This matches git's `determine_submodule_update_strategy()`.
    /// - The update strategy `none` causes this method to silently skip the submodule.
    /// - The `merge` and `rebase` strategies support already-up-to-date, fast-forward, and clean diverged updates.
    /// - The `!command` strategy runs the locally configured command with the target commit as its argument.
    pub fn update_submodule<P>(
        &self,
        mut progress: P,
        should_interrupt: &AtomicBool,
        options: &super::update::Options,
    ) -> Result<Option<super::update::Outcome>, super::update::Error>
    where
        P: gix_features::progress::NestedProgress + 'static,
        P::SubProgress: gix_features::progress::NestedProgress + 'static,
    {
        // Step 1: Optionally init
        if options.init {
            self.init(false)?;
        }

        // Step 2: Determine target commit from superproject index
        let target_commit = match self.index_id()? {
            Some(id) => id,
            None => return Ok(None),
        };

        // Step 3: Determine the update strategy.
        // Git's logic: local config > .gitmodules > default (checkout).
        let configured_strategy = {
            let config = &self.state.repo.config.resolved;
            // Check local config first (user overrides via `git config submodule.<name>.update`)
            let local_strategy = config
                .sections_by_name("submodule")
                .into_iter()
                .flatten()
                .filter(|s| {
                    s.header().subsection_name() == Some(self.name()) && s.meta().source == gix_config::Source::Local
                })
                .find_map(|s| s.value("update"))
                .and_then(|v| gix_submodule::config::Update::try_from(v.as_ref()).ok());
            local_strategy.unwrap_or_else(|| {
                self.state
                    .modules
                    .update(self.name())
                    .ok()
                    .flatten()
                    .unwrap_or_default()
            })
        };

        // Step 3b: Check update=none BEFORE doing any clone/fetch work.
        // Git silently skips submodules with update=none.
        if configured_strategy == gix_submodule::config::Update::None {
            return Ok(None);
        }

        // Step 4: Clone or fetch
        // Check if the submodule is initialized (has url in .git/config).
        // An uninitialized submodule (without --init) should not be updated.
        // Git checks is_submodule_active() which requires the url in .git/config.
        if !self.is_initialized() && !options.init {
            // The submodule is not initialized and --init was not passed.
            // We still have the URL from .gitmodules, but git skips uninitialized submodules.
            // Return None to indicate "nothing to do" for uninitialized submodules.
            return Ok(None);
        }

        let raw_url = self.url()?;
        let sm_state = self.state()?;
        let work_dir = self.work_dir()?;

        // Validate submodule path for symlink attacks before clone/checkout.
        if let Some(worktree) = self.state.repo.workdir() {
            let path_bstr = self.path()?;
            let sm_path = gix_path::from_bstr(path_bstr.as_ref());
            super::validate_submodule_path(&sm_path, worktree)?;
        }

        // Resolve the submodule URL to a usable clone source.
        //
        // For relative URLs: resolve against the superproject's origin remote URL,
        // falling back to resolving relative to the worktree (git falls back to getcwd()).
        //
        // For absolute file URLs: if the path doesn't exist (e.g. stale URL baked into
        // .git/config from a different machine), fall back to resolving the .gitmodules
        // relative URL against the worktree.
        let resolved_url = {
            let url_path = gix_path::from_bstr(std::borrow::Cow::Borrowed(raw_url.path.as_ref()));
            let cwd = self.state.repo.options.current_dir_or_empty();
            let worktree = self.state.repo.workdir().unwrap_or_else(|| self.state.repo.git_dir());

            if url_path.is_relative() && raw_url.scheme == gix_url::Scheme::File {
                // Relative URL: try origin remote first, then worktree.
                let from_remote = self
                    .state
                    .repo
                    .find_remote("origin")
                    .ok()
                    .and_then(|r| {
                        r.url(crate::remote::Direction::Fetch)
                            .map(std::borrow::ToOwned::to_owned)
                    })
                    .and_then(|base_url| {
                        let base_path = gix_path::from_bstr(std::borrow::Cow::Borrowed(base_url.path.as_ref()));
                        let candidate = base_path.join(&url_path);
                        gix_path::realpath_opts(&candidate, cwd, gix_path::realpath::MAX_SYMLINKS)
                            .ok()
                            .filter(|p| p.exists())
                    });

                let resolved = from_remote.unwrap_or_else(|| {
                    let candidate = worktree.join(&url_path);
                    gix_path::realpath_opts(&candidate, cwd, gix_path::realpath::MAX_SYMLINKS).unwrap_or(candidate)
                });
                resolved.to_string_lossy().into_owned()
            } else if raw_url.scheme == gix_url::Scheme::File && !url_path.exists() {
                // Absolute file URL that doesn't exist (stale path from another machine).
                // Fall back to the .gitmodules relative URL resolved against the worktree.
                let fallback = self.gitmodules_url().and_then(|gm_url| {
                    let gm_path = gix_path::from_bstr(std::borrow::Cow::Borrowed(gm_url.path.as_ref()));
                    if gm_path.is_relative() {
                        let candidate = worktree.join(&gm_path);
                        gix_path::realpath_opts(&candidate, cwd, gix_path::realpath::MAX_SYMLINKS)
                            .ok()
                            .filter(|p| p.exists())
                    } else {
                        None
                    }
                });
                match fallback {
                    Some(resolved) => resolved.to_string_lossy().into_owned(),
                    None => raw_url.to_bstring().to_string(),
                }
            } else {
                raw_url.to_bstring().to_string()
            }
        };

        let (sm_repo, freshly_cloned) = if !sm_state.repository_exists {
            // Clone the submodule using the standard clone infrastructure.
            let mut prep = crate::clone::PrepareFetch::new(
                resolved_url,
                &work_dir,
                crate::create::Kind::WithWorktree,
                Default::default(),
                self.state.repo.options.clone(),
            )?;

            let (mut checkout_prep, _fetch_outcome) = prep.fetch_then_checkout(&mut progress, should_interrupt)?;
            let (_repo, _checkout_outcome) =
                checkout_prep.main_worktree(gix_features::progress::Discard, should_interrupt)?;

            // Move the cloned .git/ directory into the superproject's modules directory
            // and replace it with a .git file, matching modern git's submodule layout.
            let target_git_dir = self.git_dir();
            super::git_dir_layout::connect_work_tree_and_git_dir(&work_dir, &target_git_dir)?;

            // Re-open the repo from the new layout location.
            let mut repo =
                crate::open_opts(target_git_dir, self.state.repo.options.clone()).map_err(super::open::Error::from)?;
            repo.set_workdir(Some(work_dir.clone()))
                .map_err(super::open::Error::from)?;

            (repo, true)
        } else {
            // Repository exists, open it
            let sm_repo = match self.open()? {
                Some(repo) => repo,
                None => {
                    // Repository directory exists but can't be opened
                    return Ok(None);
                }
            };

            // Ensure core.worktree is set for existing submodule repos.
            // This is a safety net for repos that may have been cloned without it.
            let git_dir = self.git_dir();
            if git_dir.is_dir() {
                let _ = super::git_dir_layout::ensure_core_worktree(&git_dir, &work_dir);
            }

            // Check if the target commit already exists in the submodule.
            // If not, fetch. This matches git's run_update_procedure():
            // "Run fetch only if oid isn't present or it is not reachable from a ref."
            if sm_repo.find_object(target_commit).is_err() {
                // Try to fetch from the origin remote
                if let Ok(remote) = sm_repo.find_remote("origin") {
                    let _outcome = remote
                        .connect(crate::remote::Direction::Fetch)?
                        .prepare_fetch(&mut progress, Default::default())?
                        .receive(&mut progress, should_interrupt)?;
                }
            }
            (sm_repo, false)
        };

        // Step 5: Determine effective strategy.
        // Git: just-cloned submodules always use checkout, even if configured for rebase/merge/none.
        let effective_strategy = if freshly_cloned {
            gix_submodule::config::Update::Checkout
        } else {
            configured_strategy
        };

        match &effective_strategy {
            gix_submodule::config::Update::None => {
                return Ok(None);
            }
            gix_submodule::config::Update::Command(_) => {}
            gix_submodule::config::Update::Rebase | gix_submodule::config::Update::Merge => {}
            gix_submodule::config::Update::Checkout => {}
        }

        // Step 6: Apply the effective strategy.
        let checkout_outcome = match &effective_strategy {
            gix_submodule::config::Update::Checkout => {
                // Skip checkout if HEAD already matches the target commit.
                if !freshly_cloned {
                    if let Ok(head_id) = sm_repo.head_id() {
                        if head_id.detach() == target_commit {
                            return Ok(Some(super::update::Outcome {
                                strategy: effective_strategy,
                                target_commit,
                                freshly_cloned: false,
                                checkout: None,
                            }));
                        }
                    }
                }
                Some(checkout_to_commit(&sm_repo, target_commit, should_interrupt)?)
            }
            gix_submodule::config::Update::Rebase | gix_submodule::config::Update::Merge => {
                update_current_branch_strategy(&sm_repo, target_commit, effective_strategy.clone(), should_interrupt)?
            }
            gix_submodule::config::Update::Command(command) => {
                run_custom_update_command(&sm_repo, command, target_commit)?;
                None
            }
            gix_submodule::config::Update::None => {
                unreachable!("none strategy is handled before applying the update")
            }
        };

        // Step 7: Recursive update
        if options.recursive {
            if let Some(submodules) = sm_repo.submodules()? {
                let sub_opts = super::update::Options {
                    init: options.init,
                    recursive: true,
                };
                for sm in submodules {
                    // With --init, update all submodules (git implicitly activates them).
                    // Without --init, only update active (initialized) ones.
                    let should_update = options.init || sm.is_active().unwrap_or(false);
                    if should_update {
                        // Best-effort recursive update; continue on error for individual submodules.
                        // This matches git which prints "Failed to recurse into submodule path"
                        // but continues with siblings.
                        let _ = sm.update_submodule(gix_features::progress::Discard, should_interrupt, &sub_opts);
                    }
                }
            }
        }

        Ok(Some(super::update::Outcome {
            strategy: effective_strategy,
            target_commit,
            freshly_cloned,
            checkout: checkout_outcome,
        }))
    }
}

#[cfg(feature = "revision")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ancestry {
    Same,
    HeadContainsTarget,
    TargetContainsHead,
    Diverged,
}

#[cfg(feature = "revision")]
fn ancestry(
    repo: &Repository,
    head: gix_hash::ObjectId,
    target: gix_hash::ObjectId,
) -> Result<Ancestry, super::update::Error> {
    if head == target {
        return Ok(Ancestry::Same);
    }

    let base = repo.merge_base(head, target)?.detach();
    Ok(if base == target {
        Ancestry::HeadContainsTarget
    } else if base == head {
        Ancestry::TargetContainsHead
    } else {
        Ancestry::Diverged
    })
}

fn update_current_branch_strategy(
    repo: &Repository,
    target_commit: gix_hash::ObjectId,
    strategy: gix_submodule::config::Update,
    should_interrupt: &AtomicBool,
) -> Result<Option<gix_worktree_state::checkout::Outcome>, super::update::Error> {
    #[cfg(not(feature = "revision"))]
    {
        let _ = (repo, target_commit, should_interrupt);
        return Err(super::update::Error::StrategyNeedsRevisionFeature { strategy });
    }

    #[cfg(feature = "revision")]
    {
        let head = repo.head_id()?.detach();
        match ancestry(repo, head, target_commit)? {
            Ancestry::Same | Ancestry::HeadContainsTarget => Ok(None),
            Ancestry::TargetContainsHead => {
                set_head_to_commit(
                    repo,
                    target_commit,
                    &format!("submodule update: {strategy:?} {target_commit}"),
                    false,
                )?;
                checkout_commit_tree(repo, target_commit, should_interrupt).map(Some)
            }
            Ancestry::Diverged => {
                update_diverged_current_branch_strategy(repo, head, target_commit, strategy, should_interrupt)
            }
        }
    }
}

#[cfg(all(feature = "revision", not(feature = "merge")))]
fn update_diverged_current_branch_strategy(
    _repo: &Repository,
    _head: gix_hash::ObjectId,
    _target_commit: gix_hash::ObjectId,
    strategy: gix_submodule::config::Update,
    _should_interrupt: &AtomicBool,
) -> Result<Option<gix_worktree_state::checkout::Outcome>, super::update::Error> {
    Err(super::update::Error::StrategyNeedsMergeFeature { strategy })
}

#[cfg(all(feature = "revision", feature = "merge"))]
fn update_diverged_current_branch_strategy(
    repo: &Repository,
    head: gix_hash::ObjectId,
    target_commit: gix_hash::ObjectId,
    strategy: gix_submodule::config::Update,
    should_interrupt: &AtomicBool,
) -> Result<Option<gix_worktree_state::checkout::Outcome>, super::update::Error> {
    match strategy {
        gix_submodule::config::Update::Merge => {
            merge_current_branch(repo, head, target_commit, should_interrupt).map(Some)
        }
        gix_submodule::config::Update::Rebase => {
            rebase_current_branch(repo, head, target_commit, should_interrupt).map(Some)
        }
        _ => unreachable!("only branch-preserving strategies require diverged handling"),
    }
}

#[cfg(all(feature = "revision", feature = "merge"))]
fn merge_current_branch(
    repo: &Repository,
    head: gix_hash::ObjectId,
    target_commit: gix_hash::ObjectId,
    should_interrupt: &AtomicBool,
) -> Result<gix_worktree_state::checkout::Outcome, super::update::Error> {
    let labels = gix_merge::blob::builtin_driver::text::Labels {
        ancestor: None,
        current: Some("HEAD".into()),
        other: Some("submodule".into()),
    };
    let mut outcome = repo.merge_commits(head, target_commit, labels, repo.tree_merge_options()?.into())?;
    if outcome
        .tree_merge
        .has_unresolved_conflicts(crate::merge::tree::TreatAsUnresolved::default())
    {
        return Err(super::update::Error::StrategyConflict {
            strategy: gix_submodule::config::Update::Merge,
            head,
            target: target_commit,
        });
    }

    let tree_id = outcome.tree_merge.tree.write()?.detach();
    let committer = committer_or_fallback(repo)?;
    let commit = gix_object::Commit {
        tree: tree_id,
        parents: smallvec::smallvec![head, target_commit],
        author: committer.clone(),
        committer,
        encoding: None,
        message: format!("Merge commit '{target_commit}' into submodule\n").into(),
        extra_headers: Default::default(),
    };
    let merge_commit = repo.write_object(&commit)?.detach();

    set_head_to_commit(
        repo,
        merge_commit,
        &format!("submodule update: merge {target_commit}"),
        false,
    )?;
    checkout_commit_tree(repo, merge_commit, should_interrupt)
}

#[cfg(all(feature = "revision", feature = "merge"))]
fn rebase_current_branch(
    repo: &Repository,
    head: gix_hash::ObjectId,
    target_commit: gix_hash::ObjectId,
    should_interrupt: &AtomicBool,
) -> Result<gix_worktree_state::checkout::Outcome, super::update::Error> {
    use crate::ext::ObjectIdExt;

    let mut commits = head
        .attach(repo)
        .ancestors()
        .with_hidden([target_commit])
        .all()?
        .map(|info| info.map(|info| info.id))
        .collect::<Result<Vec<_>, _>>()?;
    commits.reverse();

    for commit in &commits {
        let parent_count = repo.find_commit(*commit)?.parent_ids().count();
        if parent_count > 1 {
            return Err(super::update::Error::StrategyNeedsLinearHistory {
                strategy: gix_submodule::config::Update::Rebase,
                commit: *commit,
            });
        }
    }

    set_head_to_commit(
        repo,
        target_commit,
        &format!("submodule update: rebase start {target_commit}"),
        false,
    )?;
    let mut checkout = checkout_commit_tree(repo, target_commit, should_interrupt)?;

    for commit in commits {
        let rebased = replay_commit_onto_head(repo, commit, target_commit)?;
        checkout = checkout_commit_tree(repo, rebased, should_interrupt)?;
    }

    Ok(checkout)
}

#[cfg(all(feature = "revision", feature = "merge"))]
fn replay_commit_onto_head(
    repo: &Repository,
    commit_id: gix_hash::ObjectId,
    target_commit: gix_hash::ObjectId,
) -> Result<gix_hash::ObjectId, super::update::Error> {
    let head = repo.head_id()?.detach();
    let commit = repo.find_commit(commit_id)?;
    let parent_ids: Vec<_> = commit.parent_ids().map(crate::Id::detach).collect();
    if parent_ids.len() > 1 {
        return Err(super::update::Error::StrategyNeedsLinearHistory {
            strategy: gix_submodule::config::Update::Rebase,
            commit: commit_id,
        });
    }

    let parent_tree = match parent_ids.first().copied() {
        Some(parent) => repo.find_commit(parent)?.tree_id()?.detach(),
        None => gix_hash::ObjectId::empty_tree(repo.object_hash()),
    };
    let head_tree = repo.find_commit(head)?.tree_id()?.detach();
    let commit_tree = commit.tree_id()?.detach();

    let labels = gix_merge::blob::builtin_driver::text::Labels {
        ancestor: Some("parent-of-rebased".into()),
        current: Some("HEAD".into()),
        other: Some("rebased".into()),
    };
    let mut outcome = repo.merge_trees(parent_tree, head_tree, commit_tree, labels, repo.tree_merge_options()?)?;
    if outcome.has_unresolved_conflicts(crate::merge::tree::TreatAsUnresolved::default()) {
        return Err(super::update::Error::StrategyConflict {
            strategy: gix_submodule::config::Update::Rebase,
            head,
            target: target_commit,
        });
    }

    let tree = outcome.tree.write()?.detach();
    let author: gix_actor::Signature = commit.author()?.into();
    let committer = committer_or_fallback(repo)?;
    let rebased = gix_object::Commit {
        tree,
        parents: smallvec::smallvec![head],
        author,
        committer,
        encoding: None,
        message: commit.message_raw_sloppy().to_vec().into(),
        extra_headers: Default::default(),
    };
    let rebased = repo.write_object(&rebased)?.detach();
    set_head_to_commit(repo, rebased, &format!("submodule update: rebase {commit_id}"), false)?;
    Ok(rebased)
}

#[cfg(all(feature = "revision", feature = "merge"))]
fn committer_or_fallback(repo: &Repository) -> Result<gix_actor::Signature, super::update::Error> {
    Ok(match repo.committer().and_then(std::result::Result::ok) {
        Some(committer) => committer.into(),
        None => gix_actor::Signature {
            name: b"gitoxide".as_bstr().to_owned(),
            email: b"gitoxide@localhost".as_bstr().to_owned(),
            time: gix_date::Time::now_local_or_utc(),
        },
    })
}

fn run_custom_update_command(
    repo: &Repository,
    command: &crate::bstr::BString,
    target_commit: gix_hash::ObjectId,
) -> Result<(), super::update::Error> {
    let command_path = gix_path::from_bstring(command.clone());
    let mut cmd: std::process::Command = gix_command::prepare(command_path.into_os_string())
        .command_may_be_shell_script()
        .with_context(repo.command_context()?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .arg(target_commit.to_string())
        .into();

    if let Some(workdir) = repo.workdir() {
        cmd.current_dir(workdir);
    } else {
        cmd.current_dir(repo.git_dir());
    }

    let status = cmd.status().map_err(|err| super::update::Error::CommandSpawn {
        command: command.clone(),
        source: err,
    })?;
    if !status.success() {
        return Err(super::update::Error::CommandFailed {
            command: command.clone(),
            status,
        });
    }

    Ok(())
}

/// Checkout the given `commit_id` in the submodule repository by detaching HEAD and checking out the tree.
///
/// This mirrors git's `run_update_command()` for `SM_UPDATE_CHECKOUT`, which runs
/// `git checkout -q <oid>` in the submodule directory.
fn checkout_to_commit(
    repo: &Repository,
    commit_id: gix_hash::ObjectId,
    should_interrupt: &AtomicBool,
) -> Result<gix_worktree_state::checkout::Outcome, super::update::Error> {
    // Set HEAD to the target commit (detached).
    set_head_to_commit(
        repo,
        commit_id,
        &format!("submodule update: checkout {commit_id}"),
        true,
    )?;

    checkout_commit_tree(repo, commit_id, should_interrupt)
}

fn set_head_to_commit(
    repo: &Repository,
    commit_id: gix_hash::ObjectId,
    reflog_message: &str,
    detach: bool,
) -> Result<(), super::update::Error> {
    use gix_ref::transaction::{Change, LogChange, RefEdit};
    // Get committer, falling back to a generic identity if not configured.
    let fallback_time_str = {
        let t = gix_date::Time::now_local_or_utc();
        format!("{} {}", t.seconds, t.offset)
    };
    let fallback = gix_actor::SignatureRef {
        name: b"gitoxide".as_bstr(),
        email: b"gitoxide@localhost".as_bstr(),
        time: &fallback_time_str,
    };
    let committer = repo.committer().and_then(std::result::Result::ok).unwrap_or(fallback);

    let ref_name = if detach {
        "HEAD".try_into().expect("valid ref name")
    } else {
        match repo.head()?.kind {
            crate::head::Kind::Symbolic(reference) => reference.name,
            crate::head::Kind::Unborn(name) => name,
            crate::head::Kind::Detached { .. } => "HEAD".try_into().expect("valid ref name"),
        }
    };

    repo.edit_references_as(
        Some(RefEdit {
            change: Change::Update {
                log: LogChange {
                    mode: gix_ref::transaction::RefLog::AndReference,
                    force_create_reflog: false,
                    message: reflog_message.into(),
                },
                expected: gix_ref::transaction::PreviousValue::Any,
                new: gix_ref::Target::Object(commit_id),
            },
            name: ref_name,
            deref: false,
        }),
        Some(committer),
    )?;
    Ok(())
}

fn checkout_commit_tree(
    repo: &Repository,
    commit_id: gix_hash::ObjectId,
    should_interrupt: &AtomicBool,
) -> Result<gix_worktree_state::checkout::Outcome, super::update::Error> {
    let workdir = repo.workdir().ok_or(super::update::Error::MissingWorkdir)?;

    // Read the old index from disk (if it exists) so we can detect removed files.
    // For freshly cloned repos the index won't exist yet, which is fine — we'll get an empty set.
    let old_entry_paths: HashSet<crate::bstr::BString> = gix_index::File::at(
        repo.index_path(),
        repo.object_hash(),
        true, // skip_hash for performance
        Default::default(),
    )
    .ok()
    .map(|old_idx| {
        let backing = old_idx.path_backing();
        old_idx
            .entries()
            .iter()
            .map(|e| e.path_in(backing).to_owned())
            .collect()
    })
    .unwrap_or_default();

    // Build index from the target tree
    let tree_id = repo.find_object(commit_id)?.peel_to_tree()?.id;

    let index =
        gix_index::State::from_tree(&tree_id, &repo.objects, repo.config.protect_options()?).map_err(|err| {
            super::update::Error::IndexFromTree {
                id: tree_id,
                source: err,
            }
        })?;
    let mut index = gix_index::File::from_state(index, repo.index_path());

    let mut opts = repo.checkout_options(gix_worktree::stack::state::attributes::Source::IdMapping)?;
    // The destination may have files from a previous checkout; allow overwriting.
    opts.destination_is_initially_empty = false;
    opts.overwrite_existing = true;

    let outcome = gix_worktree_state::checkout(
        &mut index,
        workdir,
        repo.objects
            .clone()
            .into_arc()
            .map_err(super::update::Error::OpenArcOdb)?,
        &gix_features::progress::Discard,
        &gix_features::progress::Discard,
        should_interrupt,
        opts,
    )?;

    // Delete files that existed in the old index but are absent from the new tree.
    if !old_entry_paths.is_empty() {
        let new_entry_paths: HashSet<crate::bstr::BString> = {
            let backing = index.path_backing();
            index.entries().iter().map(|e| e.path_in(backing).to_owned()).collect()
        };
        for removed_path in old_entry_paths.difference(&new_entry_paths) {
            let full_path = workdir.join(gix_path::from_bstr(removed_path));
            let _ = std::fs::remove_file(&full_path);
            // Try to remove empty parent directories up to the workdir root.
            let mut parent = full_path.parent();
            while let Some(dir) = parent {
                if dir == workdir {
                    break;
                }
                if std::fs::remove_dir(dir).is_err() {
                    break;
                }
                parent = dir.parent();
            }
        }
    }

    index.write(Default::default())?;
    Ok(outcome)
}
