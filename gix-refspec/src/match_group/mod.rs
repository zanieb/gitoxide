use std::collections::{BTreeSet, HashSet};

use bstr::BStr;

use crate::{instruction::Push, parse::Operation, types::Mode, MatchGroup, RefSpecRef};

pub(crate) mod types;
pub use types::{match_lhs, match_push, match_rhs, Item, Mapping, PushDeletion, PushUpdate, Source, SourceRef};

///
pub mod validate;

/// Initialization
impl<'a> MatchGroup<'a> {
    /// Take all the fetch ref specs from `specs` get a match group ready.
    pub fn from_fetch_specs(specs: impl IntoIterator<Item = RefSpecRef<'a>>) -> Self {
        MatchGroup {
            specs: specs.into_iter().filter(|s| s.op == Operation::Fetch).collect(),
        }
    }

    /// Take all the push ref specs from `specs` get a match group ready.
    pub fn from_push_specs(specs: impl IntoIterator<Item = RefSpecRef<'a>>) -> Self {
        MatchGroup {
            specs: specs.into_iter().filter(|s| s.op == Operation::Push).collect(),
        }
    }
}

/// Matching
impl<'spec> MatchGroup<'spec> {
    /// Match all `items` against all *fetch* specs present in this group, returning deduplicated mappings from source to destination.
    /// `items` are expected to be references on the remote, which will be matched and mapped to obtain their local counterparts,
    /// i.e. *left side of refspecs is mapped to their right side*.
    /// *Note that this method is correct only for fetch-specs*, even though it also *works for push-specs*.
    ///
    /// Object names are never mapped and always returned as match.
    ///
    /// Note that negative matches are not part of the return value, so they are not observable but will be used to remove mappings.
    // TODO: figure out how to deal with push-specs, probably when push is being implemented.
    pub fn match_lhs<'item>(
        self,
        mut items: impl Iterator<Item = Item<'item>> + Clone,
    ) -> match_lhs::Outcome<'spec, 'item> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::default();
        let mut push_unique = |mapping| {
            if seen.insert(calculate_hash(&mapping)) {
                out.push(mapping);
            }
        };
        let mut matchers: Vec<Option<Matcher<'_>>> = self
            .specs
            .iter()
            .copied()
            .map(Matcher::from)
            .enumerate()
            .map(|(idx, m)| match m.lhs {
                Some(Needle::Object(id)) => {
                    push_unique(Mapping {
                        item_index: None,
                        lhs: SourceRef::ObjectId(id),
                        rhs: m.rhs.map(Needle::to_bstr),
                        spec_index: idx,
                    });
                    None
                }
                _ => Some(m),
            })
            .collect();

        let mut has_negation = false;
        for (spec_index, (spec, matcher)) in self.specs.iter().zip(matchers.iter_mut()).enumerate() {
            if spec.mode == Mode::Negative {
                has_negation = true;
                continue;
            }
            for (item_index, item) in items.clone().enumerate() {
                let Some(matcher) = matcher else { continue };
                let (matched, rhs) = matcher.matches_lhs(item);
                if matched {
                    push_unique(Mapping {
                        item_index: Some(item_index),
                        lhs: SourceRef::FullName(item.full_ref_name.into()),
                        rhs,
                        spec_index,
                    });
                }
            }
        }

        if let Some(hash_kind) = has_negation.then(|| items.next().map(|i| i.target.kind())).flatten() {
            let null_id = hash_kind.null();
            for matcher in matchers
                .into_iter()
                .zip(self.specs.iter())
                .filter_map(|(m, spec)| m.and_then(|m| (spec.mode == Mode::Negative).then_some(m)))
            {
                out.retain(|m| match &m.lhs {
                    SourceRef::ObjectId(_) => true,
                    SourceRef::FullName(name) => {
                        !matcher
                            .matches_lhs(Item {
                                full_ref_name: name.as_ref(),
                                target: &null_id,
                                object: None,
                            })
                            .0
                    }
                });
            }
        }
        match_lhs::Outcome {
            group: self,
            mappings: out,
        }
    }

    /// Match all `items` against all *fetch* specs present in this group, returning deduplicated mappings from destination to source.
    /// `items` are expected to be tracking references in the local clone, which will be matched and reverse-mapped to obtain their remote counterparts,
    /// i.e. *right side of refspecs is mapped to their left side*.
    /// *Note that this method is correct only for fetch-specs*, even though it also *works for push-specs*.
    ///
    /// Note that negative matches are not part of the return value, so they are not observable but will be used to remove mappings.
    // Reverse-mapping is implemented here: https://github.com/git/git/blob/76cf4f61c87855ebf0784b88aaf737d6b09f504b/branch.c#L252
    pub fn match_rhs<'item>(
        self,
        mut items: impl Iterator<Item = Item<'item>> + Clone,
    ) -> match_rhs::Outcome<'spec, 'item> {
        let mut out = Vec::<Mapping<'spec, 'item>>::new();
        let mut seen = BTreeSet::default();
        let mut push_unique = |mapping| {
            if seen.insert(calculate_hash(&mapping)) {
                out.push(mapping);
            }
        };
        let mut matchers: Vec<Matcher<'_>> = self.specs.iter().copied().map(Matcher::from).collect();

        let mut has_negation = false;
        for (spec_index, (spec, matcher)) in self.specs.iter().zip(matchers.iter_mut()).enumerate() {
            if spec.mode == Mode::Negative {
                has_negation = true;
                continue;
            }
            for (item_index, item) in items.clone().enumerate() {
                let (matched, lhs) = matcher.matches_rhs(item);
                if let Some(lhs) = lhs.filter(|_| matched) {
                    push_unique(Mapping {
                        item_index: Some(item_index),
                        lhs: SourceRef::FullName(lhs),
                        rhs: Some(item.full_ref_name.into()),
                        spec_index,
                    });
                }
            }
        }

        if let Some(hash_kind) = has_negation.then(|| items.next().map(|i| i.target.kind())).flatten() {
            let null_id = hash_kind.null();
            for matcher in matchers
                .into_iter()
                .zip(self.specs.iter())
                .filter_map(|(m, spec)| (spec.mode == Mode::Negative).then_some(m))
            {
                out.retain(|m| match &m.lhs {
                    SourceRef::ObjectId(_) => true,
                    SourceRef::FullName(name) => {
                        !matcher
                            .matches_rhs(Item {
                                full_ref_name: name.as_ref(),
                                target: &null_id,
                                object: None,
                            })
                            .0
                    }
                });
            }
        }
        match_rhs::Outcome {
            group: self,
            mappings: out,
        }
    }

    /// Match all *push* specs in this group against local and remote references.
    ///
    /// `local_items` are the references available for use as push sources, while `remote_items` are advertised remote
    /// references. The remote side is used to discover already-existing destinations and to implement the special `:`
    /// all-matching-branches refspec.
    ///
    /// This method matches ref names and object ids. Source revspecs that are not object ids should be resolved by the
    /// caller before constructing the actual push command.
    pub fn match_push<'local, 'remote>(
        self,
        local_items: impl IntoIterator<Item = Item<'local>>,
        remote_items: impl IntoIterator<Item = Item<'remote>>,
    ) -> match_push::Outcome<'spec> {
        let local_items: Vec<_> = local_items.into_iter().collect();
        let remote_items: Vec<_> = remote_items.into_iter().collect();
        let remote_index = |name: &BStr| remote_items.iter().position(|item| item.full_ref_name == name);

        let mut updates = Vec::new();
        let mut deletions = Vec::new();
        let mut seen_updates = HashSet::default();
        let mut seen_deletions = HashSet::default();
        let mut negative_matchers = Vec::new();

        for (spec_index, spec) in self.specs.iter().copied().enumerate() {
            if spec.mode == Mode::Negative {
                negative_matchers.push(Matcher::from(spec));
                continue;
            }

            match spec.instruction() {
                crate::Instruction::Push(Push::Matching {
                    allow_non_fast_forward, ..
                }) => {
                    let matcher = Matcher::from(spec);
                    if let (Some(Needle::Object(id)), Some(dst)) = (matcher.lhs, matcher.rhs) {
                        push_unique_update(
                            PushUpdate {
                                local_item_index: None,
                                remote_item_index: remote_index(dst.to_bstr().as_ref()),
                                src: SourceRef::ObjectId(id),
                                dst: dst.to_bstr().into_owned(),
                                spec_index,
                                allow_non_fast_forward,
                            },
                            &mut seen_updates,
                            &mut updates,
                        );
                        continue;
                    }

                    for (local_item_index, item) in local_items.iter().copied().enumerate() {
                        let (matched, dst) = matcher.matches_lhs(item);
                        if matched {
                            let dst = dst.map_or_else(|| item.full_ref_name.to_owned(), std::borrow::Cow::into_owned);
                            push_unique_update(
                                PushUpdate {
                                    local_item_index: Some(local_item_index),
                                    remote_item_index: remote_index(dst.as_ref()),
                                    src: SourceRef::FullName(item.full_ref_name.to_owned().into()).into_owned(),
                                    dst,
                                    spec_index,
                                    allow_non_fast_forward,
                                },
                                &mut seen_updates,
                                &mut updates,
                            );
                        }
                    }
                }
                crate::Instruction::Push(Push::AllMatchingBranches { allow_non_fast_forward }) => {
                    for (local_item_index, local) in local_items.iter().copied().enumerate() {
                        if !is_branch(local.full_ref_name) {
                            continue;
                        }
                        if let Some(remote_item_index) = remote_index(local.full_ref_name) {
                            push_unique_update(
                                PushUpdate {
                                    local_item_index: Some(local_item_index),
                                    remote_item_index: Some(remote_item_index),
                                    src: SourceRef::FullName(local.full_ref_name.to_owned().into()).into_owned(),
                                    dst: local.full_ref_name.to_owned(),
                                    spec_index,
                                    allow_non_fast_forward,
                                },
                                &mut seen_updates,
                                &mut updates,
                            );
                        }
                    }
                }
                crate::Instruction::Push(Push::Delete { ref_or_pattern }) => {
                    let matcher = Matcher::from(spec);
                    let matched_remote_indices: Vec<_> = remote_items
                        .iter()
                        .copied()
                        .enumerate()
                        .filter_map(|(remote_item_index, item)| {
                            matcher
                                .matches_rhs(item)
                                .0
                                .then_some((remote_item_index, item.full_ref_name.to_owned()))
                        })
                        .collect();

                    if matched_remote_indices.is_empty() {
                        let dst = Needle::from(ref_or_pattern).to_bstr().into_owned();
                        push_unique_deletion(
                            PushDeletion {
                                remote_item_index: remote_index(dst.as_ref()),
                                dst,
                                spec_index,
                            },
                            &mut seen_deletions,
                            &mut deletions,
                        );
                    } else {
                        for (remote_item_index, dst) in matched_remote_indices {
                            push_unique_deletion(
                                PushDeletion {
                                    remote_item_index: Some(remote_item_index),
                                    dst,
                                    spec_index,
                                },
                                &mut seen_deletions,
                                &mut deletions,
                            );
                        }
                    }
                }
                crate::Instruction::Push(Push::Exclude { .. }) => unreachable!("handled as negative spec"),
                crate::Instruction::Fetch(_) => unreachable!("push groups contain only push specs"),
            }
        }

        if !negative_matchers.is_empty() {
            updates.retain(|update| match &update.src {
                SourceRef::ObjectId(_) => true,
                SourceRef::FullName(name) => {
                    let Some(local) = local_items
                        .iter()
                        .find(|item| item.full_ref_name == name.as_ref())
                        .copied()
                    else {
                        return true;
                    };
                    !negative_matchers.iter().any(|matcher| matcher.matches_lhs(local).0)
                }
            });

            let null_target = remote_items
                .first()
                .or_else(|| local_items.first())
                .map_or_else(|| gix_hash::Kind::shortest().null(), |item| item.target.kind().null());
            deletions.retain(|deletion| {
                let remote = deletion
                    .remote_item_index
                    .and_then(|idx| remote_items.get(idx))
                    .copied()
                    .unwrap_or(Item {
                        full_ref_name: deletion.dst.as_ref(),
                        target: &null_target,
                        object: None,
                    });
                !negative_matchers.iter().any(|matcher| matcher.matches_lhs(remote).0)
            });
        }

        match_push::Outcome {
            group: self,
            updates,
            deletions,
        }
    }
}

fn push_unique_update(item: PushUpdate, seen: &mut HashSet<(Source, bstr::BString)>, out: &mut Vec<PushUpdate>) {
    if seen.insert((item.src.clone(), item.dst.clone())) {
        out.push(item);
    }
}

fn push_unique_deletion(item: PushDeletion, seen: &mut HashSet<bstr::BString>, out: &mut Vec<PushDeletion>) {
    if seen.insert(item.dst.clone()) {
        out.push(item);
    }
}

fn calculate_hash<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::Hasher;
    let mut s = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

fn is_branch(name: &BStr) -> bool {
    name.starts_with(b"refs/heads/")
}

mod util;
use util::{Matcher, Needle};
