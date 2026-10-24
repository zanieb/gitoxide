mod single {
    use crate::matching::baseline;

    fn test_hashes() -> (String, String) {
        let annotated_tag = match gix_testtools::object_hash() {
            gix_hash::Kind::Sha1 => "78b1c1be9421b33a49a7a8176d93eeeafa112da1",
            gix_hash::Kind::Sha256 => "b071221ea854da2958fba3a37527ca5cf32c4ebcd71ab0b68b6b8f10f04e93ad",
            _ => unimplemented!(),
        };
        let initial_commit = match gix_testtools::object_hash() {
            gix_hash::Kind::Sha1 => "9d2fab1a0ba3585d0bc50922bfdd04ebb59361df",
            gix_hash::Kind::Sha256 => "ac050883b75422e0d03bfee760c591b292cbc10cee8ad934480ea5fb2ebc44fe",
            _ => unimplemented!(),
        };

        (annotated_tag.into(), initial_commit.into())
    }

    #[test]
    fn fetch_only() {
        let (annotated_tag, initial_commit) = test_hashes();

        baseline::agrees_with_fetch_specs(Some("refs/heads/main"));
        baseline::agrees_with_fetch_specs(Some("heads/main"));
        baseline::agrees_with_fetch_specs(Some("main"));
        baseline::agrees_with_fetch_specs(Some("v0.0-f1"));
        baseline::agrees_with_fetch_specs(Some("tags/v0.0-f2"));
        baseline::of_objects_always_matches_if_the_server_has_the_object(Some(annotated_tag.as_ref()));
        baseline::of_objects_always_matches_if_the_server_has_the_object(Some(initial_commit.as_ref()));
    }

    #[test]
    fn fetch_and_update() {
        let (annotated_tag, initial_commit) = test_hashes();

        baseline::of_objects_with_destinations_are_written_into_given_local_branches(
            Some(format!("{annotated_tag}:special").as_ref()),
            [format!("{annotated_tag}:refs/heads/special").as_ref()],
        );
        baseline::of_objects_with_destinations_are_written_into_given_local_branches(
            Some(format!("{annotated_tag}:1111111111111111111111111111111111111111").as_ref()),
            [format!("{annotated_tag}:refs/heads/1111111111111111111111111111111111111111").as_ref()],
        );
        baseline::of_objects_with_destinations_are_written_into_given_local_branches(
            Some(format!("{initial_commit}:tags/special").as_ref()),
            [format!("{initial_commit}:refs/tags/special").as_ref()],
        );
        baseline::of_objects_with_destinations_are_written_into_given_local_branches(
            Some(format!("{initial_commit}:refs/tags/special").as_ref()),
            [format!("{initial_commit}:refs/tags/special").as_ref()],
        );

        baseline::agrees_but_observable_refs_are_vague(Some("f1:origin/f1"), ["refs/heads/f1:refs/heads/origin/f1"]);
        baseline::agrees_but_observable_refs_are_vague(
            Some("f1:remotes/origin/f1"),
            ["refs/heads/f1:refs/remotes/origin/f1"],
        );
        baseline::agrees_but_observable_refs_are_vague(Some("f1:notes/f1"), ["refs/heads/f1:refs/heads/notes/f1"]);
        baseline::agrees_with_fetch_specs(Some("+refs/heads/*:refs/remotes/origin/*"));
        baseline::agrees_with_fetch_specs(Some("refs/heads/f*:refs/remotes/origin/a*"));
        baseline::agrees_with_fetch_specs(Some("refs/heads/*1:refs/remotes/origin/*1"));
    }
}

mod multiple {
    use bstr::BString;
    use gix_hash::ObjectId;
    use gix_refspec::{MatchGroup, match_group::Item, match_group::validate::Fix, parse::Operation};

    use crate::matching::baseline;

    #[test]
    fn fetch_only() {
        baseline::agrees_with_fetch_specs(["main", "f1"]);
        baseline::agrees_with_fetch_specs(["heads/main", "heads/f1"]);
        baseline::agrees_with_fetch_specs(["refs/heads/main", "refs/heads/f1"]);
        baseline::agrees_with_fetch_specs(["heads/f1", "f2", "refs/heads/f3", "heads/main"]);
        baseline::agrees_with_fetch_specs(["f*:a*", "refs/heads/main"]);
        baseline::agrees_with_fetch_specs([
            "refs/tags/*:refs/remotes/origin/*",
            "refs/heads/*:refs/remotes/origin/*",
        ]);
        baseline::agrees_with_fetch_specs(["refs/tags/*:refs/tags/*"]);
    }

    #[test]
    fn fetch_and_update_and_negations() {
        baseline::agrees_with_fetch_specs(["refs/heads/f*:refs/remotes/origin/a*", "^f1"]);
        baseline::agrees_with_fetch_specs(["refs/heads/f*:refs/remotes/origin/a*", "^refs/heads/f1"]);
        baseline::agrees_with_fetch_specs(["^heads/f2", "refs/heads/f*:refs/remotes/origin/a*"]);
        baseline::agrees_with_fetch_specs(["^refs/heads/f2", "refs/heads/f*:refs/remotes/origin/a*"]);
        baseline::agrees_with_fetch_specs(["^main", "refs/heads/*:refs/remotes/origin/*"]);
        baseline::agrees_with_fetch_specs(["^refs/heads/main", "refs/heads/*:refs/remotes/origin/*"]);
        baseline::agrees_with_fetch_specs(["refs/heads/*:refs/remotes/origin/*", "^refs/heads/main"]);
        baseline::agrees_with_fetch_specs(["refs/heads/*:refs/remotes/origin/*", "^refs/heads/*-deploy"]);
    }

    #[test]
    fn negative_glob_excludes_matching_fetch_sources() -> gix_testtools::TestResult {
        let target = gix_testtools::object_hash().null();
        let items = [
            "refs/heads/main",
            "refs/heads/feature-deploy",
            "refs/heads/bugfix",
            "refs/heads/release-deploy",
        ]
        .into_iter()
        .map(|name| Item {
            full_ref_name: name.into(),
            target: &target,
            object: None,
        });
        let specs = ["refs/heads/*:refs/remotes/origin/*", "^refs/heads/*-deploy"]
            .into_iter()
            .map(|spec| gix_refspec::parse(spec.into(), Operation::Fetch))
            .collect::<gix_error::Result<Vec<_>>>()?;
        let outcome = MatchGroup::from_fetch_specs(specs).match_lhs(items);
        let sources: Vec<_> = outcome.mappings.iter().map(|mapping| mapping.lhs.to_string()).collect();
        assert_eq!(sources, ["refs/heads/main", "refs/heads/bugfix"]);
        Ok(())
    }

    #[test]
    fn negative_sources_are_matched_literally_in_both_directions() {
        let target = ObjectId::from_hex(b"1111111111111111111111111111111111111111").expect("valid object id");
        let remote_item = Item {
            full_ref_name: "refs/heads/main".into(),
            target: &target,
            object: None,
        };
        let local_item = Item {
            full_ref_name: "refs/remotes/origin/main".into(),
            target: &target,
            object: None,
        };
        let positive = "refs/heads/*:refs/remotes/origin/*";

        for (negative, should_exclude) in [
            ("^main", false),
            ("^heads/*", false),
            ("^refs/heads/main", true),
            ("^refs/heads/*", true),
        ] {
            let specs = || {
                [positive, negative]
                    .into_iter()
                    .map(|spec| gix_refspec::parse(spec.into(), Operation::Fetch).expect("valid refspec"))
            };

            let outcome = MatchGroup::from_fetch_specs(specs()).match_lhs([remote_item].into_iter());
            assert_eq!(
                outcome.mappings.is_empty(),
                should_exclude,
                "{negative} applies literally when matching sources"
            );

            let outcome = MatchGroup::from_fetch_specs(specs()).match_rhs([local_item].into_iter());
            assert_eq!(
                outcome.mappings.is_empty(),
                should_exclude,
                "{negative} applies literally after reverse mapping destinations"
            );
        }
    }

    #[test]
    fn reverse_fetch_mapping_preserves_sources_for_negation_and_deduplication() -> gix_testtools::TestResult {
        let target = gix_testtools::object_hash().null();
        let item = Item {
            full_ref_name: "refs/remotes/origin/main".into(),
            target: &target,
            object: None,
        };
        for (negative, expected) in [
            ("^main", vec!["refs/heads/main"]),
            ("^refs/heads/main", vec!["main"]),
            ("^heads/main", vec!["main", "refs/heads/main"]),
        ] {
            let specs = [
                "main:refs/remotes/origin/main",
                "refs/heads/main:refs/remotes/origin/main",
                "main:refs/remotes/origin/main",
                negative,
            ]
            .into_iter()
            .map(|spec| gix_refspec::parse(spec.into(), Operation::Fetch))
            .collect::<gix_error::Result<Vec<_>>>()?;
            let outcome = MatchGroup::from_fetch_specs(specs).match_rhs(std::iter::once(item));
            let sources: Vec<_> = outcome.mappings.iter().map(|mapping| mapping.lhs.to_string()).collect();
            assert_eq!(
                sources, expected,
                "{negative} excludes literal sources, and only identical mappings are deduplicated"
            );
        }
        Ok(())
    }

    #[test]
    fn glob_prefix_and_suffix_cannot_overlap_in_either_direction() -> gix_testtools::TestResult {
        let target = gix_testtools::object_hash().null();
        let item = Item {
            full_ref_name: "refs/heads/main".into(),
            target: &target,
            object: None,
        };
        let spec = gix_refspec::parse("refs/heads/*heads/main:refs/heads/*heads/main".into(), Operation::Fetch)?;
        let group = MatchGroup::from_fetch_specs([spec]);
        assert!(
            group.clone().match_lhs(std::iter::once(item)).mappings.is_empty(),
            "a glob capture cannot have negative length when matching sources"
        );
        assert!(
            group.match_rhs(std::iter::once(item)).mappings.is_empty(),
            "a glob capture cannot have negative length when matching destinations"
        );
        Ok(())
    }

    #[test]
    fn reverse_fetch_mapping_honors_negative_source_patterns() {
        let specs = ["refs/heads/*:refs/remotes/origin/*", "^refs/heads/*-deploy"]
            .into_iter()
            .map(|spec| gix_refspec::parse(spec.into(), Operation::Fetch).expect("valid refspec"));
        let target = ObjectId::from_hex(b"1111111111111111111111111111111111111111").expect("valid object id");
        let refs = [
            BString::from("refs/remotes/origin/main"),
            BString::from("refs/remotes/origin/foo-deploy"),
        ];
        let items: Vec<_> = refs
            .iter()
            .map(|name| Item {
                full_ref_name: name.as_ref(),
                target: &target,
                object: None,
            })
            .collect();

        let outcome = MatchGroup::from_fetch_specs(specs).match_rhs(items.iter().copied());
        insta::assert_debug_snapshot!(outcome.mappings, @r#"
        [
            Mapping {
                item_index: Some(
                    0,
                ),
                lhs: FullName(
                    "refs/heads/main",
                ),
                rhs: Some(
                    "refs/remotes/origin/main",
                ),
                spec_index: 0,
            },
        ]
        "#);
    }

    #[test]
    fn fetch_and_update_with_empty_lhs() {
        baseline::agrees_but_observable_refs_are_vague([":refs/heads/f1"], ["HEAD:refs/heads/f1"]);
        baseline::agrees_but_observable_refs_are_vague([":f1"], ["HEAD:refs/heads/f1"]);
        baseline::agrees_but_observable_refs_are_vague(["@:f1"], ["HEAD:refs/heads/f1"]);
    }

    #[test]
    fn fetch_and_update_head_to_head_never_updates_actual_head_ref() {
        baseline::agrees_but_observable_refs_are_vague(["@:HEAD"], ["HEAD:refs/heads/HEAD"]);
    }

    #[test]
    fn fetch_and_update_head_with_empty_rhs() {
        baseline::agrees_but_observable_refs_are_vague([":"], ["HEAD:"]);
        baseline::agrees_but_observable_refs_are_vague(["HEAD:"], ["HEAD:"]);
        baseline::agrees_but_observable_refs_are_vague(["@:"], ["HEAD:"]);
    }

    #[test]
    fn fetch_and_update_multiple_destinations() {
        baseline::agrees_with_fetch_specs([
            "refs/heads/*:refs/remotes/origin/*",
            "refs/heads/main:refs/remotes/new-origin/main",
        ]);
        baseline::agrees_with_fetch_specs([
            "refs/heads/*:refs/remotes/origin/*",
            "refs/heads/main:refs/remotes/origin/main", // duplicates are removed immediately.
        ]);
    }

    #[test]
    fn fetch_and_update_with_conflicts() {
        insta::assert_debug_snapshot!(baseline::agrees_with_fetch_specs_validation_error([
                "refs/heads/f1:refs/remotes/origin/conflict",
                "refs/heads/f2:refs/remotes/origin/conflict",
            ]), "conflicting destinations identify every contributing refspec", @"
        Found 1 issue that prevents the refspec mapping to be used:\x20
        \tConflicting destination \"refs/remotes/origin/conflict\" would be written by refs/heads/f1 (\"refs/heads/f1:refs/remotes/origin/conflict\"), refs/heads/f2 (\"refs/heads/f2:refs/remotes/origin/conflict\")
        ");
        insta::assert_debug_snapshot!(baseline::agrees_with_fetch_specs_validation_error([
                "refs/heads/f1:refs/remotes/origin/conflict2",
                "refs/heads/f2:refs/remotes/origin/conflict2",
                "refs/heads/f1:refs/remotes/origin/conflict",
                "refs/heads/f2:refs/remotes/origin/conflict",
                "refs/heads/f3:refs/remotes/origin/conflict",
            ]), "conflicting destinations identify every contributing refspec", @"
        Found 2 issues that prevent the refspec mapping to be used:\x20
        \tConflicting destination \"refs/remotes/origin/conflict\" would be written by refs/heads/f1 (\"refs/heads/f1:refs/remotes/origin/conflict\"), refs/heads/f2 (\"refs/heads/f2:refs/remotes/origin/conflict\"), refs/heads/f3 (\"refs/heads/f3:refs/remotes/origin/conflict\")
        \tConflicting destination \"refs/remotes/origin/conflict2\" would be written by refs/heads/f1 (\"refs/heads/f1:refs/remotes/origin/conflict2\"), refs/heads/f2 (\"refs/heads/f2:refs/remotes/origin/conflict2\")
        ");
        insta::assert_debug_snapshot!(baseline::agrees_with_fetch_specs_validation_error([
                "refs/heads/f1:refs/remotes/origin/same",
                "refs/tags/v0.0-f1:refs/remotes/origin/same",
            ]), "conflicting destinations identify every contributing refspec", @"
        Found 1 issue that prevents the refspec mapping to be used:\x20
        \tConflicting destination \"refs/remotes/origin/same\" would be written by refs/heads/f1 (\"refs/heads/f1:refs/remotes/origin/same\"), refs/tags/v0.0-f1 (\"refs/tags/v0.0-f1:refs/remotes/origin/same\")
        ");
        insta::assert_debug_snapshot!(baseline::agrees_with_fetch_specs_validation_error([
                "+refs/heads/*:refs/remotes/origin/*",
                "refs/heads/f1:refs/remotes/origin/f2",
                "refs/heads/f2:refs/remotes/origin/f1",
            ]), "conflicting destinations identify every contributing refspec", @"
        Found 2 issues that prevent the refspec mapping to be used:\x20
        \tConflicting destination \"refs/remotes/origin/f1\" would be written by refs/heads/f1 (\"+refs/heads/*:refs/remotes/origin/*\"), refs/heads/f2 (\"refs/heads/f2:refs/remotes/origin/f1\")
        \tConflicting destination \"refs/remotes/origin/f2\" would be written by refs/heads/f2 (\"+refs/heads/*:refs/remotes/origin/*\"), refs/heads/f1 (\"refs/heads/f1:refs/remotes/origin/f2\")
        ");
    }

    #[test]
    fn fetch_and_update_with_fixes() {
        let glob_spec = "refs/heads/f*:foo/f*";
        let glob_spec_ref = gix_refspec::parse(glob_spec.into(), Operation::Fetch).unwrap();
        baseline::agrees_and_applies_fixes(
            [glob_spec, "f1:f1"],
            [
                Fix::MappingWithPartialDestinationRemoved {
                    name: "foo/f1".into(),
                    spec: glob_spec_ref.to_owned(),
                },
                Fix::MappingWithPartialDestinationRemoved {
                    name: "foo/f2".into(),
                    spec: glob_spec_ref.to_owned(),
                },
                Fix::MappingWithPartialDestinationRemoved {
                    name: "foo/f3".into(),
                    spec: glob_spec_ref.to_owned(),
                },
            ],
            ["refs/heads/f1:refs/heads/f1"],
        );
    }
}

mod push {
    use bstr::{BString, ByteSlice};
    use gix_hash::ObjectId;
    use gix_refspec::{
        MatchGroup, RefSpec,
        match_group::{PushDeletion, PushUpdate, SourceRef},
        parse::Operation,
    };

    #[test]
    fn explicit_refs_and_implicit_destinations() {
        let local = [
            new_ref("refs/heads/main", "1111111111111111111111111111111111111111"),
            new_ref("refs/heads/dev", "2222222222222222222222222222222222222222"),
            new_ref("refs/tags/v1", "3333333333333333333333333333333333333333"),
        ];
        let remote = [new_ref("refs/heads/main", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")];
        let specs = parse_specs([
            "main",
            "refs/heads/dev:refs/heads/review/dev",
            "+refs/tags/v1:refs/tags/v1",
        ]);
        let outcome = group(&specs).match_push(items(&local), items(&remote));

        assert_eq!(
            outcome.updates,
            [
                update(0, Some(0), "refs/heads/main", "refs/heads/main", 0, false),
                update(1, None, "refs/heads/dev", "refs/heads/review/dev", 1, false),
                update(2, None, "refs/tags/v1", "refs/tags/v1", 2, true),
            ]
        );
        assert!(outcome.deletions.is_empty());
    }

    #[test]
    fn glob_updates_and_negative_specs() {
        let local = [
            new_ref("refs/heads/main", "1111111111111111111111111111111111111111"),
            new_ref("refs/heads/feature-deploy", "2222222222222222222222222222222222222222"),
            new_ref("refs/heads/bugfix", "3333333333333333333333333333333333333333"),
            new_ref("refs/tags/v1", "4444444444444444444444444444444444444444"),
        ];
        let remote = [new_ref("refs/heads/main", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")];
        let specs = parse_specs(["refs/heads/*:refs/heads/*", "^refs/heads/*-deploy"]);
        let outcome = group(&specs).match_push(items(&local), items(&remote));

        assert_eq!(
            outcome.updates,
            [
                update(0, Some(0), "refs/heads/main", "refs/heads/main", 0, false),
                update(2, None, "refs/heads/bugfix", "refs/heads/bugfix", 0, false),
            ]
        );
        assert!(outcome.deletions.is_empty());
    }

    #[test]
    fn all_matching_branches_only_updates_existing_remote_branches() {
        let local = [
            new_ref("refs/heads/main", "1111111111111111111111111111111111111111"),
            new_ref("refs/heads/local-only", "2222222222222222222222222222222222222222"),
            new_ref("refs/tags/v1", "3333333333333333333333333333333333333333"),
        ];
        let remote = [
            new_ref("refs/heads/main", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            new_ref("refs/heads/remote-only", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            new_ref("refs/tags/v1", "cccccccccccccccccccccccccccccccccccccccc"),
        ];
        let specs = parse_specs([":"]);
        let outcome = group(&specs).match_push(items(&local), items(&remote));

        assert_eq!(
            outcome.updates,
            [update(0, Some(0), "refs/heads/main", "refs/heads/main", 0, false)]
        );
        assert!(outcome.deletions.is_empty());
    }

    #[test]
    fn deletes_are_expanded_and_matched_to_remote_refs() {
        let remote = [
            new_ref("refs/heads/obsolete", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            new_ref("refs/tags/v1", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ];
        let specs = parse_specs([":obsolete", ":refs/tags/v1"]);
        let outcome = group(&specs).match_push(std::iter::empty(), items(&remote));

        assert!(outcome.updates.is_empty());
        assert_eq!(
            outcome.deletions,
            [
                deletion(Some(0), "refs/heads/obsolete", 0),
                deletion(Some(1), "refs/tags/v1", 1),
            ]
        );
    }

    #[test]
    fn negative_specs_filter_deletes_by_destination() {
        let remote = [
            new_ref("refs/heads/main", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            new_ref("refs/heads/feature", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ];
        let specs = parse_specs([":refs/heads/main", ":refs/heads/feature", "^refs/heads/feature"]);
        let outcome = group(&specs).match_push(std::iter::empty(), items(&remote));

        assert!(outcome.updates.is_empty());
        assert_eq!(
            outcome.deletions,
            [deletion(Some(0), "refs/heads/main", 0)],
            "negative push refspecs should exclude matching deletion destinations"
        );
    }

    #[test]
    fn object_ids_can_be_pushed_to_a_named_destination() {
        let object_id = "1111111111111111111111111111111111111111";
        let specs = parse_specs([format!("{object_id}:refs/heads/by-id")]);
        let outcome = group(&specs).match_push(std::iter::empty(), std::iter::empty());

        assert_eq!(
            outcome.updates,
            [PushUpdate {
                local_item_index: None,
                remote_item_index: None,
                src: SourceRef::ObjectId(ObjectId::from_hex(object_id.as_bytes()).unwrap()),
                dst: "refs/heads/by-id".into(),
                spec_index: 0,
                allow_non_fast_forward: false,
            }]
        );
        assert!(outcome.deletions.is_empty());
    }

    #[test]
    fn duplicate_push_specs_keep_first_mapping() {
        let local = [new_ref("refs/heads/main", "1111111111111111111111111111111111111111")];
        let remote = [new_ref("refs/heads/main", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")];
        let specs = parse_specs(["main", "refs/heads/main:refs/heads/main"]);
        let outcome = group(&specs).match_push(items(&local), items(&remote));

        assert_eq!(
            outcome.updates,
            [update(0, Some(0), "refs/heads/main", "refs/heads/main", 0, false)],
            "the second refspec maps the same source and destination and should be ignored"
        );
        assert!(outcome.deletions.is_empty());
    }

    #[test]
    fn duplicate_delete_specs_keep_first_mapping() {
        let remote = [new_ref(
            "refs/heads/obsolete",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )];
        let specs = parse_specs([":obsolete", ":refs/heads/obsolete"]);
        let outcome = group(&specs).match_push(std::iter::empty(), items(&remote));

        assert!(outcome.updates.is_empty());
        assert_eq!(
            outcome.deletions,
            [deletion(Some(0), "refs/heads/obsolete", 0)],
            "duplicate deletions should not ask the remote to delete the same ref twice"
        );
    }

    fn parse_specs(specs: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<RefSpec> {
        specs
            .into_iter()
            .map(|spec| {
                gix_refspec::parse(spec.as_ref().into(), Operation::Push)
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    fn group(specs: &[RefSpec]) -> MatchGroup<'_> {
        MatchGroup::from_push_specs(specs.iter().map(RefSpec::to_ref))
    }

    fn update(
        local_item_index: usize,
        remote_item_index: Option<usize>,
        src: &str,
        dst: &str,
        spec_index: usize,
        allow_non_fast_forward: bool,
    ) -> PushUpdate {
        PushUpdate {
            local_item_index: Some(local_item_index),
            remote_item_index,
            src: SourceRef::FullName(src.as_bytes().as_bstr().to_owned().into()).into_owned(),
            dst: dst.into(),
            spec_index,
            allow_non_fast_forward,
        }
    }

    fn deletion(remote_item_index: Option<usize>, dst: &str, spec_index: usize) -> PushDeletion {
        PushDeletion {
            remote_item_index,
            dst: dst.into(),
            spec_index,
        }
    }

    fn items(refs: &[Ref]) -> impl Iterator<Item = gix_refspec::match_group::Item<'_>> {
        refs.iter().map(Ref::to_item)
    }

    fn new_ref(name: &str, id_hex: &str) -> Ref {
        Ref {
            name: name.into(),
            target: ObjectId::from_hex(id_hex.as_bytes()).unwrap(),
            object: None,
        }
    }

    #[derive(Debug, Clone)]
    struct Ref {
        name: BString,
        target: ObjectId,
        object: Option<ObjectId>,
    }

    impl Ref {
        fn to_item(&self) -> gix_refspec::match_group::Item<'_> {
            gix_refspec::match_group::Item {
                full_ref_name: self.name.as_ref(),
                target: &self.target,
                object: self.object.as_deref(),
            }
        }
    }
}
