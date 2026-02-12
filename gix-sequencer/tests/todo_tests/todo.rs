mod parse {
    use bstr::ByteSlice;
    use gix_hash::Kind;
    use gix_sequencer::todo::{AmendMessage, Operation, TodoList};

    #[test]
    fn empty_input_produces_empty_list() {
        let list = TodoList::parse(b"".as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 0);
    }

    #[test]
    fn whitespace_only_input_produces_empty_list() {
        let list = TodoList::parse(b"   \n  \n\n".as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 0);
    }

    #[test]
    fn comments_only_input_produces_empty_list() {
        let input = b"# This is a rebase todo list\n# All lines are comments\n# Nothing to do\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 0);
    }

    #[test]
    fn comments_interspersed_with_operations_are_ignored() {
        let input = b"\
# Rebase abc..def onto abc (2 commands)
pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa First
# This is a comment between operations
pick bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb Second
# Another comment at the end
";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 2);
    }

    #[test]
    fn pick_with_full_hash() {
        let input = b"pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Add feature\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Pick { commit, summary } => {
                assert_eq!(commit.hex_len(), 40);
                assert_eq!(commit.to_string(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(summary.as_bytes(), b"Add feature");
            }
            other => panic!("expected Pick, got {other:?}"),
        }
    }

    #[test]
    fn pick_with_abbreviated_hash_preserves_length() {
        let input = b"pick abcdef1 Short hash commit\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Pick { commit, .. } => {
                assert_eq!(commit.hex_len(), 7, "abbreviated hash should preserve 7-char length");
                assert_eq!(commit.to_string(), "abcdef1");
            }
            other => panic!("expected Pick, got {other:?}"),
        }
    }

    #[test]
    fn pick_with_4_char_hash_minimum_length() {
        let input = b"pick abcd Minimal hash\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Pick { commit, .. } => {
                assert_eq!(commit.hex_len(), 4);
            }
            other => panic!("expected Pick, got {other:?}"),
        }
    }

    #[test]
    fn reword_operation() {
        let input = b"reword aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Fix typo\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Reword { commit, summary } => {
                assert_eq!(commit.to_string(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(summary.as_bytes(), b"Fix typo");
            }
            other => panic!("expected Reword, got {other:?}"),
        }
    }

    #[test]
    fn edit_operation() {
        let input = b"edit aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Stop here\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        assert!(matches!(&list.operations[0], Operation::Edit { .. }));
    }

    #[test]
    fn squash_operation() {
        let input = b"squash aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Squash me\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        assert!(matches!(&list.operations[0], Operation::Squash { .. }));
    }

    #[test]
    fn fixup_plain() {
        let input = b"fixup aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Fixup me\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Fixup { amend_message, .. } => {
                assert_eq!(*amend_message, AmendMessage::No);
            }
            other => panic!("expected Fixup, got {other:?}"),
        }
    }

    #[test]
    fn fixup_with_replace_flag() {
        let input = b"fixup -C aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Replace msg\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Fixup {
                amend_message, summary, ..
            } => {
                assert_eq!(*amend_message, AmendMessage::Replace);
                assert_eq!(summary.as_bytes(), b"Replace msg");
            }
            other => panic!("expected Fixup with Replace, got {other:?}"),
        }
    }

    #[test]
    fn fixup_with_edit_flag() {
        let input = b"fixup -c aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Edit msg\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Fixup { amend_message, .. } => {
                assert_eq!(*amend_message, AmendMessage::Edit);
            }
            other => panic!("expected Fixup with Edit, got {other:?}"),
        }
    }

    #[test]
    fn exec_command() {
        let input = b"exec make test && echo done\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Exec { command } => {
                assert_eq!(command.as_bytes(), b"make test && echo done");
            }
            other => panic!("expected Exec, got {other:?}"),
        }
    }

    #[test]
    fn exec_missing_command_errors() {
        let input = b"exec\n";
        let result = TodoList::parse(input.as_bstr(), Kind::Sha1);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Missing argument"),
            "expected missing argument error, got: {err}"
        );
    }

    #[test]
    fn break_operation() {
        let input = b"break\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        assert!(matches!(&list.operations[0], Operation::Break));
    }

    #[test]
    fn noop_operation() {
        let input = b"noop\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        assert!(matches!(&list.operations[0], Operation::Noop));
    }

    #[test]
    fn drop_operation() {
        let input = b"drop aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Remove this\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Drop { commit, summary } => {
                assert_eq!(commit.to_string(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(summary.as_bytes(), b"Remove this");
            }
            other => panic!("expected Drop, got {other:?}"),
        }
    }

    #[test]
    fn revert_operation() {
        let input = b"revert aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Revert bad commit\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Revert { commit, summary } => {
                assert_eq!(commit.to_string(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(summary.as_bytes(), b"Revert bad commit");
            }
            other => panic!("expected Revert, got {other:?}"),
        }
    }

    #[test]
    fn label_operation() {
        let input = b"label onto\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Label { name } => {
                assert_eq!(name.as_bytes(), b"onto");
            }
            other => panic!("expected Label, got {other:?}"),
        }
    }

    #[test]
    fn label_missing_name_errors() {
        let input = b"label\n";
        let result = TodoList::parse(input.as_bstr(), Kind::Sha1);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Missing argument"),
            "expected missing argument error, got: {err}"
        );
    }

    #[test]
    fn reset_operation() {
        let input = b"reset onto\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Reset { name } => {
                assert_eq!(name.as_bytes(), b"onto");
            }
            other => panic!("expected Reset, got {other:?}"),
        }
    }

    #[test]
    fn reset_missing_name_errors() {
        let input = b"reset\n";
        let result = TodoList::parse(input.as_bstr(), Kind::Sha1);
        assert!(result.is_err());
    }

    #[test]
    fn update_ref_operation() {
        let input = b"update-ref refs/heads/main\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::UpdateRef { reference } => {
                assert_eq!(reference.as_bytes(), b"refs/heads/main");
            }
            other => panic!("expected UpdateRef, got {other:?}"),
        }
    }

    #[test]
    fn update_ref_missing_ref_errors() {
        let input = b"update-ref\n";
        let result = TodoList::parse(input.as_bstr(), Kind::Sha1);
        assert!(result.is_err());
    }

    #[test]
    fn merge_with_commit_and_oneline() {
        let input = b"merge -C aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa my-branch # Merge my-branch\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Merge { commit, label, oneline } => {
                let (prefix, amend) = commit.as_ref().expect("should have commit");
                assert_eq!(prefix.to_string(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(*amend, AmendMessage::Replace);
                assert_eq!(label.as_bytes(), b"my-branch");
                assert_eq!(oneline.as_bytes(), b"Merge my-branch");
            }
            other => panic!("expected Merge, got {other:?}"),
        }
    }

    #[test]
    fn merge_with_edit_flag() {
        let input = b"merge -c aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa my-branch # Edit merge msg\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        match &list.operations[0] {
            Operation::Merge { commit, .. } => {
                let (_, amend) = commit.as_ref().expect("should have commit");
                assert_eq!(*amend, AmendMessage::Edit);
            }
            other => panic!("expected Merge, got {other:?}"),
        }
    }

    #[test]
    fn merge_without_commit() {
        let input = b"merge my-branch # Simple merge\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        match &list.operations[0] {
            Operation::Merge { commit, label, oneline } => {
                assert!(commit.is_none());
                assert_eq!(label.as_bytes(), b"my-branch");
                assert_eq!(oneline.as_bytes(), b"Simple merge");
            }
            other => panic!("expected Merge, got {other:?}"),
        }
    }

    #[test]
    fn merge_without_oneline() {
        let input = b"merge -C aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa my-branch\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        match &list.operations[0] {
            Operation::Merge { oneline, .. } => {
                assert!(oneline.is_empty());
            }
            other => panic!("expected Merge, got {other:?}"),
        }
    }

    #[test]
    fn merge_missing_label_errors() {
        let input = b"merge\n";
        let result = TodoList::parse(input.as_bstr(), Kind::Sha1);
        assert!(result.is_err());
    }

    #[test]
    fn short_aliases_for_all_commands() {
        let input = b"\
p aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Pick
r bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb Reword
e cccccccccccccccccccccccccccccccccccccccc Edit
s dddddddddddddddddddddddddddddddddddddddd Squash
f eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee Fixup
x echo hello
b
d 1111111111111111111111111111111111111111 Drop
l my-label
t my-reset-target
u refs/heads/main
m my-branch # Merge
";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 12);
        assert!(matches!(&list.operations[0], Operation::Pick { .. }));
        assert!(matches!(&list.operations[1], Operation::Reword { .. }));
        assert!(matches!(&list.operations[2], Operation::Edit { .. }));
        assert!(matches!(&list.operations[3], Operation::Squash { .. }));
        assert!(matches!(&list.operations[4], Operation::Fixup { .. }));
        assert!(matches!(&list.operations[5], Operation::Exec { .. }));
        assert!(matches!(&list.operations[6], Operation::Break));
        assert!(matches!(&list.operations[7], Operation::Drop { .. }));
        assert!(matches!(&list.operations[8], Operation::Label { .. }));
        assert!(matches!(&list.operations[9], Operation::Reset { .. }));
        assert!(matches!(&list.operations[10], Operation::UpdateRef { .. }));
        assert!(matches!(&list.operations[11], Operation::Merge { .. }));
    }

    #[test]
    fn unknown_operation_errors_with_line_number() {
        let input = b"pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ok\nbogus something\n";
        let result = TodoList::parse(input.as_bstr(), Kind::Sha1);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bogus"), "error should mention the unknown keyword: {msg}");
        assert!(msg.contains("line 2"), "error should mention line number 2: {msg}");
    }

    #[test]
    fn missing_commit_hash_errors() {
        let input = b"pick\n";
        let result = TodoList::parse(input.as_bstr(), Kind::Sha1);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Missing commit"),
            "expected missing commit error, got: {err}"
        );
    }

    #[test]
    fn invalid_hex_in_hash_errors() {
        let input = b"pick zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz Bad hash\n";
        let result = TodoList::parse(input.as_bstr(), Kind::Sha1);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Invalid commit"),
            "expected invalid commit error, got: {err}"
        );
    }

    #[test]
    fn leading_whitespace_on_lines_is_trimmed() {
        let input = b"  pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Indented\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        assert!(matches!(&list.operations[0], Operation::Pick { .. }));
    }

    #[test]
    fn all_operations_in_one_todo_list() {
        // Ported from git/t/t3404-rebase-interactive.sh: comprehensive todo list
        let input = b"\
pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa First commit
reword bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb Edit message
edit cccccccccccccccccccccccccccccccccccccccc Stop here
squash dddddddddddddddddddddddddddddddddddddddd Squash into prev
fixup eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee Quick fix
fixup -C ffffffffffffffffffffffffffffffffffffffff Replace msg
fixup -c 1111111111111111111111111111111111111111 Edit combined msg
exec make test
break
noop
drop 2222222222222222222222222222222222222222 Remove this
revert 3333333333333333333333333333333333333333 Undo bad change
merge -C 4444444444444444444444444444444444444444 feature # Merge feature
merge feature-2 # Octopus
label onto
reset onto
update-ref refs/heads/main
";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 17);

        // Verify specific operations
        assert!(matches!(&list.operations[0], Operation::Pick { .. }));
        assert!(matches!(&list.operations[1], Operation::Reword { .. }));
        assert!(matches!(&list.operations[2], Operation::Edit { .. }));
        assert!(matches!(&list.operations[3], Operation::Squash { .. }));
        assert!(matches!(
            &list.operations[4],
            Operation::Fixup {
                amend_message: AmendMessage::No,
                ..
            }
        ));
        assert!(matches!(
            &list.operations[5],
            Operation::Fixup {
                amend_message: AmendMessage::Replace,
                ..
            }
        ));
        assert!(matches!(
            &list.operations[6],
            Operation::Fixup {
                amend_message: AmendMessage::Edit,
                ..
            }
        ));
        assert!(matches!(&list.operations[7], Operation::Exec { .. }));
        assert!(matches!(&list.operations[8], Operation::Break));
        assert!(matches!(&list.operations[9], Operation::Noop));
        assert!(matches!(&list.operations[10], Operation::Drop { .. }));
        assert!(matches!(&list.operations[11], Operation::Revert { .. }));
        assert!(matches!(&list.operations[12], Operation::Merge { commit: Some(_), .. }));
        assert!(matches!(&list.operations[13], Operation::Merge { commit: None, .. }));
        assert!(matches!(&list.operations[14], Operation::Label { .. }));
        assert!(matches!(&list.operations[15], Operation::Reset { .. }));
        assert!(matches!(&list.operations[16], Operation::UpdateRef { .. }));
    }

    #[test]
    fn pick_without_summary_uses_empty_summary() {
        // Git allows commit-only lines with no summary text
        let input = b"pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Pick { summary, .. } => {
                assert!(summary.is_empty(), "summary should be empty when not provided");
            }
            other => panic!("expected Pick, got {other:?}"),
        }
    }
}

mod write {
    use bstr::ByteSlice;
    use gix_sequencer::todo::{AmendMessage, Operation, TodoList};

    #[test]
    fn empty_list_writes_nothing() {
        let list = TodoList {
            operations: std::collections::VecDeque::new(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn write_single_pick() {
        let list = TodoList {
            operations: vec![Operation::Pick {
                commit: gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .unwrap()
                    .into(),
                summary: "Test commit".into(),
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(
            out.as_bstr(),
            b"pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Test commit\n".as_bstr()
        );
    }

    #[test]
    fn write_abbreviated_hash_preserves_length() {
        let prefix = gix_hash::Prefix::from_hex("abcdef1").unwrap();
        let list = TodoList {
            operations: vec![Operation::Pick {
                commit: prefix,
                summary: "Short".into(),
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(out.as_bstr(), b"pick abcdef1 Short\n".as_bstr());
    }

    #[test]
    fn write_fixup_with_replace_flag() {
        let list = TodoList {
            operations: vec![Operation::Fixup {
                commit: gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .unwrap()
                    .into(),
                summary: "Fixup".into(),
                amend_message: AmendMessage::Replace,
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(
            out.as_bstr(),
            b"fixup -C aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Fixup\n".as_bstr()
        );
    }

    #[test]
    fn write_fixup_with_edit_flag() {
        let list = TodoList {
            operations: vec![Operation::Fixup {
                commit: gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .unwrap()
                    .into(),
                summary: "Fixup".into(),
                amend_message: AmendMessage::Edit,
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(
            out.as_bstr(),
            b"fixup -c aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Fixup\n".as_bstr()
        );
    }

    #[test]
    fn write_exec_preserves_command() {
        let list = TodoList {
            operations: vec![Operation::Exec {
                command: "make test && echo done".into(),
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(out.as_bstr(), b"exec make test && echo done\n".as_bstr());
    }

    #[test]
    fn write_break() {
        let list = TodoList {
            operations: vec![Operation::Break].into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(out.as_bstr(), b"break\n".as_bstr());
    }

    #[test]
    fn write_noop() {
        let list = TodoList {
            operations: vec![Operation::Noop].into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(out.as_bstr(), b"noop\n".as_bstr());
    }

    #[test]
    fn write_merge_with_commit_and_oneline() {
        let list = TodoList {
            operations: vec![Operation::Merge {
                commit: Some((
                    gix_hash::ObjectId::from_hex(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                        .unwrap()
                        .into(),
                    AmendMessage::Replace,
                )),
                label: "my-branch".into(),
                oneline: "Merge branch".into(),
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(
            out.as_bstr(),
            b"merge -C aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa my-branch # Merge branch\n".as_bstr()
        );
    }

    #[test]
    fn write_merge_without_commit() {
        let list = TodoList {
            operations: vec![Operation::Merge {
                commit: None,
                label: "my-branch".into(),
                oneline: "Merge branch".into(),
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(out.as_bstr(), b"merge my-branch # Merge branch\n".as_bstr());
    }

    #[test]
    fn write_merge_without_oneline() {
        let list = TodoList {
            operations: vec![Operation::Merge {
                commit: None,
                label: "my-branch".into(),
                oneline: "".into(),
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(out.as_bstr(), b"merge my-branch\n".as_bstr());
    }

    #[test]
    fn write_label() {
        let list = TodoList {
            operations: vec![Operation::Label { name: "onto".into() }].into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(out.as_bstr(), b"label onto\n".as_bstr());
    }

    #[test]
    fn write_reset() {
        let list = TodoList {
            operations: vec![Operation::Reset { name: "onto".into() }].into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(out.as_bstr(), b"reset onto\n".as_bstr());
    }

    #[test]
    fn write_update_ref() {
        let list = TodoList {
            operations: vec![Operation::UpdateRef {
                reference: "refs/heads/main".into(),
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
        assert_eq!(out.as_bstr(), b"update-ref refs/heads/main\n".as_bstr());
    }

    #[test]
    #[should_panic(expected = "AmendMessage::No is invalid for merge operations")]
    fn write_merge_with_amend_no_panics() {
        let list = TodoList {
            operations: vec![Operation::Merge {
                commit: Some((
                    gix_hash::Prefix::from_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
                    AmendMessage::No,
                )),
                label: "branch".into(),
                oneline: "msg".into(),
            }]
            .into(),
        };
        let mut out = Vec::new();
        list.write_to(&mut out).unwrap();
    }
}

mod roundtrip {
    use bstr::ByteSlice;
    use gix_hash::Kind;
    use gix_sequencer::todo::TodoList;

    #[test]
    fn full_todo_list_roundtrips() {
        let input = b"\
pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa First commit
reword bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb Second commit
edit cccccccccccccccccccccccccccccccccccccccc Third commit
squash dddddddddddddddddddddddddddddddddddddddd Fourth commit
fixup eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee Fifth commit
fixup -C ffffffffffffffffffffffffffffffffffffffff Sixth commit
exec make test
break
noop
drop 1111111111111111111111111111111111111111 Seventh commit
revert 2222222222222222222222222222222222222222 Eighth commit
merge -C 3333333333333333333333333333333333333333 my-branch # Merge branch
label onto
reset onto
update-ref refs/heads/main
";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(
            output.as_bstr(),
            input.as_bstr(),
            "parse -> write should produce identical output"
        );
    }

    #[test]
    fn abbreviated_hash_roundtrips() {
        let input = b"pick abcdef1 Short hash\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(output.as_bstr(), input.as_bstr());
    }

    #[test]
    fn comments_are_stripped_during_roundtrip() {
        let input = b"\
# Rebase abc..def onto abc (1 command)
pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa First commit
# Done
";
        let expected = b"pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa First commit\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(
            output.as_bstr(),
            expected.as_bstr(),
            "comments should be stripped after parse -> write"
        );
    }

    #[test]
    fn rebase_merges_todo_roundtrips() {
        // Ported from Git's rebase --rebase-merges todo format
        let input = b"\
label onto
reset onto
pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa feature commit 1
pick bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb feature commit 2
label feature
reset onto
merge -C cccccccccccccccccccccccccccccccccccccccc feature # Merge branch 'feature'
update-ref refs/heads/main
";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(output.as_bstr(), input.as_bstr());
    }

    #[test]
    fn fixup_edit_flag_roundtrips() {
        let input = b"fixup -c aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Edit combined message\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(output.as_bstr(), input.as_bstr());
    }

    #[test]
    fn merge_with_edit_flag_roundtrips() {
        let input = b"merge -c aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa branch # Merge\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(output.as_bstr(), input.as_bstr());
    }

    #[test]
    fn merge_no_commit_no_oneline_roundtrips() {
        let input = b"merge my-branch\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(output.as_bstr(), input.as_bstr());
    }
}
