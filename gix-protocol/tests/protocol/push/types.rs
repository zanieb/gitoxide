use gix_protocol::push::{Command, Options};

/// Tests for push command types, ported from git push protocol specification
/// and git/t/t5516-fetch-push.sh patterns.

mod command {
    use super::*;

    fn null_id() -> gix_hash::ObjectId {
        gix_hash::ObjectId::null(gix_hash::Kind::Sha1)
    }

    fn id(hex: &str) -> gix_hash::ObjectId {
        gix_hash::ObjectId::from_hex(hex.as_bytes()).expect("valid hex")
    }

    #[test]
    fn new_creates_command() {
        let old = id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let new = id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let cmd = Command::new("refs/heads/main", old, new);
        assert_eq!(cmd.ref_name, "refs/heads/main");
        assert_eq!(cmd.old_id, old);
        assert_eq!(cmd.new_id, new);
    }

    #[test]
    fn is_delete_when_new_id_is_null() {
        // Corresponds to: git push origin :refs/heads/branch
        // From t5516: 'push deletion with negotiation'
        let old = id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let cmd = Command::new("refs/heads/branch", old, null_id());
        assert!(cmd.is_delete(), "command with null new_id should be a delete");
        assert!(!cmd.is_create(), "delete should not be a create");
    }

    #[test]
    fn is_create_when_old_id_is_null() {
        // Corresponds to: git push origin main (when remote doesn't have the ref)
        // From t5516: 'push without wildcard' (initial push to new remote)
        let new = id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let cmd = Command::new("refs/heads/main", null_id(), new);
        assert!(cmd.is_create(), "command with null old_id should be a create");
        assert!(!cmd.is_delete(), "create should not be a delete");
    }

    #[test]
    fn regular_update_is_neither_create_nor_delete() {
        // Corresponds to: git push origin main (fast-forward update)
        // From t5516: 'push updates local refs'
        let old = id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let new = id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let cmd = Command::new("refs/heads/main", old, new);
        assert!(!cmd.is_delete());
        assert!(!cmd.is_create());
    }

    #[test]
    fn force_push_looks_like_regular_update() {
        // Force push is the same as regular update at the command level;
        // the difference is in the capability negotiation (not in the Command type).
        // From t5516: 'push --force with matching heads'
        let old = id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let new = id("cccccccccccccccccccccccccccccccccccccccc");
        let cmd = Command::new("refs/heads/main", old, new);
        assert!(!cmd.is_delete());
        assert!(!cmd.is_create());
    }

    #[test]
    fn tag_ref_command() {
        // Corresponds to: git push origin v1.0
        // From t5516: 'push tag with non-existent, incomplete dest'
        let new = id("dddddddddddddddddddddddddddddddddddddddd");
        let cmd = Command::new("refs/tags/v1.0", null_id(), new);
        assert!(cmd.is_create());
        assert_eq!(cmd.ref_name, "refs/tags/v1.0");
    }

    #[test]
    fn ref_name_preserves_bytes() {
        // Ref names should preserve the exact bytes, not be converted through UTF-8
        let cmd = Command::new(
            bstr::BString::from(b"refs/heads/\xc0\xc1".as_ref()),
            null_id(),
            id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        );
        assert_eq!(&cmd.ref_name[..], b"refs/heads/\xc0\xc1");
    }
}

mod command_format {
    use super::*;

    fn null_id() -> gix_hash::ObjectId {
        gix_hash::ObjectId::null(gix_hash::Kind::Sha1)
    }

    fn id(hex: &str) -> gix_hash::ObjectId {
        gix_hash::ObjectId::from_hex(hex.as_bytes()).expect("valid hex")
    }

    #[test]
    fn command_line_format_matches_protocol_spec() {
        // The git push protocol sends: "<old-oid> <new-oid> <refname>"
        // Verify our Command fields produce the correct format when formatted.
        let old = id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let new = id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let cmd = Command::new("refs/heads/main", old, new);
        let line = format!("{} {} {}", cmd.old_id, cmd.new_id, cmd.ref_name);
        assert_eq!(
            line,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb refs/heads/main"
        );
    }

    #[test]
    fn delete_command_line_has_null_new_id() {
        let old = id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let cmd = Command::new("refs/heads/branch", old, null_id());
        let line = format!("{} {} {}", cmd.old_id, cmd.new_id, cmd.ref_name);
        assert_eq!(
            line,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 0000000000000000000000000000000000000000 refs/heads/branch"
        );
    }

    #[test]
    fn create_command_line_has_null_old_id() {
        let new = id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let cmd = Command::new("refs/heads/new-branch", null_id(), new);
        let line = format!("{} {} {}", cmd.old_id, cmd.new_id, cmd.ref_name);
        assert_eq!(
            line,
            "0000000000000000000000000000000000000000 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb refs/heads/new-branch"
        );
    }

    #[test]
    fn multiple_commands_format() {
        // Simulate formatting multiple commands as sent in push protocol
        let old = id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let new = id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let commands = vec![
            Command::new("refs/heads/main", old, new),
            Command::new("refs/heads/develop", old, new),
            Command::new("refs/tags/v1.0", null_id(), new),
        ];
        assert_eq!(commands.len(), 3);
        assert!(!commands[0].is_create());
        assert!(!commands[0].is_delete());
        assert!(commands[2].is_create());
    }
}

mod options {
    use super::*;

    #[test]
    fn default_options() {
        let opts = Options::default();
        assert!(!opts.dry_run, "dry_run should default to false");
        assert!(!opts.atomic, "atomic should default to false");
    }

    #[test]
    fn dry_run_option() {
        // Corresponds to: git push --dry-run
        // From t5516: 'push with dry-run'
        let opts = Options {
            dry_run: true,
            atomic: false,
        };
        assert!(opts.dry_run);
    }

    #[test]
    fn atomic_option() {
        // Corresponds to: git push --atomic
        let opts = Options {
            dry_run: false,
            atomic: true,
        };
        assert!(opts.atomic);
    }

    #[test]
    fn both_options_set() {
        // git push --dry-run --atomic
        let opts = Options {
            dry_run: true,
            atomic: true,
        };
        assert!(opts.dry_run);
        assert!(opts.atomic);
    }
}

mod outcome {
    use gix_protocol::push::{
        response::{StatusV1, UnpackStatus},
        Outcome,
    };

    #[test]
    fn successful_push_outcome() {
        let outcome = Outcome {
            unpack_status: UnpackStatus::Ok,
            ref_updates: vec![StatusV1::Ok {
                ref_name: "refs/heads/main".into(),
            }],
        };
        assert!(outcome.unpack_status.is_ok());
        assert_eq!(outcome.ref_updates.len(), 1);
        assert!(outcome.ref_updates[0].is_ok());
    }

    #[test]
    fn failed_unpack_outcome() {
        let outcome = Outcome {
            unpack_status: UnpackStatus::Failed {
                reason: "disk full".into(),
            },
            ref_updates: vec![StatusV1::Ng {
                ref_name: "refs/heads/main".into(),
                reason: "unpacker error".into(),
            }],
        };
        assert!(!outcome.unpack_status.is_ok());
        assert!(!outcome.ref_updates[0].is_ok());
    }

    #[test]
    fn mixed_ref_updates_in_outcome() {
        // Simulates a push where some refs succeed and some fail
        let outcome = Outcome {
            unpack_status: UnpackStatus::Ok,
            ref_updates: vec![
                StatusV1::Ok {
                    ref_name: "refs/heads/main".into(),
                },
                StatusV1::Ng {
                    ref_name: "refs/heads/protected".into(),
                    reason: "hook declined".into(),
                },
                StatusV1::Ok {
                    ref_name: "refs/tags/v1.0".into(),
                },
            ],
        };
        assert!(outcome.unpack_status.is_ok());
        assert_eq!(outcome.ref_updates.len(), 3);
        let ok_count = outcome.ref_updates.iter().filter(|r| r.is_ok()).count();
        let ng_count = outcome.ref_updates.iter().filter(|r| !r.is_ok()).count();
        assert_eq!(ok_count, 2);
        assert_eq!(ng_count, 1);
    }

    #[test]
    fn empty_ref_updates_in_outcome() {
        // Edge case: outcome with no ref updates (shouldn't happen in practice,
        // but the type allows it)
        let outcome = Outcome {
            unpack_status: UnpackStatus::Ok,
            ref_updates: vec![],
        };
        assert!(outcome.unpack_status.is_ok());
        assert!(outcome.ref_updates.is_empty());
    }
}

mod progress_id {
    use gix_protocol::push::ProgressId;

    #[test]
    fn progress_ids_are_unique_4byte_codes() {
        let remote: gix_features::progress::Id = ProgressId::RemoteProgress.into();
        let sending: gix_features::progress::Id = ProgressId::SendingPack.into();
        let counting: gix_features::progress::Id = ProgressId::CountingObjects.into();

        // Each progress ID should be unique
        assert_ne!(remote, sending);
        assert_ne!(remote, counting);
        assert_ne!(sending, counting);
    }

    #[test]
    fn progress_ids_are_readable_4byte_ascii() {
        // The progress IDs should be readable ASCII for debugging
        let remote: gix_features::progress::Id = ProgressId::RemoteProgress.into();
        let sending: gix_features::progress::Id = ProgressId::SendingPack.into();
        let counting: gix_features::progress::Id = ProgressId::CountingObjects.into();

        assert_eq!(&remote, b"PURP");
        assert_eq!(&sending, b"PUSP");
        assert_eq!(&counting, b"PUCO");
    }
}

mod command_properties {
    use super::*;

    fn null_id() -> gix_hash::ObjectId {
        gix_hash::ObjectId::null(gix_hash::Kind::Sha1)
    }

    fn id(hex: &str) -> gix_hash::ObjectId {
        gix_hash::ObjectId::from_hex(hex.as_bytes()).expect("valid hex")
    }

    /// Edge case: both old and new null - is_create and is_delete are based only on
    /// their respective fields, so both return true independently.
    #[test]
    fn both_null_is_create_and_delete() {
        let cmd = Command::new("refs/heads/main", null_id(), null_id());
        assert!(cmd.is_create(), "null old_id makes it a create");
        assert!(cmd.is_delete(), "null new_id makes it a delete");
    }

    /// Command can be cloned and compared.
    #[test]
    fn clone_and_equality() {
        let old = id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let new = id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let cmd = Command::new("refs/heads/main", old, new);
        let cloned = cmd.clone();
        assert_eq!(cmd, cloned);
    }

    /// Command debug format includes all fields.
    #[test]
    fn debug_output_is_meaningful() {
        let old = id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let new = id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let cmd = Command::new("refs/heads/main", old, new);
        let debug = format!("{cmd:?}");
        assert!(
            debug.contains("refs/heads/main"),
            "debug should contain ref name: {debug}"
        );
    }
}

#[cfg(any(feature = "blocking-client", feature = "async-client"))]
mod error {
    use gix_protocol::push;
    use gix_transport::IsSpuriousError;

    #[test]
    fn io_connection_reset_is_spurious() {
        let err = push::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        ));
        assert!(err.is_spurious(), "connection reset should be spurious for retry logic");
    }

    #[test]
    fn io_connection_aborted_is_spurious() {
        let err = push::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "connection aborted",
        ));
        assert!(
            err.is_spurious(),
            "connection aborted should be spurious for retry logic"
        );
    }

    #[test]
    fn io_other_error_is_not_spurious() {
        let err = push::Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        ));
        assert!(!err.is_spurious(), "permission denied should not be spurious");
    }

    #[test]
    fn unpack_failed_is_not_spurious() {
        let err = push::Error::UnpackFailed {
            reason: "index-pack failed".to_string(),
        };
        assert!(
            !err.is_spurious(),
            "unpack failure is a server-side issue, not spurious"
        );
    }

    #[test]
    fn atomic_not_supported_is_not_spurious() {
        let err = push::Error::AtomicNotSupported;
        assert!(!err.is_spurious(), "capability mismatch is not spurious");
    }

    #[test]
    fn no_commands_is_not_spurious() {
        let err = push::Error::NoCommands;
        assert!(!err.is_spurious(), "no commands is a client-side issue");
    }

    #[test]
    fn pack_generation_is_not_spurious() {
        let err = push::Error::PackGeneration(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "pack error")));
        assert!(!err.is_spurious(), "pack generation failure is not spurious");
    }
}
