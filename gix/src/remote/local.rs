use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
};

use gix_object::{ObjectRef, Write};
use gix_protocol::{fetch::refmap, handshake::Ref};

use crate::{
    ObjectId, Remote, Repository, Result,
    error::{OptionExt, ResultExt, bail, message},
    remote::{Direction, fetch},
};

impl Remote<'_> {
    /// Copy verified objects reachable from the requested references or object IDs and update the mapped references.
    /// Automatic tag following includes tags whose targets are among the reached objects.
    /// Returns `Ok(None)` when the caller needs to use the regular upload-pack transport.
    /// Protocol restrictions and repository ownership checks apply before any objects are read.
    pub fn try_fetch_local(&self, should_interrupt: &AtomicBool) -> Result<Option<fetch::refs::update::Outcome>> {
        if self.refspecs(Direction::Fetch).is_empty()
            || !self
                .url(Direction::Fetch)
                .is_some_and(|url| url.scheme == gix_url::Scheme::File && url.host().is_none())
        {
            return Ok(None);
        }
        let (url, version) = self.sanitized_url_and_version(Direction::Fetch)?;
        if version != gix_transport::Protocol::V2 {
            return Ok(None);
        }
        let source = crate::open_opts(
            gix_path::from_bstr(&url.path)?,
            crate::open::Options::default()
                .without_repository_environment_overrides()
                .bail_if_untrusted(true),
        )?;
        let config = source.config_snapshot();
        let client = self.repo.config_snapshot();
        if source.object_hash() != self.repo.object_hash()
            || source.is_shallow()?
            || self.repo.is_shallow()?
            || source.namespace().is_some()
            || std::env::var_os("GIT_NAMESPACE").is_some()
            || source.common_dir().join("info/grafts").try_exists().or_error()?
            || source.references()?.prefixed("refs/replace/")?.next().is_some()
            || config.plumbing().sections_by_name("uploadpack").is_some()
            || config.plumbing().raw_values("transfer.hideRefs").is_ok()
            || client.plumbing().raw_values("fetch.fsckObjects").is_ok()
            || client.plumbing().raw_values("transfer.fsckObjects").is_ok()
            || self.name().is_some_and(|name| {
                client
                    .plumbing()
                    .raw_values_by("remote", Some(name.as_bstr()), "uploadpack")
                    .is_ok()
            })
            || config.string("extensions.partialClone").is_some()
            || config
                .plumbing()
                .sections_by_name("remote")
                .is_some_and(|mut sections| sections.any(|section| section.contains_value_name("promisor")))
        {
            return Ok(None);
        }
        check_interrupt(should_interrupt)?;
        let mut refs = Vec::new();
        for reference in source.references()?.all()? {
            match advertise(reference?) {
                Ok(advertised) => refs.push(advertised),
                Err(err) if err.is_not_found() => return Ok(None),
                Err(err) => return Err(err),
            }
        }
        let head = source.head()?;
        if let Some(id) = head.id() {
            refs.push(ref_for_target(
                &source,
                "HEAD".into(),
                head.referent_name().map(|n| n.as_bstr().to_owned()),
                id.detach(),
            )?);
        } else if let Some(target) = head.referent_name() {
            refs.push(Ref::Unborn {
                full_ref_name: "HEAD".into(),
                target: target.as_bstr().to_owned(),
            });
        }
        let caps = gix_transport::client::Capabilities::from_lines(
            format!("version 2\nobject-format={}\n", source.object_hash()).into(),
        )?;
        let ref_map = fetch::RefMap::from_refs(
            refs,
            &caps,
            refmap::init::Context {
                fetch_refspecs: self.refspecs(Direction::Fetch).to_vec(),
                extra_refspecs: self
                    .fetch_tags()
                    .to_refspec()
                    .map(|r| r.to_owned())
                    .into_iter()
                    .collect(),
            },
        )?;
        if ref_map.is_missing_required_mapping() {
            bail!("None of the requested fetch refspecs matched the local source");
        }
        if ref_map.refspecs.len() > 1
            && (0..ref_map.refspecs.len()).any(|i| {
                !ref_map
                    .mappings
                    .iter()
                    .any(|m| m.spec_index == refmap::SpecIndex::ExplicitInRemote(i))
            })
        {
            return Ok(None);
        }
        // Serve an object ID only if a visible ref names it or reaches it through tag targets
        // and commit parents. Let upload-pack decide other object-ID requests.
        let wanted = ref_map
            .mappings
            .iter()
            .filter_map(|m| match m.remote {
                refmap::Source::ObjectId(id) => Some(id),
                _ => None,
            })
            .collect::<HashSet<_>>();
        if !wanted.is_empty() && !reachable_from_refs(&source, &ref_map.remote_refs, wanted, should_interrupt)? {
            return Ok(None);
        }
        let auto_tags = self.fetch_tags() == fetch::Tags::Included;
        let roots = ref_map
            .mappings
            .iter()
            .filter(|m| !auto_tags || m.spec_index.implicit_index().is_none())
            .filter_map(|m| m.remote.as_id().map(ToOwned::to_owned))
            .collect();
        let mut seen = HashSet::new();
        copy_reachable(&source, self.repo, roots, &mut seen, should_interrupt)?;
        if auto_tags {
            let tags = ref_map
                .mappings
                .iter()
                .filter(|m| {
                    m.spec_index.implicit_index().is_some() && m.remote.peeled_id().is_some_and(|id| seen.contains(id))
                })
                .filter_map(|m| m.remote.as_id().map(ToOwned::to_owned))
                .collect();
            copy_reachable(&source, self.repo, tags, &mut seen, should_interrupt)?;
        }
        check_interrupt(should_interrupt)?;
        fetch::refs::update(
            self.repo,
            fetch::RefLogMessage::Prefixed { action: "fetch".into() },
            &ref_map.mappings,
            &ref_map.refspecs,
            &ref_map.extra_refspecs,
            self.fetch_tags(),
            fetch::DryRun::No,
            fetch::WritePackedRefs::Never,
        )
        .map(Some)
    }
}

fn advertise(mut reference: crate::Reference<'_>) -> Result<Ref> {
    let name = reference.name().as_bstr().to_owned();
    let target = match reference.target() {
        gix_ref::TargetRef::Symbolic(target) => Some(target.as_bstr().to_owned()),
        gix_ref::TargetRef::Object(_) => None,
    };
    let id = reference.follow_to_object()?.detach();
    ref_for_target(reference.repo, name, target, id)
}

fn ref_for_target(
    repo: &Repository,
    name: crate::bstr::BString,
    target: Option<crate::bstr::BString>,
    id: ObjectId,
) -> Result<Ref> {
    let object = if repo.find_header(id)?.kind() == gix_object::Kind::Tag {
        repo.find_object(id)?.peel_tags_to_end()?.id
    } else {
        id
    };
    let tag = (id != object).then_some(id);
    Ok(match (target, tag) {
        (Some(target), tag) => Ref::Symbolic {
            full_ref_name: name,
            target,
            tag,
            object,
        },
        (None, Some(tag)) => Ref::Peeled {
            full_ref_name: name,
            tag,
            object,
        },
        (None, None) => Ref::Direct {
            full_ref_name: name,
            object,
        },
    })
}

fn reachable_from_refs(
    repo: &Repository,
    refs: &[Ref],
    mut wanted: HashSet<ObjectId>,
    interrupt: &AtomicBool,
) -> Result<bool> {
    let mut pending: Vec<_> = refs
        .iter()
        .filter_map(|r| r.unpack().1.map(ToOwned::to_owned))
        .collect();
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        if wanted.remove(&id) && wanted.is_empty() {
            return Ok(true);
        }
        let obj = verified(repo, id, interrupt)?;
        match gix_object::Data::new(&obj.data, obj.kind, repo.object_hash()).decode()? {
            ObjectRef::Commit(commit) => pending.extend(commit.parents()),
            ObjectRef::Tag(tag) => pending.push(tag.target()),
            ObjectRef::Tree(_) | ObjectRef::Blob(_) => {}
        }
    }
    Ok(false)
}

fn copy_reachable(
    source: &Repository,
    dest: &Repository,
    mut pending: Vec<ObjectId>,
    seen: &mut HashSet<ObjectId>,
    interrupt: &AtomicBool,
) -> Result<()> {
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let obj = verified(source, id, interrupt)?;
        match gix_object::Data::new(&obj.data, obj.kind, source.object_hash()).decode()? {
            ObjectRef::Commit(commit) => {
                pending.push(commit.tree());
                pending.extend(commit.parents());
            }
            ObjectRef::Tag(tag) => pending.push(tag.target()),
            ObjectRef::Tree(tree) => pending.extend(
                tree.entries
                    .iter()
                    .filter(|e| !e.mode.is_commit())
                    .map(|e| e.oid.to_owned()),
            ),
            ObjectRef::Blob(_) => {}
        }
        if !dest.has_object(id) {
            dest.objects.write_buf_with_known_id(obj.kind, &obj.data, id)?;
        }
    }
    Ok(())
}

fn verified<'repo>(repo: &'repo Repository, id: ObjectId, interrupt: &AtomicBool) -> Result<crate::Object<'repo>> {
    check_interrupt(interrupt)?;
    repo.try_find_object_verified(id)?
        .ok_or_raise(|| message!("Local source is missing object {id}"))
}

fn check_interrupt(interrupt: &AtomicBool) -> Result<()> {
    if interrupt.load(Ordering::Relaxed) {
        bail!(gix_error::cancelled("Local fetch interrupted"));
    }
    Ok(())
}
