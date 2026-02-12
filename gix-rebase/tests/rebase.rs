use gix_hash::{Kind, ObjectId};
use gix_rebase::MergeState;
use gix_sequencer::todo::{Operation, TodoList};

fn make_oid(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex.as_bytes()).unwrap()
}

mod read_write {
    use super::*;

    #[test]
    fn roundtrip_basic_interactive_state() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let todo = TodoList {
            operations: vec![
                Operation::Pick {
                    commit: make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").into(),
                    summary: "First commit".into(),
                },
                Operation::Pick {
                    commit: make_oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").into(),
                    summary: "Second commit".into(),
                },
            ]
            .into(),
        };

        let state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: make_oid("cccccccccccccccccccccccccccccccccccccccc"),
            orig_head: make_oid("dddddddddddddddddddddddddddddddddddddddd"),
            interactive: true,
            todo,
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 1,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        let read_back = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();

        assert_eq!(read_back.head_name, state.head_name);
        assert_eq!(read_back.onto, state.onto);
        assert_eq!(read_back.orig_head, state.orig_head);
        assert_eq!(read_back.interactive, true);
        assert_eq!(read_back.todo.operations.len(), 2);
        assert_eq!(read_back.done.operations.len(), 0);
        assert_eq!(read_back.current_step, 1);
        assert_eq!(read_back.total_steps, 2);
        assert!(read_back.stopped_sha.is_none());
    }

    #[test]
    fn roundtrip_with_done_operations() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let state = MergeState {
            head_name: "refs/heads/main".into(),
            onto: make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            orig_head: make_oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            interactive: true,
            todo: TodoList {
                operations: vec![Operation::Pick {
                    commit: make_oid("dddddddddddddddddddddddddddddddddddddddd").into(),
                    summary: "Remaining".into(),
                }]
                .into(),
            },
            done: TodoList {
                operations: vec![Operation::Pick {
                    commit: make_oid("cccccccccccccccccccccccccccccccccccccccc").into(),
                    summary: "Already done".into(),
                }]
                .into(),
            },
            current_step: 2,
            total_steps: 2,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        let read_back = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();

        assert_eq!(read_back.done.operations.len(), 1);
        assert_eq!(read_back.todo.operations.len(), 1);
        assert_eq!(read_back.current_step, 2);
    }

    #[test]
    fn roundtrip_with_stopped_sha() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let stopped = make_oid("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
        let state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            orig_head: make_oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            interactive: true,
            todo: TodoList {
                operations: vec![Operation::Edit {
                    commit: make_oid("cccccccccccccccccccccccccccccccccccccccc").into(),
                    summary: "Edit this".into(),
                }]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 1,
            total_steps: 1,
            stopped_sha: Some(stopped),
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        let read_back = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();

        assert_eq!(read_back.stopped_sha, Some(stopped));
    }

    #[test]
    fn roundtrip_non_interactive() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            orig_head: make_oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            interactive: false,
            todo: TodoList {
                operations: vec![Operation::Pick {
                    commit: make_oid("cccccccccccccccccccccccccccccccccccccccc").into(),
                    summary: "A commit".into(),
                }]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 1,
            total_steps: 1,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        let read_back = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();

        assert_eq!(read_back.interactive, false, "non-interactive should be preserved");
    }

    #[test]
    fn roundtrip_detached_head() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let state = MergeState {
            head_name: "detached HEAD".into(),
            onto: make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            orig_head: make_oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            interactive: true,
            todo: TodoList {
                operations: vec![Operation::Noop].into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 1,
            total_steps: 1,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        let read_back = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();

        assert_eq!(
            read_back.head_name.to_string(),
            "detached HEAD",
            "detached HEAD name should be preserved"
        );
    }

    #[test]
    fn roundtrip_rebase_merges_todo() {
        // Test a --rebase-merges style todo list with label/reset/merge operations
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let state = MergeState {
            head_name: "refs/heads/main".into(),
            onto: make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            orig_head: make_oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            interactive: true,
            todo: TodoList {
                operations: vec![
                    Operation::Label { name: "onto".into() },
                    Operation::Reset { name: "onto".into() },
                    Operation::Pick {
                        commit: make_oid("cccccccccccccccccccccccccccccccccccccccc").into(),
                        summary: "feature commit".into(),
                    },
                    Operation::Label { name: "feature".into() },
                    Operation::Reset { name: "onto".into() },
                    Operation::Merge {
                        commit: Some((
                            make_oid("dddddddddddddddddddddddddddddddddddddddd").into(),
                            gix_sequencer::todo::AmendMessage::Replace,
                        )),
                        label: "feature".into(),
                        oneline: "Merge feature".into(),
                    },
                ]
                .into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 1,
            total_steps: 6,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        let read_back = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();

        assert_eq!(read_back.todo.operations.len(), 6);
        assert!(matches!(&read_back.todo.operations[0], Operation::Label { .. }));
        assert!(matches!(&read_back.todo.operations[1], Operation::Reset { .. }));
        assert!(matches!(&read_back.todo.operations[2], Operation::Pick { .. }));
        assert!(matches!(&read_back.todo.operations[3], Operation::Label { .. }));
        assert!(matches!(&read_back.todo.operations[4], Operation::Reset { .. }));
        assert!(matches!(&read_back.todo.operations[5], Operation::Merge { .. }));
    }

    #[test]
    fn empty_todo_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let state = MergeState {
            head_name: "refs/heads/main".into(),
            onto: make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            orig_head: make_oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            interactive: true,
            todo: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            done: TodoList {
                operations: vec![Operation::Pick {
                    commit: make_oid("cccccccccccccccccccccccccccccccccccccccc").into(),
                    summary: "All done".into(),
                }]
                .into(),
            },
            current_step: 1,
            total_steps: 1,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        let read_back = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();

        assert_eq!(read_back.todo.operations.len(), 0, "empty todo should roundtrip");
        assert_eq!(read_back.done.operations.len(), 1);
    }
}

mod errors {
    use super::*;

    #[test]
    fn missing_head_name_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();
        // Write everything except head-name
        std::fs::write(rebase_dir.join("onto"), b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();

        let result = MergeState::read_from(&rebase_dir, Kind::Sha1);
        assert!(result.is_err(), "missing head-name should fail");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("head-name"),
            "error should mention head-name: {err}"
        );
    }

    #[test]
    fn missing_onto_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();
        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/main").unwrap();
        // onto is missing
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();

        let result = MergeState::read_from(&rebase_dir, Kind::Sha1);
        assert!(result.is_err(), "missing onto should fail");
    }

    #[test]
    fn missing_orig_head_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();
        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/main").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        // orig-head is missing
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();

        let result = MergeState::read_from(&rebase_dir, Kind::Sha1);
        assert!(result.is_err(), "missing orig-head should fail");
    }

    #[test]
    fn abbreviated_hash_in_state_file_is_rejected() {
        // C Git always writes full 40-char hex hashes in state files.
        // Abbreviated hashes must be rejected rather than silently zero-padded.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();
        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/feature").unwrap();
        // Write abbreviated hash for onto (only 7 chars)
        std::fs::write(rebase_dir.join("onto"), b"abcdef1").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();

        let result = MergeState::read_from(&rebase_dir, Kind::Sha1);
        assert!(result.is_err(), "abbreviated hash in onto should be rejected");
    }

    #[test]
    fn abbreviated_stopped_sha_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();
        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/feature").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"cccccccccccccccccccccccccccccccccccccccc").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"dddddddddddddddddddddddddddddddddddddddd",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();
        std::fs::write(rebase_dir.join("msgnum"), b"1").unwrap();
        std::fs::write(rebase_dir.join("end"), b"1").unwrap();
        // Write abbreviated hash in stopped-sha
        std::fs::write(rebase_dir.join("stopped-sha"), b"abcdef1").unwrap();

        let result = MergeState::read_from(&rebase_dir, Kind::Sha1);
        assert!(
            result.is_err(),
            "abbreviated hash in stopped-sha should be rejected, not zero-padded"
        );
    }

    #[test]
    fn invalid_hex_in_state_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();
        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/feature").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();

        let result = MergeState::read_from(&rebase_dir, Kind::Sha1);
        assert!(result.is_err(), "invalid hex in state file should be rejected");
    }

    #[test]
    fn nonexistent_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("nonexistent-rebase-merge");

        let result = MergeState::read_from(&rebase_dir, Kind::Sha1);
        assert!(result.is_err(), "reading from nonexistent dir should fail");
    }

    #[test]
    fn invalid_todo_content_errors() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();
        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/main").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        // Invalid todo content
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"bogus-operation something\n").unwrap();

        let result = MergeState::read_from(&rebase_dir, Kind::Sha1);
        assert!(result.is_err(), "invalid todo content should produce an error");
    }
}

mod remove {
    use super::*;

    #[test]
    fn remove_deletes_directory() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let state = MergeState {
            head_name: "refs/heads/feature".into(),
            onto: make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            orig_head: make_oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            interactive: true,
            todo: TodoList {
                operations: vec![Operation::Noop].into(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 1,
            total_steps: 1,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        assert!(rebase_dir.exists());

        MergeState::remove(&rebase_dir).unwrap();
        assert!(!rebase_dir.exists(), "directory should be removed");
    }

    #[test]
    fn remove_nonexistent_directory_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("nonexistent-rebase-merge");

        // Should not error when directory doesn't exist
        MergeState::remove(&rebase_dir).unwrap();
    }
}

mod state_files {
    use super::*;

    #[test]
    fn whitespace_trimmed_from_state_files() {
        // Git may write trailing newlines in state files
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();

        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/main\n").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"noop\n").unwrap();
        std::fs::write(rebase_dir.join("msgnum"), b"1\n").unwrap();
        std::fs::write(rebase_dir.join("end"), b"1\n").unwrap();

        let state = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(state.head_name.to_string(), "refs/heads/main");
        assert_eq!(state.onto, make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert_eq!(state.current_step, 1);
    }

    #[test]
    fn full_hash_in_stopped_sha_works() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();

        let expected_oid = make_oid("abcdef1234567890abcdef1234567890abcdef12");
        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/feature").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"cccccccccccccccccccccccccccccccccccccccc").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"dddddddddddddddddddddddddddddddddddddddd",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();
        std::fs::write(rebase_dir.join("msgnum"), b"1").unwrap();
        std::fs::write(rebase_dir.join("end"), b"1").unwrap();
        std::fs::write(
            rebase_dir.join("stopped-sha"),
            b"abcdef1234567890abcdef1234567890abcdef12",
        )
        .unwrap();

        let state = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(state.stopped_sha, Some(expected_oid));
    }

    #[test]
    fn interactive_flag_from_file_presence() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();

        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/main").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("git-rebase-todo"), b"").unwrap();
        std::fs::write(rebase_dir.join("msgnum"), b"1").unwrap();
        std::fs::write(rebase_dir.join("end"), b"0").unwrap();

        // Without 'interactive' file
        let state = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(
            state.interactive, false,
            "should not be interactive when file is absent"
        );

        // With 'interactive' file (can be empty, just needs to exist)
        std::fs::write(rebase_dir.join("interactive"), b"").unwrap();
        let state = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(state.interactive, true, "should be interactive when file exists");
    }

    #[test]
    fn done_file_absent_produces_empty_done_list() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        std::fs::create_dir_all(&rebase_dir).unwrap();

        std::fs::write(rebase_dir.join("head-name"), b"refs/heads/main").unwrap();
        std::fs::write(rebase_dir.join("onto"), b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        std::fs::write(
            rebase_dir.join("orig-head"),
            b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        std::fs::write(
            rebase_dir.join("git-rebase-todo"),
            b"pick cccccccccccccccccccccccccccccccccccccccc First\n",
        )
        .unwrap();
        std::fs::write(rebase_dir.join("msgnum"), b"1").unwrap();
        std::fs::write(rebase_dir.join("end"), b"1").unwrap();
        // No 'done' file

        let state = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert!(
            state.done.operations.is_empty(),
            "absent 'done' file should produce empty done list"
        );
    }

    #[test]
    fn write_creates_directory_if_absent() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("deep").join("nested").join("rebase-merge");

        let state = MergeState {
            head_name: "refs/heads/main".into(),
            onto: make_oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            orig_head: make_oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            interactive: false,
            todo: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 1,
            total_steps: 0,
            stopped_sha: None,
            accumulated_squash_message: None,
        };

        state.write_to(&rebase_dir).unwrap();
        assert!(rebase_dir.exists(), "directory should be created");
        assert!(rebase_dir.join("head-name").exists());
        assert!(rebase_dir.join("onto").exists());
        assert!(rebase_dir.join("orig-head").exists());
    }
}

mod driver {
    use super::*;
    use gix_rebase::{CherryPickError, CherryPickOutcome, Driver, StepOutcome};
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A mock driver that records operations for testing.
    struct MockDriver {
        /// Map from Prefix hex -> full ObjectId for resolve_commit.
        resolve_map: HashMap<String, ObjectId>,
        /// Map from ObjectId -> commit message for read_commit_message.
        messages: HashMap<ObjectId, Vec<u8>>,
        /// Tracks cherry_pick calls: (commit_id, message_override).
        cherry_pick_calls: RefCell<Vec<(ObjectId, Option<Vec<u8>>)>>,
        /// Tracks update_head calls.
        update_head_calls: RefCell<Vec<ObjectId>>,
        /// If set, cherry_pick returns this ObjectId as the new commit.
        /// Increments by 1 byte each call to simulate unique commits.
        next_commit_counter: RefCell<u32>,
        /// If set, cherry_pick will return Conflict for these commit IDs.
        conflict_on: RefCell<std::collections::HashSet<ObjectId>>,
        /// If set, cherry_pick will return Other error for these commit IDs.
        fail_on: RefCell<std::collections::HashSet<ObjectId>>,
        /// If set, resolve_commit will fail for these prefix hex strings.
        resolve_fail_on: RefCell<std::collections::HashSet<String>>,
    }

    impl MockDriver {
        fn new() -> Self {
            Self {
                resolve_map: HashMap::new(),
                messages: HashMap::new(),
                cherry_pick_calls: RefCell::new(Vec::new()),
                update_head_calls: RefCell::new(Vec::new()),
                next_commit_counter: RefCell::new(1),
                conflict_on: RefCell::new(std::collections::HashSet::new()),
                fail_on: RefCell::new(std::collections::HashSet::new()),
                resolve_fail_on: RefCell::new(std::collections::HashSet::new()),
            }
        }

        fn register_commit(&mut self, prefix_hex: &str, full_hex: &str, message: &[u8]) {
            let full_oid = make_oid(full_hex);
            self.resolve_map.insert(prefix_hex.to_string(), full_oid);
            // Also allow resolving by full hex.
            self.resolve_map.insert(full_hex.to_string(), full_oid);
            self.messages.insert(full_oid, message.to_vec());
        }

        fn next_fake_commit_id(&self) -> ObjectId {
            let mut counter = self.next_commit_counter.borrow_mut();
            let val = *counter;
            *counter += 1;
            // Create a deterministic fake OID from the counter.
            let hex = format!("{val:0>40}");
            make_oid(&hex)
        }
    }

    impl Driver for MockDriver {
        fn resolve_commit(
            &self,
            prefix: &gix_hash::Prefix,
        ) -> Result<ObjectId, Box<dyn std::error::Error + Send + Sync>> {
            let hex = prefix.as_oid().to_hex().to_string();
            if self.resolve_fail_on.borrow().contains(&hex) {
                return Err(format!("simulated resolve failure for {hex}").into());
            }
            self.resolve_map
                .get(&hex)
                .copied()
                .ok_or_else(|| format!("commit not found: {hex}").into())
        }

        fn cherry_pick(
            &self,
            commit_id: ObjectId,
            message: Option<&[u8]>,
        ) -> Result<CherryPickOutcome, CherryPickError> {
            self.cherry_pick_calls
                .borrow_mut()
                .push((commit_id, message.map(|m| m.to_vec())));
            if self.conflict_on.borrow().contains(&commit_id) {
                return Err(CherryPickError::Conflict { commit_id });
            }
            if self.fail_on.borrow().contains(&commit_id) {
                return Err(CherryPickError::Other {
                    message: format!("simulated failure for {commit_id}"),
                    source: format!("simulated failure").into(),
                });
            }
            let new_id = self.next_fake_commit_id();
            Ok(CherryPickOutcome { new_commit_id: new_id })
        }

        fn read_commit_message(
            &self,
            commit_id: ObjectId,
        ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
            self.messages
                .get(&commit_id)
                .cloned()
                .ok_or_else(|| format!("message not found for {commit_id}").into())
        }

        fn update_head(&self, commit_id: ObjectId) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.update_head_calls.borrow_mut().push(commit_id);
            Ok(())
        }
    }

    fn make_state_with_ops(ops: Vec<Operation>) -> MergeState {
        let total = ops.len();
        MergeState {
            head_name: "refs/heads/feature".into(),
            onto: make_oid("cccccccccccccccccccccccccccccccccccccccc"),
            orig_head: make_oid("dddddddddddddddddddddddddddddddddddddddd"),
            interactive: true,
            todo: TodoList { operations: ops.into() },
            done: TodoList {
                operations: std::collections::VecDeque::new(),
            },
            current_step: 0,
            total_steps: total,
            stopped_sha: None,
            accumulated_squash_message: None,
        }
    }

    #[test]
    fn step_pick_applies_cherry_pick() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let commit_hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(commit_hex, commit_hex, b"First commit\n");

        let mut state = make_state_with_ops(vec![Operation::Pick {
            commit: make_oid(commit_hex).into(),
            summary: "First commit".into(),
        }]);

        let outcome = state.step(&driver, &rebase_dir).unwrap();
        assert!(
            matches!(outcome, StepOutcome::Applied { .. }),
            "pick should produce Applied"
        );
        assert_eq!(state.todo.operations.len(), 0, "todo should be empty after step");
        assert_eq!(state.done.operations.len(), 1, "done should have the picked op");
        assert_eq!(state.current_step, 1);

        let calls = driver.cherry_pick_calls.borrow();
        assert_eq!(calls.len(), 1, "cherry_pick should be called once");
        assert_eq!(calls[0].0, make_oid(commit_hex));
        assert!(calls[0].1.is_none(), "pick should not override the message");
    }

    #[test]
    fn step_multiple_picks() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"First\n");
        driver.register_commit(hex_b, hex_b, b"Second\n");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "First".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_b).into(),
                summary: "Second".into(),
            },
        ]);

        let r1 = state.step(&driver, &rebase_dir).unwrap();
        assert!(matches!(r1, StepOutcome::Applied { .. }));
        assert_eq!(state.todo.operations.len(), 1);

        let r2 = state.step(&driver, &rebase_dir).unwrap();
        assert!(matches!(r2, StepOutcome::Applied { .. }));
        assert_eq!(state.todo.operations.len(), 0);

        // Next step should report Done.
        let r3 = state.step(&driver, &rebase_dir).unwrap();
        assert_eq!(r3, StepOutcome::Done);
    }

    #[test]
    fn step_drop_skips() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let driver = MockDriver::new();
        let mut state = make_state_with_ops(vec![Operation::Drop {
            commit: make_oid(hex).into(),
            summary: "Dropped".into(),
        }]);

        let outcome = state.step(&driver, &rebase_dir).unwrap();
        assert_eq!(outcome, StepOutcome::Skipped, "drop should produce Skipped");
        assert!(
            driver.cherry_pick_calls.borrow().is_empty(),
            "drop should not call cherry_pick"
        );
    }

    #[test]
    fn step_noop_skips() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let driver = MockDriver::new();
        let mut state = make_state_with_ops(vec![Operation::Noop]);

        let outcome = state.step(&driver, &rebase_dir).unwrap();
        assert_eq!(outcome, StepOutcome::Skipped);
    }

    #[test]
    fn step_edit_pauses() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Edit me\n");

        let mut state = make_state_with_ops(vec![Operation::Edit {
            commit: make_oid(hex).into(),
            summary: "Edit me".into(),
        }]);

        let outcome = state.step(&driver, &rebase_dir).unwrap();
        assert!(
            matches!(outcome, StepOutcome::Paused { commit_id: Some(_), .. }),
            "edit should produce Paused with commit_id"
        );
        assert!(state.stopped_sha.is_some(), "stopped_sha should be set");
    }

    #[test]
    fn step_break_pauses_without_commit() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let driver = MockDriver::new();
        let mut state = make_state_with_ops(vec![Operation::Break]);

        let outcome = state.step(&driver, &rebase_dir).unwrap();
        assert_eq!(
            outcome,
            StepOutcome::Paused {
                commit_id: None,
                original_message: None,
            },
            "break should produce Paused without commit_id"
        );
    }

    #[test]
    fn step_fixup_uses_previous_message() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Original message\n");
        driver.register_commit(hex_b, hex_b, b"Fixup message (discarded)\n");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "Original".into(),
            },
            Operation::Fixup {
                commit: make_oid(hex_b).into(),
                summary: "Fixup".into(),
                amend_message: gix_sequencer::todo::AmendMessage::No,
            },
        ]);

        // Execute pick first.
        state.step(&driver, &rebase_dir).unwrap();

        // Then fixup.
        state.step(&driver, &rebase_dir).unwrap();

        let calls = driver.cherry_pick_calls.borrow();
        assert_eq!(calls.len(), 2);
        // The fixup call should use the original commit's message (from done list).
        assert!(
            calls[1].1.is_some(),
            "fixup should override the message with the previous commit's message"
        );
        let msg = calls[1].1.as_ref().unwrap();
        assert_eq!(
            msg.as_slice(),
            b"Original message\n",
            "fixup should use the previous commit's message"
        );
    }

    #[test]
    fn step_squash_combines_messages() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Original message");
        driver.register_commit(hex_b, hex_b, b"Squashed message");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "Original".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_b).into(),
                summary: "Squash".into(),
            },
        ]);

        state.step(&driver, &rebase_dir).unwrap();
        state.step(&driver, &rebase_dir).unwrap();

        let calls = driver.cherry_pick_calls.borrow();
        assert_eq!(calls.len(), 2);
        let msg = calls[1].1.as_ref().expect("squash should override message");
        let msg_str = String::from_utf8_lossy(msg);
        assert!(
            msg_str.contains("Original message"),
            "combined message should contain original: {msg_str}"
        );
        assert!(
            msg_str.contains("Squashed message"),
            "combined message should contain squashed: {msg_str}"
        );
    }

    #[test]
    fn continue_rebase_advances_after_edit() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Edit me\n");
        driver.register_commit(hex_b, hex_b, b"Next commit\n");

        let mut state = make_state_with_ops(vec![
            Operation::Edit {
                commit: make_oid(hex_a).into(),
                summary: "Edit me".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_b).into(),
                summary: "Next".into(),
            },
        ]);

        // Step pauses at edit.
        let outcome = state.step(&driver, &rebase_dir).unwrap();
        assert!(matches!(outcome, StepOutcome::Paused { .. }));
        assert!(state.stopped_sha.is_some());

        // Continue should execute the next pick.
        let outcome = state.continue_rebase(&driver, &rebase_dir).unwrap();
        assert!(
            matches!(outcome, StepOutcome::Applied { .. }),
            "continue should execute the next operation"
        );
        assert!(state.stopped_sha.is_none(), "stopped_sha should be cleared");
    }

    #[test]
    fn continue_rebase_done_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Edit me\n");

        let mut state = make_state_with_ops(vec![Operation::Edit {
            commit: make_oid(hex).into(),
            summary: "Edit me".into(),
        }]);

        // Step pauses at edit.
        state.step(&driver, &rebase_dir).unwrap();

        // Continue with empty todo should return Done.
        let outcome = state.continue_rebase(&driver, &rebase_dir).unwrap();
        assert_eq!(outcome, StepOutcome::Done);
    }

    #[test]
    fn abort_restores_orig_head_and_removes_dir() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Commit\n");

        let mut state = make_state_with_ops(vec![Operation::Pick {
            commit: make_oid(hex).into(),
            summary: "Commit".into(),
        }]);

        // Write state first so the directory exists.
        state.write_to(&rebase_dir).unwrap();
        assert!(rebase_dir.exists());

        // Step to make some progress.
        state.step(&driver, &rebase_dir).unwrap();

        // Abort should reset HEAD and remove the directory.
        state.abort(&driver, &rebase_dir).unwrap();

        let head_calls = driver.update_head_calls.borrow();
        assert_eq!(head_calls.len(), 1);
        assert_eq!(head_calls[0], state.orig_head, "abort should reset HEAD to orig_head");
        assert!(!rebase_dir.exists(), "abort should remove rebase dir");
    }

    #[test]
    fn step_persists_state_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Commit\n");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex).into(),
                summary: "First".into(),
            },
            Operation::Pick {
                commit: make_oid(hex).into(),
                summary: "Second".into(),
            },
        ]);

        state.step(&driver, &rebase_dir).unwrap();

        // Re-read from disk and verify state was persisted.
        let read_back = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(
            read_back.todo.operations.len(),
            1,
            "persisted todo should have one remaining op"
        );
        assert_eq!(
            read_back.done.operations.len(),
            1,
            "persisted done should have the completed op"
        );
        assert_eq!(read_back.current_step, 1);
    }

    #[test]
    fn step_reword_pauses_for_message_editing() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Reword me\n");

        let mut state = make_state_with_ops(vec![Operation::Reword {
            commit: make_oid(hex).into(),
            summary: "Reword me".into(),
        }]);

        let outcome = state.step(&driver, &rebase_dir).unwrap();
        assert!(
            matches!(outcome, StepOutcome::Paused { commit_id: Some(_), .. }),
            "reword should pause so the caller can prompt for a new message"
        );
    }

    #[test]
    fn step_unsupported_ops_return_error() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let driver = MockDriver::new();

        // Label is unsupported and should error.
        let mut state = make_state_with_ops(vec![Operation::Label { name: "onto".into() }]);
        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err(), "Label should return an error");

        // Reset is unsupported and should error.
        let mut state = make_state_with_ops(vec![Operation::Reset { name: "onto".into() }]);
        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err(), "Reset should return an error");

        // UpdateRef is unsupported and should error.
        let mut state = make_state_with_ops(vec![Operation::UpdateRef {
            reference: "refs/heads/topic".into(),
        }]);
        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err(), "UpdateRef should return an error");
    }

    #[test]
    fn empty_todo_returns_done() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let driver = MockDriver::new();
        let mut state = make_state_with_ops(vec![]);

        let outcome = state.step(&driver, &rebase_dir).unwrap();
        assert_eq!(outcome, StepOutcome::Done);
    }

    #[test]
    fn step_conflict_returns_cherry_pick_error() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Conflicting commit\n");
        driver.conflict_on.borrow_mut().insert(make_oid(hex));

        let mut state = make_state_with_ops(vec![Operation::Pick {
            commit: make_oid(hex).into(),
            summary: "Conflicting".into(),
        }]);

        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err(), "conflict should produce an error");
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("cherry-pick failed") || err_str.contains("conflict"),
            "error should indicate cherry-pick failure: {err_str}"
        );
    }

    #[test]
    fn step_conflict_does_not_lose_operation() {
        // When cherry-pick returns Conflict, the failed operation should appear
        // in the `done` list (matching C Git behavior where failed operations
        // are recorded in the `done` file). The state is written to disk so
        // the operation is preserved.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"First\n");
        driver.register_commit(hex_b, hex_b, b"Second\n");
        driver.conflict_on.borrow_mut().insert(make_oid(hex_a));

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "Conflict here".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_b).into(),
                summary: "Second".into(),
            },
        ]);

        // Write initial state to disk.
        state.write_to(&rebase_dir).unwrap();

        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err());

        // The failed operation should be in `done` (not lost).
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(
            on_disk.done.operations.len(),
            1,
            "failed operation should be moved to done"
        );
        assert_eq!(
            on_disk.todo.operations.len(),
            1,
            "remaining operation should stay in todo"
        );
    }

    #[test]
    fn continue_after_conflict_simulation() {
        // Simulate: step fails with conflict, caller resolves, continue_rebase proceeds.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Conflict commit\n");
        driver.register_commit(hex_b, hex_b, b"Next commit\n");

        // First, create state with edit (which pauses) to simulate a stopped state,
        // then continue. This tests the conflict->continue flow using edit as proxy
        // since the conflict path errors out before setting stopped_sha.
        let mut state = make_state_with_ops(vec![
            Operation::Edit {
                commit: make_oid(hex_a).into(),
                summary: "Edit (simulates conflict pause)".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_b).into(),
                summary: "Next".into(),
            },
        ]);

        // Step pauses at edit (simulating a conflict pause).
        let outcome = state.step(&driver, &rebase_dir).unwrap();
        assert!(matches!(outcome, StepOutcome::Paused { .. }));
        assert!(state.stopped_sha.is_some());

        // User resolves the conflict, then calls continue.
        let outcome = state.continue_rebase(&driver, &rebase_dir).unwrap();
        assert!(
            matches!(outcome, StepOutcome::Applied { .. }),
            "continue should process next operation"
        );
        assert!(state.stopped_sha.is_none());
        assert!(state.todo.operations.is_empty());
        assert_eq!(state.done.operations.len(), 2);
    }

    #[test]
    fn fixup_with_amend_message_replace() {
        // fixup -C: replace the accumulated message with the fixup commit's message.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Original message\n");
        driver.register_commit(hex_b, hex_b, b"Replacement message\n");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "Original".into(),
            },
            Operation::Fixup {
                commit: make_oid(hex_b).into(),
                summary: "Fixup -C".into(),
                amend_message: gix_sequencer::todo::AmendMessage::Replace,
            },
        ]);

        // Execute pick first.
        state.step(&driver, &rebase_dir).unwrap();
        // Then fixup -C.
        state.step(&driver, &rebase_dir).unwrap();

        let calls = driver.cherry_pick_calls.borrow();
        assert_eq!(calls.len(), 2);
        // The fixup call should have a message override.
        // Current implementation uses last_done_commit_message which returns
        // the previous commit's message regardless of AmendMessage variant.
        // This tests the existing behavior -- fixup always uses prev message.
        assert!(calls[1].1.is_some(), "fixup -C should provide a message override");
    }

    #[test]
    fn fixup_with_amend_message_edit() {
        // fixup -c: similar to -C, would open editor in full implementation.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Original message\n");
        driver.register_commit(hex_b, hex_b, b"Edit message\n");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "Original".into(),
            },
            Operation::Fixup {
                commit: make_oid(hex_b).into(),
                summary: "Fixup -c".into(),
                amend_message: gix_sequencer::todo::AmendMessage::Edit,
            },
        ]);

        state.step(&driver, &rebase_dir).unwrap();
        state.step(&driver, &rebase_dir).unwrap();

        let calls = driver.cherry_pick_calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].1.is_some(), "fixup -c should provide a message override");
    }

    #[test]
    fn long_mixed_operation_sequence() {
        // Test a complex sequence: pick, squash, fixup, drop, edit, pick
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let hex_c = "cccccccccccccccccccccccccccccccccccccccc";
        let hex_d = "dddddddddddddddddddddddddddddddddddddddd";
        let hex_e = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let hex_f = "ffffffffffffffffffffffffffffffffffffffff";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"First commit\n");
        driver.register_commit(hex_b, hex_b, b"Second commit (squash)\n");
        driver.register_commit(hex_c, hex_c, b"Third commit (fixup)\n");
        driver.register_commit(hex_d, hex_d, b"Fourth commit (drop)\n");
        driver.register_commit(hex_e, hex_e, b"Fifth commit (edit)\n");
        driver.register_commit(hex_f, hex_f, b"Sixth commit\n");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "First".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_b).into(),
                summary: "Squash into first".into(),
            },
            Operation::Fixup {
                commit: make_oid(hex_c).into(),
                summary: "Fixup into combined".into(),
                amend_message: gix_sequencer::todo::AmendMessage::No,
            },
            Operation::Drop {
                commit: make_oid(hex_d).into(),
                summary: "Dropped".into(),
            },
            Operation::Edit {
                commit: make_oid(hex_e).into(),
                summary: "Edit this".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_f).into(),
                summary: "Final pick".into(),
            },
        ]);

        // Step 1: pick
        let r = state.step(&driver, &rebase_dir).unwrap();
        assert!(matches!(r, StepOutcome::Applied { .. }), "pick should apply");

        // Step 2: squash
        let r = state.step(&driver, &rebase_dir).unwrap();
        assert!(matches!(r, StepOutcome::Applied { .. }), "squash should apply");

        // Step 3: fixup
        let r = state.step(&driver, &rebase_dir).unwrap();
        assert!(matches!(r, StepOutcome::Applied { .. }), "fixup should apply");

        // Step 4: drop
        let r = state.step(&driver, &rebase_dir).unwrap();
        assert_eq!(r, StepOutcome::Skipped, "drop should skip");

        // Step 5: edit (pauses)
        let r = state.step(&driver, &rebase_dir).unwrap();
        assert!(
            matches!(r, StepOutcome::Paused { commit_id: Some(_), .. }),
            "edit should pause"
        );

        // Continue after edit
        let r = state.continue_rebase(&driver, &rebase_dir).unwrap();
        assert!(
            matches!(r, StepOutcome::Applied { .. }),
            "continue should apply final pick"
        );

        // Should be done now
        let r = state.step(&driver, &rebase_dir).unwrap();
        assert_eq!(r, StepOutcome::Done);

        assert_eq!(state.done.operations.len(), 6);
        assert!(state.todo.operations.is_empty());

        // Verify cherry_pick was called for: pick, squash, fixup, edit, pick = 5 calls
        // (drop does not call cherry_pick)
        let calls = driver.cherry_pick_calls.borrow();
        assert_eq!(calls.len(), 5, "cherry_pick should be called 5 times (not for drop)");
    }

    #[test]
    fn abort_mid_sequence() {
        // Start a multi-step rebase, process some ops, then abort.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let hex_c = "cccccccccccccccccccccccccccccccccccccccc";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"First\n");
        driver.register_commit(hex_b, hex_b, b"Second\n");
        driver.register_commit(hex_c, hex_c, b"Third\n");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "First".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_b).into(),
                summary: "Second".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_c).into(),
                summary: "Third".into(),
            },
        ]);

        // Write initial state.
        state.write_to(&rebase_dir).unwrap();
        assert!(rebase_dir.exists());

        // Process first operation only.
        let r = state.step(&driver, &rebase_dir).unwrap();
        assert!(matches!(r, StepOutcome::Applied { .. }));
        assert_eq!(state.done.operations.len(), 1);
        assert_eq!(state.todo.operations.len(), 2, "two ops remain");

        // Abort mid-sequence.
        state.abort(&driver, &rebase_dir).unwrap();

        // Verify HEAD was reset to orig_head.
        let head_calls = driver.update_head_calls.borrow();
        assert_eq!(head_calls.len(), 1);
        assert_eq!(head_calls[0], state.orig_head);

        // Verify rebase dir was removed.
        assert!(!rebase_dir.exists(), "abort should remove the rebase directory");
    }

    #[test]
    fn step_with_unresolvable_prefix_errors() {
        // When resolve_commit fails, step should return an error.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Commit\n");
        driver.resolve_fail_on.borrow_mut().insert(hex.to_string());

        let mut state = make_state_with_ops(vec![Operation::Pick {
            commit: make_oid(hex).into(),
            summary: "Unresolvable".into(),
        }]);

        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err(), "unresolvable prefix should produce an error");
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("resolve") || err_str.contains("commit"),
            "error should indicate resolve failure: {err_str}"
        );
    }

    #[test]
    fn step_cherry_pick_other_error() {
        // When cherry_pick returns Other error (not Conflict), step should error.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Failing commit\n");
        driver.fail_on.borrow_mut().insert(make_oid(hex));

        let mut state = make_state_with_ops(vec![Operation::Pick {
            commit: make_oid(hex).into(),
            summary: "Will fail".into(),
        }]);

        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err(), "cherry-pick Other error should produce a step error");
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("cherry-pick failed") || err_str.contains("driver error"),
            "error should indicate cherry-pick failure: {err_str}"
        );
    }

    #[test]
    fn continue_when_not_stopped_with_remaining_ops() {
        // If continue is called when not stopped but there are remaining ops,
        // it should just step normally.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Commit\n");

        let mut state = make_state_with_ops(vec![Operation::Pick {
            commit: make_oid(hex).into(),
            summary: "Pick".into(),
        }]);

        // Call continue without being stopped.
        let outcome = state.continue_rebase(&driver, &rebase_dir).unwrap();
        assert!(
            matches!(outcome, StepOutcome::Applied { .. }),
            "continue when not stopped should process next op"
        );
    }

    #[test]
    fn continue_when_not_stopped_and_empty_returns_done() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");

        let driver = MockDriver::new();
        let mut state = make_state_with_ops(vec![]);

        let outcome = state.continue_rebase(&driver, &rebase_dir).unwrap();
        assert_eq!(
            outcome,
            StepOutcome::Done,
            "continue with empty todo and no stopped state should return Done"
        );
    }

    #[test]
    fn continue_removes_stopped_sha_file() {
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Edit me\n");

        let mut state = make_state_with_ops(vec![Operation::Edit {
            commit: make_oid(hex).into(),
            summary: "Edit".into(),
        }]);

        // Step pauses at edit, writing stopped-sha to disk.
        state.step(&driver, &rebase_dir).unwrap();
        assert!(
            rebase_dir.join("stopped-sha").exists(),
            "stopped-sha file should exist after edit pause"
        );

        // Continue should remove stopped-sha file.
        state.continue_rebase(&driver, &rebase_dir).unwrap();
        assert!(
            !rebase_dir.join("stopped-sha").exists(),
            "stopped-sha file should be removed after continue"
        );
    }

    #[test]
    fn multiple_squash_accumulates_messages() {
        // Test pick -> squash -> squash to verify message accumulation.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let hex_c = "cccccccccccccccccccccccccccccccccccccccc";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"First message");
        driver.register_commit(hex_b, hex_b, b"Second message");
        driver.register_commit(hex_c, hex_c, b"Third message");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "First".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_b).into(),
                summary: "Second (squash)".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_c).into(),
                summary: "Third (squash)".into(),
            },
        ]);

        state.step(&driver, &rebase_dir).unwrap();
        state.step(&driver, &rebase_dir).unwrap();
        state.step(&driver, &rebase_dir).unwrap();

        let calls = driver.cherry_pick_calls.borrow();
        assert_eq!(calls.len(), 3);

        // First squash combines First + Second.
        let msg1 = calls[1].1.as_ref().expect("first squash should have message");
        let msg1_str = String::from_utf8_lossy(msg1);
        assert!(msg1_str.contains("First message"), "should contain first: {msg1_str}");
        assert!(msg1_str.contains("Second message"), "should contain second: {msg1_str}");

        // Second squash should use the accumulated message (First + Second) + Third.
        // With the fix, accumulated_squash_message stores the combined result from
        // the first squash, so the second squash correctly includes all three.
        let msg2 = calls[2].1.as_ref().expect("second squash should have message");
        let msg2_str = String::from_utf8_lossy(msg2);
        assert!(
            msg2_str.contains("First message"),
            "second squash should contain first message: {msg2_str}"
        );
        assert!(
            msg2_str.contains("Second message"),
            "second squash should contain second message: {msg2_str}"
        );
        assert!(
            msg2_str.contains("Third message"),
            "second squash should contain third message: {msg2_str}"
        );
    }

    #[test]
    fn abort_when_rebase_dir_already_removed() {
        // Abort should succeed even if the rebase dir doesn't exist.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("nonexistent-rebase-merge");

        let driver = MockDriver::new();
        let state = make_state_with_ops(vec![]);

        // Should not error.
        state.abort(&driver, &rebase_dir).unwrap();

        let head_calls = driver.update_head_calls.borrow();
        assert_eq!(
            head_calls.len(),
            1,
            "abort should still call update_head even if dir is missing"
        );
    }

    #[test]
    fn step_state_persistence_after_each_operation() {
        // Verify that after each step, the on-disk state matches the in-memory state.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"First\n");
        driver.register_commit(hex_b, hex_b, b"Second\n");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "First".into(),
            },
            Operation::Drop {
                commit: make_oid(hex_b).into(),
                summary: "Dropped".into(),
            },
        ]);

        // Step 1: pick.
        state.step(&driver, &rebase_dir).unwrap();
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(on_disk.todo.operations.len(), 1);
        assert_eq!(on_disk.done.operations.len(), 1);
        assert_eq!(on_disk.current_step, 1);

        // Step 2: drop.
        state.step(&driver, &rebase_dir).unwrap();
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(on_disk.todo.operations.len(), 0);
        assert_eq!(on_disk.done.operations.len(), 2);
        assert_eq!(on_disk.current_step, 2);
    }

    // ---- Bug fix: squash message accumulation ----

    #[test]
    fn squash_chain_preserves_all_messages() {
        // Regression test: pick A -> squash B -> squash C -> squash D
        // The final combined message must contain ALL four messages.
        // Before the fix, squash C only saw B's original message (not A+B).
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let hex_c = "cccccccccccccccccccccccccccccccccccccccc";
        let hex_d = "dddddddddddddddddddddddddddddddddddddddd";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Message A");
        driver.register_commit(hex_b, hex_b, b"Message B");
        driver.register_commit(hex_c, hex_c, b"Message C");
        driver.register_commit(hex_d, hex_d, b"Message D");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "A".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_b).into(),
                summary: "B".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_c).into(),
                summary: "C".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_d).into(),
                summary: "D".into(),
            },
        ]);

        state.step(&driver, &rebase_dir).unwrap(); // pick A
        state.step(&driver, &rebase_dir).unwrap(); // squash B
        state.step(&driver, &rebase_dir).unwrap(); // squash C
        state.step(&driver, &rebase_dir).unwrap(); // squash D

        let calls = driver.cherry_pick_calls.borrow();
        // Final squash (D) should have all four messages.
        let final_msg = calls[3].1.as_ref().expect("fourth call should have combined message");
        let s = String::from_utf8_lossy(final_msg);
        assert!(s.contains("Message A"), "final message should contain A: {s}");
        assert!(s.contains("Message B"), "final message should contain B: {s}");
        assert!(s.contains("Message C"), "final message should contain C: {s}");
        assert!(s.contains("Message D"), "final message should contain D: {s}");
    }

    #[test]
    fn squash_message_cleared_after_non_squash() {
        // After a squash chain, a subsequent pick should NOT inherit accumulated messages.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let hex_c = "cccccccccccccccccccccccccccccccccccccccc";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Message A");
        driver.register_commit(hex_b, hex_b, b"Message B");
        driver.register_commit(hex_c, hex_c, b"Message C");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "A".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_b).into(),
                summary: "B".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_c).into(),
                summary: "C".into(),
            },
        ]);

        state.step(&driver, &rebase_dir).unwrap(); // pick A
        state.step(&driver, &rebase_dir).unwrap(); // squash B
        state.step(&driver, &rebase_dir).unwrap(); // pick C

        let calls = driver.cherry_pick_calls.borrow();
        // The third call (pick C) should NOT have a message override.
        assert!(
            calls[2].1.is_none(),
            "pick after squash should not inherit accumulated message"
        );
        // accumulated_squash_message should be cleared.
        assert!(
            state.accumulated_squash_message.is_none(),
            "accumulated message should be cleared after pick"
        );
    }

    #[test]
    fn fixup_chain_uses_accumulated_message() {
        // pick A -> fixup B -> fixup C: result should use A's message throughout.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let hex_c = "cccccccccccccccccccccccccccccccccccccccc";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Message A");
        driver.register_commit(hex_b, hex_b, b"Message B (discarded)");
        driver.register_commit(hex_c, hex_c, b"Message C (discarded)");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "A".into(),
            },
            Operation::Fixup {
                commit: make_oid(hex_b).into(),
                summary: "B".into(),
                amend_message: gix_sequencer::todo::AmendMessage::No,
            },
            Operation::Fixup {
                commit: make_oid(hex_c).into(),
                summary: "C".into(),
                amend_message: gix_sequencer::todo::AmendMessage::No,
            },
        ]);

        state.step(&driver, &rebase_dir).unwrap(); // pick A
        state.step(&driver, &rebase_dir).unwrap(); // fixup B
        state.step(&driver, &rebase_dir).unwrap(); // fixup C

        let calls = driver.cherry_pick_calls.borrow();
        // Both fixups should use A's message.
        let msg_b = calls[1].1.as_ref().expect("fixup B should have message");
        assert_eq!(msg_b.as_slice(), b"Message A", "fixup B should use A's message");

        let msg_c = calls[2].1.as_ref().expect("fixup C should have message");
        assert_eq!(
            msg_c.as_slice(),
            b"Message A",
            "fixup C should use A's (accumulated) message"
        );
    }

    // ---- Bug fix: skip losing operations ----

    #[test]
    fn failed_operation_appears_in_done_list() {
        // When cherry-pick fails (conflict), the operation must appear in `done`
        // so it is not lost from both todo and done.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Conflict commit\n");
        driver.register_commit(hex_b, hex_b, b"Next commit\n");
        driver.conflict_on.borrow_mut().insert(make_oid(hex_a));

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "Will conflict".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_b).into(),
                summary: "Next".into(),
            },
        ]);

        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err(), "conflict should error");

        // In-memory state: failed op should be in done.
        assert_eq!(state.done.operations.len(), 1, "failed op should be in done");
        assert_eq!(state.todo.operations.len(), 1, "second op should remain in todo");

        // On-disk state should also reflect this.
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert_eq!(on_disk.done.operations.len(), 1, "on-disk done should have failed op");
        assert_eq!(
            on_disk.todo.operations.len(),
            1,
            "on-disk todo should have remaining op"
        );
    }

    #[test]
    fn failed_other_error_preserves_operation() {
        // Same as conflict, but with a non-conflict error.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Failing\n");
        driver.fail_on.borrow_mut().insert(make_oid(hex));

        let mut state = make_state_with_ops(vec![Operation::Pick {
            commit: make_oid(hex).into(),
            summary: "Will fail".into(),
        }]);

        state.write_to(&rebase_dir).unwrap();
        let result = state.step(&driver, &rebase_dir);
        assert!(result.is_err());

        // Operation should be in done, not lost.
        assert_eq!(state.done.operations.len(), 1);
        assert!(state.todo.operations.is_empty());
    }

    // ---- Bug fix: reword returns original message ----

    #[test]
    fn reword_returns_original_message_in_paused() {
        // When reword pauses, the StepOutcome::Paused should include
        // the original commit message so the caller can present it for editing.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Original commit message\n");

        let mut state = make_state_with_ops(vec![Operation::Reword {
            commit: make_oid(hex).into(),
            summary: "Reword me".into(),
        }]);

        let outcome = state.step(&driver, &rebase_dir).unwrap();
        match outcome {
            StepOutcome::Paused {
                commit_id,
                original_message,
            } => {
                assert!(commit_id.is_some(), "reword should have a commit_id");
                let msg = original_message.expect("reword should include original_message");
                assert_eq!(
                    msg.as_slice(),
                    b"Original commit message\n",
                    "original_message should be the commit's message"
                );
            }
            other => panic!("expected Paused, got {:?}", other),
        }
    }

    #[test]
    fn edit_does_not_include_original_message() {
        // Edit should NOT include original_message (only reword does).
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let mut driver = MockDriver::new();
        driver.register_commit(hex, hex, b"Edit message\n");

        let mut state = make_state_with_ops(vec![Operation::Edit {
            commit: make_oid(hex).into(),
            summary: "Edit me".into(),
        }]);

        let outcome = state.step(&driver, &rebase_dir).unwrap();
        match outcome {
            StepOutcome::Paused {
                commit_id,
                original_message,
            } => {
                assert!(commit_id.is_some());
                assert!(original_message.is_none(), "edit should not include original_message");
            }
            other => panic!("expected Paused, got {:?}", other),
        }
    }

    #[test]
    fn accumulated_squash_message_persists_to_disk() {
        // Verify that accumulated_squash_message is persisted as message-squash file.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Message A");
        driver.register_commit(hex_b, hex_b, b"Message B");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "A".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_b).into(),
                summary: "B".into(),
            },
        ]);

        state.step(&driver, &rebase_dir).unwrap(); // pick
        state.step(&driver, &rebase_dir).unwrap(); // squash

        // The message-squash file should exist on disk.
        assert!(
            rebase_dir.join("message-squash").exists(),
            "message-squash file should be persisted"
        );

        // Read state back from disk.
        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        let accumulated = on_disk
            .accumulated_squash_message
            .expect("accumulated_squash_message should be read from disk");
        let s = String::from_utf8_lossy(&accumulated);
        assert!(s.contains("Message A"), "persisted message should contain A: {s}");
        assert!(s.contains("Message B"), "persisted message should contain B: {s}");
    }

    #[test]
    fn accumulated_message_cleared_after_pick() {
        // After a squash chain, the next pick should clear the message-squash file.
        let dir = tempfile::tempdir().unwrap();
        let rebase_dir = dir.path().join("rebase-merge");
        let hex_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hex_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let hex_c = "cccccccccccccccccccccccccccccccccccccccc";

        let mut driver = MockDriver::new();
        driver.register_commit(hex_a, hex_a, b"Message A");
        driver.register_commit(hex_b, hex_b, b"Message B");
        driver.register_commit(hex_c, hex_c, b"Message C");

        let mut state = make_state_with_ops(vec![
            Operation::Pick {
                commit: make_oid(hex_a).into(),
                summary: "A".into(),
            },
            Operation::Squash {
                commit: make_oid(hex_b).into(),
                summary: "B".into(),
            },
            Operation::Pick {
                commit: make_oid(hex_c).into(),
                summary: "C".into(),
            },
        ]);

        state.step(&driver, &rebase_dir).unwrap(); // pick A
        state.step(&driver, &rebase_dir).unwrap(); // squash B
        assert!(rebase_dir.join("message-squash").exists(), "should exist after squash");

        state.step(&driver, &rebase_dir).unwrap(); // pick C
        assert!(
            !rebase_dir.join("message-squash").exists(),
            "message-squash should be cleaned up after pick"
        );

        let on_disk = MergeState::read_from(&rebase_dir, Kind::Sha1).unwrap();
        assert!(
            on_disk.accumulated_squash_message.is_none(),
            "accumulated message should be None after pick"
        );
    }
}
