pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[path = "todo_tests/mod.rs"]
mod todo_tests;

use bstr::ByteSlice;
use gix_hash::Kind;
use gix_sequencer::todo::{Operation, TodoList};

/// Integration test: parse a todo list from a realistic git rebase session
/// and verify the full parse -> inspect -> write cycle.
#[test]
fn parse_realistic_rebase_todo() {
    let input = b"\
# Rebase aabbccdd..11223344 onto aabbccdd (3 commands)
pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Initial commit
pick bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb Add feature X

# Rebase instructions...
# p, pick = use commit
";
    let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
    assert_eq!(list.operations.len(), 2);
    assert!(matches!(&list.operations[0], Operation::Pick { .. }));
    assert!(matches!(&list.operations[1], Operation::Pick { .. }));
}

/// Integration test: verify that write_to produces output that can be re-parsed
/// identically (full roundtrip through the public API).
#[test]
fn parse_write_parse_roundtrip() {
    let input = b"\
pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa First commit
reword bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb Second commit
exec cargo test
break
label onto
reset onto
update-ref refs/heads/main
";
    let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();

    let mut output = Vec::new();
    list.write_to(&mut output).unwrap();

    let reparsed = TodoList::parse(output.as_bstr(), Kind::Sha1).unwrap();
    assert_eq!(list, reparsed, "parse -> write -> parse should be idempotent");
}

/// Empty input produces empty operations list.
#[test]
fn parse_empty_input() {
    let list = TodoList::parse(b"".as_bstr(), Kind::Sha1).unwrap();
    assert!(list.operations.is_empty());
}

/// Comments-only input produces empty operations list.
#[test]
fn parse_comments_only() {
    let input = b"# This is just a comment\n# And another\n";
    let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
    assert!(list.operations.is_empty());
}
