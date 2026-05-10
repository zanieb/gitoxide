use std::sync::atomic::AtomicBool;

use crate::{
    bstr::{BString, ByteSlice, ByteVec},
    ext::ObjectIdExt,
};
use gix_object::Exists as _;
use gix_object::Write as _;
use gix_refspec::match_group::SourceRef;
#[cfg(feature = "async-network-client")]
use gix_transport::client::async_io::Transport;
#[cfg(feature = "blocking-network-client")]
use gix_transport::client::blocking_io::Transport;

use super::{Error, Outcome, PreparePush};

impl<T> PreparePush<'_, '_, T>
where
    T: Transport,
{
    /// Execute the push, sending all necessary objects to the remote and updating refs.
    ///
    /// Returns `Ok(Outcome)` with per-reference status information.
    #[gix_protocol::maybe_async::maybe_async]
    pub async fn send<P>(mut self, progress: P, should_interrupt: &AtomicBool) -> Result<Outcome, Error>
    where
        P: gix_features::progress::NestedProgress,
        P::SubProgress: 'static,
    {
        let mut con = self.con.take().expect("send() can only be called once");
        let handshake = con.handshake.take().expect("send() can only be called once");
        let repo = con.remote.repo;

        let expected_object_hash = repo.object_hash();
        if self.ref_map.object_hash != expected_object_hash {
            return Err(Error::IncompatibleObjectHash {
                local: expected_object_hash,
                remote: self.ref_map.object_hash,
            });
        }

        let (commands, client_updates) = build_push_commands(&self.ref_map, repo, self.expected_old_ids.as_ref())?;

        // If we have no commands AND no client-side updates, check if all refspecs
        // were no-ops (refs already at target). If so, return empty success while
        // preserving the existing `NoMapping` error for matching pushes into an
        // empty remote advertisement.
        if commands.is_empty() && client_updates.is_empty() {
            let has_refspecs = !self.ref_map.refspecs.is_empty() || !self.ref_map.extra_refspecs.is_empty();
            if !has_refspecs || self.ref_map.remote_refs.is_empty() {
                return Err(Error::NoMapping {
                    refspecs: self.ref_map.refspecs.clone(),
                    num_remote_refs: self.ref_map.remote_refs.len(),
                });
            }
            // All refspecs were no-ops — return empty success.
            return Ok(Outcome {
                ref_map: std::mem::take(&mut self.ref_map),
                handshake,
                updates: Vec::new(),
                unpack_ok: true,
            });
        }

        // For dry-run, don't actually send anything to the server.
        if self.dry_run {
            let mut updates: Vec<_> = commands
                .iter()
                .map(|cmd| gix_protocol::push::response::StatusV1::Ok {
                    ref_name: cmd.ref_name.clone(),
                })
                .collect();
            updates.extend(client_updates);
            return Ok(Outcome {
                ref_map: std::mem::take(&mut self.ref_map),
                handshake,
                updates,
                unpack_ok: true,
            });
        }

        // If all commands were handled client-side, return early.
        if commands.is_empty() && !client_updates.is_empty() {
            return Ok(Outcome {
                ref_map: std::mem::take(&mut self.ref_map),
                handshake,
                updates: client_updates,
                unpack_ok: true,
            });
        }

        let protocol_commands: Vec<gix_protocol::push::Command> = commands
            .iter()
            .map(|cmd| gix_protocol::push::Command::new(cmd.ref_name.clone(), cmd.old_id, cmd.new_id))
            .collect();

        let mut new_tips = Vec::new();
        let mut known_remote = Vec::new();

        for cmd in &commands {
            if !cmd.new_id.is_null() {
                new_tips.push(cmd.new_id);
            }
            if !cmd.old_id.is_null() {
                known_remote.push(cmd.old_id);
            }
        }
        for remote_ref in &self.ref_map.remote_refs {
            use gix_protocol::handshake::Ref;
            match remote_ref {
                Ref::Direct { object, .. } | Ref::Symbolic { object, .. } => {
                    known_remote.push(*object);
                }
                Ref::Peeled { tag, object, .. } => {
                    known_remote.push(*tag);
                    known_remote.push(*object);
                }
                Ref::Unborn { .. } => {}
            }
        }

        let options = gix_protocol::push::Options {
            dry_run: false,
            atomic: self.atomic,
        };

        // Ensure the empty tree object exists in the ODB. Some backends (like jj)
        // create commits that reference the empty tree without physically storing it.
        // The pack generator needs to be able to find all referenced objects.
        {
            let empty_tree_id = gix_hash::ObjectId::empty_tree(expected_object_hash);
            if !repo.objects.exists(&empty_tree_id) {
                repo.objects
                    .write_buf(gix_object::Kind::Tree, &[])
                    .map_err(Error::PackGeneration)?;
            }
        }

        // Get a thread-safe inner handle which implements gix_pack::Find (needed for pack generation).
        // `prevent_pack_unload()` is required so that pack IDs remain stable during pack creation.
        let mut odb_for_pack = repo
            .objects
            .clone()
            .into_arc()
            .map_err(|err| Error::PackGeneration(Box::new(err)))?
            .into_inner();
        odb_for_pack.prevent_pack_unload();

        let result = gix_protocol::push(
            &protocol_commands,
            |writer, progress, should_interrupt| -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
                if new_tips.is_empty() {
                    return Ok(false);
                }
                write_pack_for_push(
                    &odb_for_pack,
                    &new_tips,
                    &known_remote,
                    expected_object_hash,
                    writer,
                    progress,
                    should_interrupt,
                )?;
                Ok(true)
            },
            progress,
            should_interrupt,
            &handshake,
            &mut con.transport.inner,
            repo.config.user_agent_tuple(),
            con.trace,
            options,
        )
        .await?;

        if matches!(handshake.server_protocol_version, gix_protocol::transport::Protocol::V2) {
            gix_protocol::indicate_end_of_interaction(&mut con.transport.inner, con.trace)
                .await
                .ok();
        }

        let mut updates = result.ref_updates;
        updates.extend(client_updates);
        Ok(Outcome {
            ref_map: std::mem::take(&mut self.ref_map),
            handshake,
            updates,
            unpack_ok: result.unpack_status.is_ok(),
        })
    }
}

struct PushCommand {
    ref_name: BString,
    old_id: gix_hash::ObjectId,
    new_id: gix_hash::ObjectId,
}

struct LocalRef {
    name: BString,
    target: gix_hash::ObjectId,
    object: Option<gix_hash::ObjectId>,
}

impl LocalRef {
    fn to_item(&self) -> gix_refspec::match_group::Item<'_> {
        gix_refspec::match_group::Item {
            full_ref_name: self.name.as_ref(),
            target: &self.target,
            object: self.object.as_ref().map(AsRef::as_ref),
        }
    }
}

struct SupplementalUpdate {
    src: BString,
    dst: BString,
    new_id: gix_hash::ObjectId,
    allow_non_fast_forward: bool,
}

fn build_push_commands(
    ref_map: &gix_protocol::fetch::RefMap,
    repo: &crate::Repository,
    expected_old_ids: Option<&std::collections::HashMap<crate::bstr::BString, gix_hash::ObjectId>>,
) -> Result<(Vec<PushCommand>, Vec<gix_protocol::push::response::StatusV1>), Error> {
    let object_hash = repo.object_hash();

    // Build a lookup from remote ref name (as bytes) to oid for finding old_id values.
    // When updating an existing remote ref, the old_id must be the ref's current value
    // so the server can verify the update is valid (compare-and-swap semantics).
    let mut remote_ref_by_name: std::collections::HashMap<&[u8], Option<gix_hash::ObjectId>> =
        std::collections::HashMap::new();
    for remote_ref in &ref_map.remote_refs {
        use gix_protocol::handshake::Ref;
        match remote_ref {
            Ref::Direct { full_ref_name, object } => {
                remote_ref_by_name.insert(full_ref_name.as_bytes(), Some(*object));
            }
            Ref::Symbolic {
                full_ref_name, object, ..
            } => {
                remote_ref_by_name.insert(full_ref_name.as_bytes(), Some(*object));
            }
            Ref::Peeled { full_ref_name, tag, .. } => {
                remote_ref_by_name.insert(full_ref_name.as_bytes(), Some(*tag));
            }
            Ref::Unborn { full_ref_name, .. } => {
                remote_ref_by_name.insert(full_ref_name.as_bytes(), None);
            }
        }
    }

    let mut commands = Vec::new();
    let mut client_updates = Vec::new();
    let mut source_by_destination = std::collections::HashMap::new();

    let local_refs = local_refs_for_push(repo)?;
    let null = gix_hash::ObjectId::null(object_hash);
    let specs: Vec<_> = ref_map
        .refspecs
        .iter()
        .chain(ref_map.extra_refspecs.iter())
        .map(gix_refspec::RefSpec::to_ref)
        .collect();
    let matched = gix_refspec::MatchGroup::from_push_specs(specs.iter().copied()).match_push(
        local_refs.iter().map(LocalRef::to_item),
        ref_map.remote_refs.iter().map(|r| remote_ref_to_item(r, &null)),
    );

    let mut supplemental_updates = Vec::new();
    for (spec_index, spec) in specs.iter().enumerate() {
        if let gix_refspec::Instruction::Push(gix_refspec::instruction::Push::Matching {
            src,
            dst,
            allow_non_fast_forward,
        }) = spec.instruction()
        {
            if matched.updates.iter().any(|update| update.spec_index == spec_index) {
                continue;
            }
            if gix_hash::ObjectId::from_hex(src.as_bytes()).is_ok() {
                continue;
            }
            if matches!(repo.try_find_reference(src), Ok(Some(_))) {
                continue;
            }

            let new_id = repo.rev_parse_single(src).map_err(|e| Error::FindLocalRef {
                name: src.to_owned(),
                source: Box::new(e),
            })?;
            supplemental_updates.push(SupplementalUpdate {
                src: src.to_owned(),
                dst: expand_push_destination(dst),
                new_id: new_id.detach(),
                allow_non_fast_forward,
            });
        }
    }

    for update in matched.updates {
        let source_label = push_source_label(&update.src);
        let new_id = match update.src {
            SourceRef::ObjectId(oid) => {
                if !repo.has_object(oid) {
                    return Err(Error::FindObject {
                        oid,
                        source: Box::new(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("object {oid} not found in local repository"),
                        )),
                    });
                }
                oid
            }
            SourceRef::FullName(name) => update
                .local_item_index
                .and_then(|idx| local_refs.get(idx))
                .map(|item| item.target)
                .ok_or_else(|| Error::FindLocalRef {
                    name: name.into_owned(),
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "matched local reference was not available",
                    )),
                })?,
        };

        record_push_destination(&update.dst, source_label, &mut source_by_destination)?;

        let remote_old_id = remote_ref_by_name
            .get(update.dst.as_bytes())
            .and_then(|oid| *oid)
            .unwrap_or_else(|| gix_hash::ObjectId::null(object_hash));

        if remote_old_id == new_id {
            continue;
        }

        let expected_old_id = expected_old_ids.and_then(|ids| ids.get(&update.dst));
        if let Some(reason) = client_side_push_rejection(
            repo,
            update.dst.as_ref(),
            remote_old_id,
            new_id,
            update.allow_non_fast_forward || expected_old_id.is_some(),
        ) {
            client_updates.push(gix_protocol::push::response::StatusV1::Ng {
                ref_name: update.dst,
                reason: reason.into(),
            });
            continue;
        }

        if let Some(expected_oid) = expected_old_id {
            if remote_old_id != *expected_oid {
                client_updates.push(gix_protocol::push::response::StatusV1::Ng {
                    ref_name: update.dst,
                    reason: "stale info".into(),
                });
                continue;
            }
        }

        commands.push(PushCommand {
            ref_name: update.dst,
            old_id: remote_old_id,
            new_id,
        });
    }

    for update in supplemental_updates {
        record_push_destination(&update.dst, update.src, &mut source_by_destination)?;

        let remote_old_id = remote_ref_by_name
            .get(update.dst.as_bytes())
            .and_then(|oid| *oid)
            .unwrap_or_else(|| gix_hash::ObjectId::null(object_hash));

        if remote_old_id == update.new_id {
            continue;
        }

        let expected_old_id = expected_old_ids.and_then(|ids| ids.get(&update.dst));
        if let Some(reason) = client_side_push_rejection(
            repo,
            update.dst.as_ref(),
            remote_old_id,
            update.new_id,
            update.allow_non_fast_forward || expected_old_id.is_some(),
        ) {
            client_updates.push(gix_protocol::push::response::StatusV1::Ng {
                ref_name: update.dst,
                reason: reason.into(),
            });
            continue;
        }

        if let Some(expected_oid) = expected_old_id {
            if remote_old_id != *expected_oid {
                client_updates.push(gix_protocol::push::response::StatusV1::Ng {
                    ref_name: update.dst,
                    reason: "stale info".into(),
                });
                continue;
            }
        }

        commands.push(PushCommand {
            ref_name: update.dst,
            old_id: remote_old_id,
            new_id: update.new_id,
        });
    }

    for deletion in matched.deletions {
        record_push_destination(&deletion.dst, BString::from("<delete>"), &mut source_by_destination)?;

        let remote_old_id = remote_ref_by_name.get(deletion.dst.as_bytes()).and_then(|oid| *oid);

        if let Some(expected_ids) = expected_old_ids {
            if let Some(expected_oid) = expected_ids.get(&deletion.dst) {
                let actual = remote_old_id.unwrap_or_else(|| gix_hash::ObjectId::null(object_hash));
                if actual != *expected_oid {
                    client_updates.push(gix_protocol::push::response::StatusV1::Ng {
                        ref_name: deletion.dst,
                        reason: "stale info".into(),
                    });
                    continue;
                }
            }
        }

        if let Some(old_id) = remote_old_id {
            commands.push(PushCommand {
                ref_name: deletion.dst,
                old_id,
                new_id: gix_hash::ObjectId::null(object_hash),
            });
        } else {
            client_updates.push(gix_protocol::push::response::StatusV1::Ok { ref_name: deletion.dst });
        }
    }

    Ok((commands, client_updates))
}

fn client_side_push_rejection(
    repo: &crate::Repository,
    dst: &crate::bstr::BStr,
    old_id: gix_hash::ObjectId,
    new_id: gix_hash::ObjectId,
    allow_non_fast_forward: bool,
) -> Option<&'static str> {
    if allow_non_fast_forward || old_id.is_null() || new_id.is_null() || old_id == new_id {
        return None;
    }

    if dst.starts_with(b"refs/tags/") {
        return Some("already exists");
    }

    if dst.starts_with(b"refs/heads/") && !is_fast_forward(repo, old_id, new_id).unwrap_or(true) {
        return Some("non-fast-forward");
    }

    None
}

fn is_fast_forward(
    repo: &crate::Repository,
    old_id: gix_hash::ObjectId,
    new_id: gix_hash::ObjectId,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let old_commit_time = repo
        .find_object(old_id)?
        .try_into_commit()
        .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)?
        .committer()
        .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)?
        .seconds();
    let mut ancestors = new_id
        .attach(repo)
        .ancestors()
        .sorting(crate::revision::walk::Sorting::ByCommitTimeCutoff {
            order: Default::default(),
            seconds: old_commit_time,
        })
        .all()
        .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)?;

    Ok(ancestors.any(|info| info.is_ok_and(|info| info.id == old_id)))
}

fn record_push_destination(
    dst: &BString,
    src: BString,
    seen: &mut std::collections::HashMap<BString, BString>,
) -> Result<(), Error> {
    if let Some(existing) = seen.get(dst) {
        if existing != &src {
            return Err(Error::DestinationConflict {
                destination: dst.clone(),
                sources: vec![existing.clone(), src],
            });
        }
    } else {
        seen.insert(dst.clone(), src);
    }
    Ok(())
}

fn push_source_label(src: &SourceRef<'_>) -> BString {
    match src {
        SourceRef::FullName(name) => name.as_ref().to_owned(),
        SourceRef::ObjectId(oid) => BString::from(oid.to_string()),
    }
}

fn local_refs_for_push(repo: &crate::Repository) -> Result<Vec<LocalRef>, Error> {
    let mut out = Vec::new();
    if let Ok(head) = repo.head() {
        if let Some(id) = head.id() {
            out.push(LocalRef {
                name: BString::from("HEAD"),
                target: id.detach(),
                object: None,
            });
        }
    }

    let references = repo
        .references()
        .map_err(|err| Error::ListLocalRefs { source: Box::new(err) })?;
    for reference in references
        .all()
        .map_err(|err| Error::ListLocalRefs { source: Box::new(err) })?
    {
        let reference = reference.map_err(|err| Error::ListLocalRefs { source: err })?;
        let raw = reference.detach();
        let target = match raw.target {
            gix_ref::Target::Object(id) => id,
            gix_ref::Target::Symbolic(_) => continue,
        };
        out.push(LocalRef {
            name: raw.name.as_bstr().to_owned(),
            target,
            object: raw.peeled,
        });
    }
    Ok(out)
}

fn expand_push_destination(dst: &crate::bstr::BStr) -> BString {
    if dst.starts_with(b"refs/") || dst.as_bytes() == b"HEAD" {
        return dst.to_owned();
    }

    let mut out = BString::from("refs/");
    if !(dst.starts_with(b"tags/") || dst.starts_with(b"remotes/")) {
        out.push_str("heads/");
    }
    out.push_str(dst);
    out
}

fn remote_ref_to_item<'a>(
    r: &'a gix_protocol::handshake::Ref,
    null: &'a gix_hash::ObjectId,
) -> gix_refspec::match_group::Item<'a> {
    let (full_ref_name, target, object) = r.unpack();
    gix_refspec::match_group::Item {
        full_ref_name,
        target: target.unwrap_or(null.as_ref()),
        object,
    }
}

fn write_pack_for_push(
    odb: &gix_odb::HandleArc,
    new_tips: &[gix_hash::ObjectId],
    known_remote: &[gix_hash::ObjectId],
    object_hash: gix_hash::Kind,
    writer: &mut dyn std::io::Write,
    progress: &mut dyn gix_features::progress::DynNestedProgress,
    should_interrupt: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use gix_features::parallel::InOrderIter;
    use gix_pack::data::output;

    if new_tips.is_empty() {
        return write_empty_pack(writer, object_hash);
    }

    // Walk from new_tips, stopping at commits the remote already has.
    // This ensures we only send objects the remote doesn't have.
    // The filter `!remote_set.contains(oid)` stops the commit walk at known remote commits.
    // The subsequent `TreeAdditionsComparedToAncestor` expansion on the resulting commits
    // correctly enumerates all tree and blob objects that differ from their ancestors,
    // which is the standard approach for computing the minimal set of objects to send.
    let remote_set: gix_hashtable::HashSet = known_remote.iter().copied().collect();
    let new_commits: Vec<gix_hash::ObjectId> =
        gix_traverse::commit::Simple::filtered(new_tips.iter().copied(), odb.clone(), |oid| !remote_set.contains(oid))
            .map(|info| info.map(|i| i.id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

    if new_commits.is_empty() {
        return write_empty_pack(writer, object_hash);
    }

    let counts_progress = progress.add_child_with_id(
        "counting objects".into(),
        gix_protocol::push::ProgressId::CountingObjects.into(),
    );

    let (counts, _stats) = output::count::objects(
        odb.clone(),
        Box::new(
            new_commits
                .into_iter()
                .map(Ok::<_, Box<dyn std::error::Error + Send + Sync>>),
        ),
        &counts_progress,
        should_interrupt,
        output::count::objects::Options {
            input_object_expansion: output::count::objects::ObjectExpansion::TreeAdditionsComparedToAncestor,
            thread_limit: None,
            chunk_size: 50,
        },
    )?;

    let num_objects: u32 = counts
        .len()
        .try_into()
        .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
            format!(
                "object count {} exceeds maximum pack size of {}",
                counts.len(),
                u32::MAX
            )
            .into()
        })?;

    let entries_progress = progress.add_child_with_id(
        "creating entries".into(),
        gix_protocol::push::ProgressId::SendingPack.into(),
    );

    let entries = output::entry::iter_from_counts(
        counts,
        odb.clone(),
        Box::new(entries_progress),
        output::entry::iter_from_counts::Options {
            version: gix_pack::data::Version::V2,
            mode: output::entry::iter_from_counts::Mode::PackCopyAndBaseObjects,
            allow_thin_pack: true,
            thread_limit: None,
            chunk_size: 10,
        },
    );

    let in_order_entries = InOrderIter::from(entries);

    let pack_iter = output::bytes::FromEntriesIter::new(
        in_order_entries,
        writer,
        num_objects,
        gix_pack::data::Version::V2,
        object_hash,
    );

    for chunk in pack_iter {
        let _bytes_written: u64 = chunk.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
    }

    Ok(())
}

/// Write an empty pack (header with 0 objects + trailing hash) to `writer`.
fn write_empty_pack(
    writer: &mut dyn std::io::Write,
    object_hash: gix_hash::Kind,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let header = gix_pack::data::header::encode(gix_pack::data::Version::V2, 0);
    std::io::Write::write_all(writer, &header)?;
    let mut hasher = gix_hash::hasher(object_hash);
    hasher.update(&header);
    let hash = hasher.try_finalize()?;
    std::io::Write::write_all(writer, hash.as_slice())?;
    std::io::Write::flush(writer)?;
    Ok(())
}
