use std::{collections::BTreeSet, sync::atomic::AtomicBool};

use crate::{
    bstr::{BStr, BString, ByteSlice, ByteVec},
    repository::add_to_index::{Error, Options, Outcome},
    Repository,
};

impl Repository {
    /// Add files from the worktree to the index, honoring pathspecs, attributes, filters and excludes.
    pub fn add_to_index(
        &self,
        pathspecs: impl IntoIterator<Item = impl AsRef<BStr>>,
        options: Options,
    ) -> Result<Outcome, Error> {
        let workdir = self.workdir().ok_or(Error::MissingWorktree)?;
        let pathspecs: Vec<BString> = pathspecs.into_iter().map(|p| p.as_ref().to_owned()).collect();
        let (mut filter, index) = self.filter_pipeline(None)?;
        let mut index = index.into_owned();
        index.set_path(self.index_path());

        let mut paths = BTreeSet::new();
        {
            let mut pathspec = self.pathspec(
                true,
                pathspecs.iter(),
                true,
                &index,
                crate::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
            )?;
            let matching_paths = pathspec
                .index_entries_with_paths(&index)
                .map(|entries| entries.map(|(path, _entry)| path.to_owned()).collect::<Vec<_>>())
                .unwrap_or_default();
            paths.extend(matching_paths);
        }

        let mut collect = gix_dir::walk::delegate::Collect::default();
        self.dirwalk(
            &index,
            pathspecs.iter(),
            &AtomicBool::default(),
            self.dirwalk_options()?
                .empty_patterns_match_prefix(true)
                .emit_tracked(true)
                .emit_ignored(Some(gix_dir::walk::EmissionMode::Matching))
                .emit_untracked(gix_dir::walk::EmissionMode::Matching),
            &mut collect,
        )?;

        let mut outcome = Outcome::default();
        for (entry, _collapsed_directory_status) in collect.into_entries_by_path() {
            match entry.status {
                gix_dir::entry::Status::Tracked | gix_dir::entry::Status::Untracked => {
                    paths.insert(entry.rela_path);
                }
                gix_dir::entry::Status::Ignored(_) => {
                    if options.force_ignored {
                        paths.insert(entry.rela_path.clone());
                    }
                    outcome.ignored_entries.push(entry.rela_path);
                }
                gix_dir::entry::Status::Pruned => {}
            }
        }
        if paths.is_empty() && !pathspecs.is_empty() {
            let ignored_pathspecs: Vec<_> = outcome
                .ignored_entries
                .iter()
                .filter(|ignored| pathspecs.iter().any(|spec| spec.as_bstr() == ignored.as_bstr()))
                .cloned()
                .collect();
            if !options.force_ignored && !ignored_pathspecs.is_empty() {
                return Err(Error::Ignored {
                    paths: ignored_pathspecs,
                });
            }
            if outcome.ignored_entries.is_empty() {
                return Err(Error::PathspecsDidNotMatch { pathspecs });
            }
        }

        for path in paths {
            let path = path.as_bstr();
            match filter.worktree_file_to_object(path, &index)? {
                Some((id, kind, _metadata)) => {
                    let worktree_path = workdir.join(gix_path::from_bstr(path));
                    let metadata = gix_index::fs::Metadata::from_path_no_follow(&worktree_path).map_err(|source| {
                        Error::Metadata {
                            path: worktree_path.clone(),
                            source,
                        }
                    })?;
                    let stat = gix_index::entry::Stat::from_fs(&metadata).map_err(|source| Error::Stat {
                        path: worktree_path,
                        source,
                    })?;
                    remove_conflicting_entries(&mut index, path);
                    index.add_entry(stat, id, gix_index::entry::Flags::empty(), kind.into(), path);
                    outcome.added_entries += 1;
                }
                None => {
                    if remove_conflicting_entries(&mut index, path) {
                        outcome.removed_entries += 1;
                    }
                }
            }
        }

        index.write(Default::default())?;
        Ok(outcome)
    }
}

fn remove_conflicting_entries(index: &mut gix_index::File, path: &BStr) -> bool {
    let mut path_with_slash = path.to_owned();
    path_with_slash.push_byte(b'/');
    let mut removed = false;
    index.remove_entries(|_, existing_path, _entry| {
        let should_remove = existing_path == path
            || existing_path.starts_with(path_with_slash.as_bstr())
            || is_parent_path(existing_path, path);
        removed |= should_remove;
        should_remove
    });
    removed
}

fn is_parent_path(parent: &BStr, path: &BStr) -> bool {
    path.len() > parent.len() && path.starts_with(parent) && path.get(parent.len()) == Some(&b'/')
}
