#[cfg(any(feature = "blocking-network-client", feature = "async-network-client-async-std"))]
mod blocking_and_async_io {
    use gix::{config::tree::Protocol, remote::Direction::Fetch};
    use gix_features::progress;
    use gix_protocol::maybe_async;

    use crate::{
        remote,
        remote::{into_daemon_remote_if_async, spawn_git_daemon_if_async},
    };

    #[maybe_async::test(
        feature = "blocking-network-client",
        async(feature = "async-network-client-async-std", async_std::test)
    )]
    async fn negative_refspecs_exclude_matching_refs_from_mappings() -> crate::Result {
        let daemon = spawn_git_daemon_if_async(remote::repo_path("base"))?;
        let repo = remote::repo("clone");

        // First, fetch without negative refspecs to get the baseline count.
        let remote = into_daemon_remote_if_async(
            repo.find_remote("origin")?
                .with_fetch_tags(gix::remote::fetch::Tags::None),
            daemon.as_ref(),
            None,
        );
        let (baseline_map, _) = remote
            .connect(Fetch)
            .await?
            .ref_map(progress::Discard, Default::default())
            .await?;
        let baseline_count = baseline_map.mappings.len();
        assert!(
            baseline_count > 2,
            "need at least a few branches to meaningfully test exclusion, got {baseline_count}"
        );

        // Find two branch names we can exclude.
        let branch_names: Vec<String> = baseline_map
            .mappings
            .iter()
            .filter_map(|m| {
                let name = m.remote.as_name()?.to_string();
                name.starts_with("refs/heads/").then_some(name)
            })
            .collect();
        assert!(
            branch_names.len() >= 2,
            "need at least 2 branch mappings, got: {branch_names:?}"
        );
        let excluded_branch = &branch_names[0];

        // Now fetch with a negative refspec excluding that branch.
        let neg_spec = gix::refspec::parse(
            format!("^{excluded_branch}").as_str().into(),
            gix::refspec::parse::Operation::Fetch,
        )?
        .to_owned();

        let remote = into_daemon_remote_if_async(
            repo.find_remote("origin")?
                .with_fetch_tags(gix::remote::fetch::Tags::None),
            daemon.as_ref(),
            None,
        );
        let (map_with_exclusion, _) = remote
            .connect(Fetch)
            .await?
            .ref_map(
                progress::Discard,
                gix::remote::ref_map::Options {
                    extra_refspecs: vec![neg_spec],
                    ..Default::default()
                },
            )
            .await?;

        // The excluded branch should not appear in mappings.
        let excluded_in_mappings = map_with_exclusion
            .mappings
            .iter()
            .any(|m| m.remote.as_name().is_some_and(|n| n.to_string() == *excluded_branch));
        assert!(
            !excluded_in_mappings,
            "branch {excluded_branch:?} should have been excluded by the negative refspec"
        );

        // We should have exactly one fewer mapping.
        assert_eq!(
            map_with_exclusion.mappings.len(),
            baseline_count - 1,
            "negative refspec should exclude exactly one mapping"
        );

        // Verify with a glob negative refspec: exclude all branches starting with a specific prefix.
        // Use "refs/heads/f*" to exclude branch "f" (and any others starting with f).
        let neg_glob = gix::refspec::parse("^refs/heads/f*".into(), gix::refspec::parse::Operation::Fetch)?.to_owned();
        let remote = into_daemon_remote_if_async(
            repo.find_remote("origin")?
                .with_fetch_tags(gix::remote::fetch::Tags::None),
            daemon.as_ref(),
            None,
        );
        let (map_with_glob_exclusion, _) = remote
            .connect(Fetch)
            .await?
            .ref_map(
                progress::Discard,
                gix::remote::ref_map::Options {
                    extra_refspecs: vec![neg_glob],
                    ..Default::default()
                },
            )
            .await?;

        // Count how many branches matched "refs/heads/f*" in baseline.
        let f_branch_count = branch_names.iter().filter(|n| n.starts_with("refs/heads/f")).count();
        assert!(f_branch_count > 0, "expected at least one f* branch in test fixture");
        assert_eq!(
            map_with_glob_exclusion.mappings.len(),
            baseline_count - f_branch_count,
            "negative glob refspec should exclude all refs/heads/f* branches"
        );
        // None of the f* branches should remain.
        let any_f_remaining = map_with_glob_exclusion.mappings.iter().any(|m| {
            m.remote
                .as_name()
                .is_some_and(|n| n.to_string().starts_with("refs/heads/f"))
        });
        assert!(
            !any_f_remaining,
            "no refs/heads/f* branches should remain after negative glob exclusion"
        );

        Ok(())
    }

    #[maybe_async::test(
        feature = "blocking-network-client",
        async(feature = "async-network-client-async-std", async_std::test)
    )]
    async fn all() -> crate::Result {
        let daemon = spawn_git_daemon_if_async(remote::repo_path("base"))?;
        for (fetch_tags, version, expected_remote_refs, expected_mappings) in [
            (gix::remote::fetch::Tags::None, None, 11, 11),
            (
                gix::remote::fetch::Tags::None,
                Some(gix::protocol::transport::Protocol::V2),
                11,
                11,
            ),
            (
                gix::remote::fetch::Tags::Included,
                Some(gix::protocol::transport::Protocol::V2),
                17,
                17,
            ),
            (
                gix::remote::fetch::Tags::All,
                Some(gix::protocol::transport::Protocol::V2),
                17,
                17,
            ),
            (
                gix::remote::fetch::Tags::None,
                Some(gix::protocol::transport::Protocol::V1),
                18,
                11,
            ),
            (
                gix::remote::fetch::Tags::Included,
                Some(gix::protocol::transport::Protocol::V1),
                18,
                17,
            ),
            (
                gix::remote::fetch::Tags::All,
                Some(gix::protocol::transport::Protocol::V1),
                18,
                17,
            ),
        ] {
            let mut repo = remote::repo("clone");
            if let Some(version) = version {
                repo.config_snapshot_mut()
                    .set_raw_value(&Protocol::VERSION, (version as u8).to_string().as_str())?;
            }

            let remote = into_daemon_remote_if_async(
                repo.find_remote("origin")?.with_fetch_tags(fetch_tags),
                daemon.as_ref(),
                None,
            );
            let (map, _handshake) = remote
                .connect(Fetch)
                .await?
                .ref_map(progress::Discard, Default::default())
                .await?;
            assert_eq!(
                    map.remote_refs.len(),
                    expected_remote_refs ,
                    "{version:?} fetch-tags={fetch_tags:?}: it gets all remote refs, independently of the refspec. But we use a prefix so pre-filter them."
                );

            assert_eq!(map.fixes.len(), 0);
            assert_eq!(
                map.mappings.len(),
                expected_mappings,
                "mappings are only a sub-set of all remotes due to refspec matching, tags are filtered out."
            );
        }
        Ok(())
    }
}
