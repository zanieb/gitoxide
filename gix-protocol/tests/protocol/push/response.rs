use bstr::BString;
use gix_protocol::push::response::{parse_v1, StatusV1, UnpackStatus};

/// Ported from git/t/t5516-fetch-push.sh and the git push protocol specification.
/// Tests cover the v1 push response parsing including:
/// - Successful unpack with ok/ng ref statuses
/// - Failed unpack with error reason
/// - Mixed ok/ng responses
/// - Edge cases: empty response, missing unpack line, empty ref names

mod parse_v1_unpack_status {
    use super::*;

    #[test]
    fn unpack_ok_with_single_ref_ok() {
        let response = b"unpack ok\nok refs/heads/main\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 1);
        assert!(refs[0].is_ok());
        assert_eq!(refs[0].ref_name(), "refs/heads/main");
    }

    #[test]
    fn unpack_ok_with_multiple_ref_updates() {
        // Corresponds to git push with multiple refspecs: "push origin main develop feature"
        let response = b"unpack ok\nok refs/heads/main\nok refs/heads/develop\nok refs/heads/feature\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].ref_name(), "refs/heads/main");
        assert_eq!(refs[1].ref_name(), "refs/heads/develop");
        assert_eq!(refs[2].ref_name(), "refs/heads/feature");
        assert!(refs.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn unpack_ok_with_rejected_ref() {
        // Corresponds to: git push that fails non-fast-forward check on one ref
        // From t5516: 'failed (non-fast-forward) push with matching heads'
        let response = b"unpack ok\nng refs/heads/main non-fast-forward\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok(), "unpack succeeded even though ref update was rejected");
        assert_eq!(refs.len(), 1);
        assert!(!refs[0].is_ok());
        match &refs[0] {
            StatusV1::Ng { ref_name, reason } => {
                assert_eq!(ref_name, &BString::from("refs/heads/main"));
                assert_eq!(reason, &BString::from("non-fast-forward"));
            }
            _ => panic!("expected Ng status"),
        }
    }

    #[test]
    fn unpack_ok_with_mixed_ok_and_ng() {
        // Corresponds to: push with multiple refs where some succeed and some fail
        // e.g., "git push origin main develop" where develop is non-ff
        let response = b"unpack ok\nok refs/heads/main\nng refs/heads/develop non-fast-forward\nok refs/tags/v1.0\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 3);
        assert!(refs[0].is_ok());
        assert_eq!(refs[0].ref_name(), "refs/heads/main");
        assert!(!refs[1].is_ok());
        assert_eq!(refs[1].ref_name(), "refs/heads/develop");
        assert!(refs[2].is_ok());
        assert_eq!(refs[2].ref_name(), "refs/tags/v1.0");
    }

    #[test]
    fn unpack_failed_with_error_reason() {
        // Corresponds to: server fails to unpack the sent objects
        let response = b"unpack index-pack abnormal exit\nng refs/heads/main unpacker error\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(!status.is_ok());
        match &status {
            UnpackStatus::Failed { reason } => {
                assert_eq!(reason, &BString::from("index-pack abnormal exit"));
            }
            _ => panic!("expected Failed status"),
        }
        assert_eq!(refs.len(), 1);
        assert!(!refs[0].is_ok());
    }

    #[test]
    fn unpack_failed_generic_error() {
        let response = b"unpack error: disk full\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(!status.is_ok());
        match &status {
            UnpackStatus::Failed { reason } => {
                assert_eq!(reason, &BString::from("error: disk full"));
            }
            _ => panic!("expected Failed status"),
        }
        assert!(
            refs.is_empty(),
            "no ref statuses after unpack failure with no ref lines"
        );
    }

    #[test]
    fn delete_ref_ok() {
        // Corresponds to: git push origin :refs/heads/branch-to-delete
        // From t5516: 'allow deleting an invalid remote ref'
        let response = b"unpack ok\nok refs/heads/branch-to-delete\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 1);
        assert!(refs[0].is_ok());
        assert_eq!(refs[0].ref_name(), "refs/heads/branch-to-delete");
    }

    #[test]
    fn tag_push_ok() {
        // Corresponds to: git push origin v1.0
        // From t5516: 'push tag with non-existent, incomplete dest'
        let response = b"unpack ok\nok refs/tags/v1.0\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 1);
        assert!(refs[0].is_ok());
        assert_eq!(refs[0].ref_name(), "refs/tags/v1.0");
    }
}

mod parse_v1_edge_cases {
    use super::*;

    #[test]
    fn empty_response_is_error() {
        let response = b"";
        let result = parse_v1(response);
        assert!(result.is_err(), "empty response should fail");
    }

    #[test]
    fn missing_unpack_line_is_error() {
        let response = b"ok refs/heads/main\n";
        let result = parse_v1(response);
        assert!(result.is_err(), "response without 'unpack' line should fail");
    }

    #[test]
    fn invalid_first_line_is_error() {
        let response = b"something unexpected\nok refs/heads/main\n";
        let result = parse_v1(response);
        assert!(result.is_err(), "invalid first line should fail");
    }

    #[test]
    fn unpack_ok_with_no_ref_statuses() {
        // Edge case: server says unpack ok but reports no ref statuses
        let response = b"unpack ok\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert!(refs.is_empty());
    }

    #[test]
    fn ng_without_reason() {
        // Edge case: ng line with ref name but no space-separated reason
        let response = b"unpack ok\nng refs/heads/broken\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 1);
        match &refs[0] {
            StatusV1::Ng { ref_name, reason } => {
                assert_eq!(ref_name, &BString::from("refs/heads/broken"));
                assert_eq!(reason, &BString::from("unknown reason"));
            }
            _ => panic!("expected Ng"),
        }
    }

    #[test]
    fn empty_lines_are_skipped() {
        let response = b"unpack ok\n\nok refs/heads/main\n\nok refs/heads/develop\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn ng_with_reason_containing_spaces() {
        // The reason can contain spaces
        let response = b"unpack ok\nng refs/heads/main hook declined push to branch\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 1);
        match &refs[0] {
            StatusV1::Ng { ref_name, reason } => {
                assert_eq!(ref_name, &BString::from("refs/heads/main"));
                assert_eq!(reason, &BString::from("hook declined push to branch"));
            }
            _ => panic!("expected Ng"),
        }
    }

    #[test]
    fn unpack_ok_is_case_sensitive() {
        // "unpack OK" should not be recognized (git is case-sensitive here)
        let response = b"unpack OK\nok refs/heads/main\n";
        let result = parse_v1(response);
        // "unpack OK" starts with "unpack " so it should be parsed as a failure
        // with reason "OK"
        let (status, _refs) = result.expect("valid response");
        assert!(!status.is_ok(), "unpack OK (capital) should be treated as error");
        match &status {
            UnpackStatus::Failed { reason } => {
                assert_eq!(reason, &BString::from("OK"));
            }
            _ => panic!("expected Failed"),
        }
    }
}

mod unpack_status {
    use super::*;

    #[test]
    fn ok_is_ok() {
        assert!(UnpackStatus::Ok.is_ok());
    }

    #[test]
    fn failed_is_not_ok() {
        let status = UnpackStatus::Failed { reason: "error".into() };
        assert!(!status.is_ok());
    }
}

mod status_v1 {
    use super::*;

    #[test]
    fn ok_is_ok() {
        let status = StatusV1::Ok {
            ref_name: "refs/heads/main".into(),
        };
        assert!(status.is_ok());
        assert_eq!(status.ref_name(), "refs/heads/main");
    }

    #[test]
    fn ng_is_not_ok() {
        let status = StatusV1::Ng {
            ref_name: "refs/heads/main".into(),
            reason: "non-fast-forward".into(),
        };
        assert!(!status.is_ok());
        assert_eq!(status.ref_name(), "refs/heads/main");
    }

    #[test]
    fn ref_name_works_for_tag_refs() {
        let status = StatusV1::Ok {
            ref_name: "refs/tags/v2.0.0".into(),
        };
        assert_eq!(status.ref_name(), "refs/tags/v2.0.0");
    }

    #[test]
    fn ref_name_works_for_notes_refs() {
        let status = StatusV1::Ok {
            ref_name: "refs/notes/commits".into(),
        };
        assert_eq!(status.ref_name(), "refs/notes/commits");
    }

    #[test]
    fn equality_check() {
        let a = StatusV1::Ok {
            ref_name: "refs/heads/main".into(),
        };
        let b = StatusV1::Ok {
            ref_name: "refs/heads/main".into(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn ok_and_ng_are_not_equal() {
        let ok = StatusV1::Ok {
            ref_name: "refs/heads/main".into(),
        };
        let ng = StatusV1::Ng {
            ref_name: "refs/heads/main".into(),
            reason: "error".into(),
        };
        assert_ne!(ok, ng);
    }
}

mod parse_v1_protocol_v2_patterns {
    use super::*;

    /// Ported from t5516: push with force-with-lease can produce specific ref status reasons.
    #[test]
    fn force_with_lease_rejected() {
        let response = b"unpack ok\nng refs/heads/main stale info\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 1);
        match &refs[0] {
            StatusV1::Ng { ref_name, reason } => {
                assert_eq!(ref_name, &BString::from("refs/heads/main"));
                assert_eq!(reason, &BString::from("stale info"));
            }
            _ => panic!("expected Ng"),
        }
    }

    /// Push response for wildcard refs (e.g., refs/heads/* matching).
    #[test]
    fn wildcard_ref_pattern_in_response() {
        let response = b"unpack ok\nok refs/heads/feature/a\nok refs/heads/feature/b\nok refs/heads/feature/c\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].ref_name(), "refs/heads/feature/a");
        assert_eq!(refs[1].ref_name(), "refs/heads/feature/b");
        assert_eq!(refs[2].ref_name(), "refs/heads/feature/c");
    }

    /// The MissingUnpackStatus error message should be descriptive.
    #[test]
    fn error_display_for_missing_unpack_status() {
        let err = parse_v1(b"").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unpack"),
            "error should mention missing unpack status: {msg}"
        );
    }
}

mod parse_v1_real_server_patterns {
    use super::*;

    #[test]
    fn large_push_with_many_refs() {
        // Simulate a push with many refs (e.g., mirror push)
        let mut response = b"unpack ok\n".to_vec();
        for i in 0..50 {
            response.extend_from_slice(format!("ok refs/heads/branch-{i}\n").as_bytes());
        }
        let (status, refs) = parse_v1(&response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 50);
        assert!(refs.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn hook_declined_with_detailed_reason() {
        // Git pre-receive hooks can return arbitrary text
        let response =
            b"unpack ok\nng refs/heads/main pre-receive hook declined: commit message must reference a JIRA ticket\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 1);
        match &refs[0] {
            StatusV1::Ng { reason, .. } => {
                assert_eq!(
                    reason,
                    &BString::from("pre-receive hook declined: commit message must reference a JIRA ticket")
                );
            }
            _ => panic!("expected Ng"),
        }
    }

    #[test]
    fn atomic_push_all_rejected() {
        // When atomic push fails, all refs are rejected
        let response = b"unpack ok\nng refs/heads/main atomic push failed\nng refs/heads/develop atomic push failed\n";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().all(|r| !r.is_ok()));
    }

    #[test]
    fn response_without_trailing_newline() {
        // Some servers might not include a trailing newline
        let response = b"unpack ok\nok refs/heads/main";
        let (status, refs) = parse_v1(response).expect("valid response");
        assert!(status.is_ok());
        assert_eq!(refs.len(), 1);
        assert!(refs[0].is_ok());
    }

    #[test]
    fn unpack_status_equality() {
        assert_eq!(UnpackStatus::Ok, UnpackStatus::Ok);
        assert_ne!(UnpackStatus::Ok, UnpackStatus::Failed { reason: "error".into() });
        let a = UnpackStatus::Failed {
            reason: "disk full".into(),
        };
        let b = UnpackStatus::Failed {
            reason: "disk full".into(),
        };
        assert_eq!(a, b);
    }
}
