use std::collections::VecDeque;

use bstr::{BStr, BString, ByteSlice};
use gix_hash::Prefix;

use crate::todo::{AmendMessage, Operation, TodoList};

/// The error returned by [`TodoList::parse()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("Unknown operation '{keyword}' on line {line_number}")]
    UnknownOperation { keyword: String, line_number: usize },
    #[error("Missing commit hash for '{keyword}' on line {line_number}")]
    MissingCommit { keyword: String, line_number: usize },
    #[error("Invalid commit hash '{hash}' on line {line_number}: {source}")]
    InvalidCommit {
        hash: String,
        line_number: usize,
        source: gix_hash::prefix::from_hex::Error,
    },
    #[error("Missing argument for '{keyword}' on line {line_number}")]
    MissingArgument { keyword: String, line_number: usize },
}

impl TodoList {
    /// Parse a todo list from a byte slice.
    ///
    /// Comment lines (starting with `#`) and empty lines are ignored.
    /// Abbreviated commit hashes are stored as [`Prefix`] values that preserve
    /// the original hex length. Callers must resolve abbreviated prefixes via the ODB.
    pub fn parse(input: &BStr, _hash_kind: gix_hash::Kind) -> Result<Self, Error> {
        let mut operations = VecDeque::new();

        for (line_number_zero, line) in input.lines().enumerate() {
            let line_number = line_number_zero + 1;
            let line = line.trim_ascii();
            if line.is_empty() || line.starts_with(b"#") {
                continue;
            }

            let (keyword, rest) = split_first_word(line);
            let keyword_str = keyword.to_str_lossy();

            match keyword {
                b"pick" | b"p" => {
                    let (commit, summary) = parse_commit_and_summary(rest, &keyword_str, line_number)?;
                    operations.push_back(Operation::Pick { commit, summary });
                }
                b"reword" | b"r" => {
                    let (commit, summary) = parse_commit_and_summary(rest, &keyword_str, line_number)?;
                    operations.push_back(Operation::Reword { commit, summary });
                }
                b"edit" | b"e" => {
                    let (commit, summary) = parse_commit_and_summary(rest, &keyword_str, line_number)?;
                    operations.push_back(Operation::Edit { commit, summary });
                }
                b"squash" | b"s" => {
                    let (commit, summary) = parse_commit_and_summary(rest, &keyword_str, line_number)?;
                    operations.push_back(Operation::Squash { commit, summary });
                }
                b"fixup" | b"f" => {
                    let (amend, fixup_rest) = parse_fixup_flags(rest);
                    let (commit, summary) = parse_commit_and_summary(fixup_rest, &keyword_str, line_number)?;
                    operations.push_back(Operation::Fixup {
                        commit,
                        summary,
                        amend_message: amend,
                    });
                }
                b"exec" | b"x" => {
                    let rest = rest.trim_ascii();
                    if rest.is_empty() {
                        return Err(Error::MissingArgument {
                            keyword: keyword_str.into_owned(),
                            line_number,
                        });
                    }
                    operations.push_back(Operation::Exec {
                        command: BString::from(rest),
                    });
                }
                b"break" | b"b" => {
                    operations.push_back(Operation::Break);
                }
                b"noop" => {
                    operations.push_back(Operation::Noop);
                }
                b"drop" | b"d" => {
                    let (commit, summary) = parse_commit_and_summary(rest, &keyword_str, line_number)?;
                    operations.push_back(Operation::Drop { commit, summary });
                }
                b"revert" => {
                    let (commit, summary) = parse_commit_and_summary(rest, &keyword_str, line_number)?;
                    operations.push_back(Operation::Revert { commit, summary });
                }
                b"merge" | b"m" => {
                    operations.push_back(parse_merge(rest, &keyword_str, line_number)?);
                }
                b"label" | b"l" => {
                    let rest = rest.trim_ascii();
                    if rest.is_empty() {
                        return Err(Error::MissingArgument {
                            keyword: keyword_str.into_owned(),
                            line_number,
                        });
                    }
                    let (name, _) = split_first_word(rest);
                    operations.push_back(Operation::Label {
                        name: BString::from(name),
                    });
                }
                b"reset" | b"t" => {
                    let rest = rest.trim_ascii();
                    if rest.is_empty() {
                        return Err(Error::MissingArgument {
                            keyword: keyword_str.into_owned(),
                            line_number,
                        });
                    }
                    let (name, _) = split_first_word(rest);
                    operations.push_back(Operation::Reset {
                        name: BString::from(name),
                    });
                }
                b"update-ref" | b"u" => {
                    let rest = rest.trim_ascii();
                    if rest.is_empty() {
                        return Err(Error::MissingArgument {
                            keyword: keyword_str.into_owned(),
                            line_number,
                        });
                    }
                    let (reference, _) = split_first_word(rest);
                    operations.push_back(Operation::UpdateRef {
                        reference: BString::from(reference),
                    });
                }
                _ => {
                    return Err(Error::UnknownOperation {
                        keyword: keyword_str.into_owned(),
                        line_number,
                    });
                }
            }
        }

        Ok(TodoList { operations })
    }
}

fn split_first_word(input: &[u8]) -> (&[u8], &[u8]) {
    let input = input.trim_ascii();
    match input.iter().position(|&b| b == b' ' || b == b'\t') {
        Some(pos) => (&input[..pos], &input[pos + 1..]),
        None => (input, &[]),
    }
}

fn parse_fixup_flags(rest: &[u8]) -> (AmendMessage, &[u8]) {
    let rest = rest.trim_ascii();
    if rest.starts_with(b"-C") {
        (AmendMessage::Replace, rest[2..].trim_ascii())
    } else if rest.starts_with(b"-c") {
        (AmendMessage::Edit, rest[2..].trim_ascii())
    } else {
        (AmendMessage::No, rest)
    }
}

/// Parse `merge [-C|-c <commit>] <label> [# <oneline>]`.
fn parse_merge(rest: &[u8], keyword: &str, line_number: usize) -> Result<Operation, Error> {
    let rest = rest.trim_ascii();
    if rest.is_empty() {
        return Err(Error::MissingArgument {
            keyword: keyword.to_owned(),
            line_number,
        });
    }

    // Check for -C/-c flag
    let (commit, remaining) = if rest.starts_with(b"-C") || rest.starts_with(b"-c") {
        let amend = if rest[1] == b'C' {
            AmendMessage::Replace
        } else {
            AmendMessage::Edit
        };
        let after_flag = rest[2..].trim_ascii();
        let (hash_bytes, after_hash) = split_first_word(after_flag);
        let hash_str = hash_bytes.to_str_lossy();
        let prefix = parse_hex_prefix(&hash_str, line_number)?;
        (Some((prefix, amend)), after_hash)
    } else {
        (None, rest)
    };

    let remaining = remaining.trim_ascii();
    // Split on `#` for label and oneline
    let (label_part, oneline_part) = match remaining.iter().position(|&b| b == b'#') {
        Some(pos) => (&remaining[..pos], remaining[pos + 1..].trim_ascii()),
        None => (remaining, &b""[..]),
    };

    let (label, _) = split_first_word(label_part);
    if label.is_empty() {
        return Err(Error::MissingArgument {
            keyword: keyword.to_owned(),
            line_number,
        });
    }

    Ok(Operation::Merge {
        commit,
        label: BString::from(label),
        oneline: BString::from(oneline_part),
    })
}

fn parse_hex_prefix(hash_str: &str, line_number: usize) -> Result<Prefix, Error> {
    // Use Prefix::from_hex which properly handles both full and abbreviated hashes.
    // For very short hashes (< 4 chars), use from_hex_nonempty.
    if hash_str.len() < Prefix::MIN_HEX_LEN {
        Prefix::from_hex_nonempty(hash_str)
    } else {
        Prefix::from_hex(hash_str)
    }
    .map_err(|source| Error::InvalidCommit {
        hash: hash_str.to_owned(),
        line_number,
        source,
    })
}

fn parse_commit_and_summary(rest: &[u8], keyword: &str, line_number: usize) -> Result<(Prefix, BString), Error> {
    let rest = rest.trim_ascii();
    if rest.is_empty() {
        return Err(Error::MissingCommit {
            keyword: keyword.to_owned(),
            line_number,
        });
    }
    let (hash_bytes, summary_rest) = split_first_word(rest);
    let hash_str = hash_bytes.to_str_lossy();

    let prefix = parse_hex_prefix(&hash_str, line_number)?;

    let summary = BString::from(summary_rest.trim_ascii());
    Ok((prefix, summary))
}

#[cfg(test)]
mod tests {
    use bstr::ByteSlice;
    use gix_hash::Kind;

    use super::*;

    #[test]
    fn parse_simple_pick_list() {
        let input = b"pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa First commit\npick bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb Second commit\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 2);
        match &list.operations[0] {
            Operation::Pick { commit, summary } => {
                assert_eq!(commit.hex_len(), 40);
                assert_eq!(commit.to_string(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(summary, "First commit");
            }
            other => panic!("expected Pick, got {other:?}"),
        }
    }

    #[test]
    fn parse_abbreviated_hashes_preserve_length() {
        let input = b"pick abcdef1 Short hash\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Pick { commit, .. } => {
                // Abbreviated hash preserves the original length
                assert_eq!(commit.hex_len(), 7);
                assert_eq!(commit.to_string(), "abcdef1");
            }
            other => panic!("expected Pick, got {other:?}"),
        }
    }

    #[test]
    fn parse_comments_and_empty_lines() {
        let input = b"# This is a comment\n\npick abcdef12 Short hash\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
    }

    #[test]
    fn parse_all_operations() {
        let input = b"\
pick aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Pick it
reword bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb Reword it
edit cccccccccccccccccccccccccccccccccccccccc Edit it
squash dddddddddddddddddddddddddddddddddddddddd Squash it
fixup eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee Fixup it
fixup -C ffffffffffffffffffffffffffffffffffffffff Fixup replace
exec make test
break
noop
drop 1111111111111111111111111111111111111111 Drop it
revert 2222222222222222222222222222222222222222 Revert it
merge -C 3333333333333333333333333333333333333333 my-branch # Merge branch
label onto
reset onto
update-ref refs/heads/main
";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 15);
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
        assert!(matches!(&list.operations[6], Operation::Exec { .. }));
        assert!(matches!(&list.operations[7], Operation::Break));
        assert!(matches!(&list.operations[8], Operation::Noop));
        assert!(matches!(&list.operations[9], Operation::Drop { .. }));
        assert!(matches!(&list.operations[10], Operation::Revert { .. }));
        assert!(matches!(&list.operations[11], Operation::Merge { .. }));
        assert!(matches!(&list.operations[12], Operation::Label { .. }));
        assert!(matches!(&list.operations[13], Operation::Reset { .. }));
        assert!(matches!(&list.operations[14], Operation::UpdateRef { .. }));
    }

    #[test]
    fn parse_revert() {
        let input = b"revert aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Revert bad commit\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Revert { commit, summary } => {
                assert_eq!(commit.to_string(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(summary, "Revert bad commit");
            }
            other => panic!("expected Revert, got {other:?}"),
        }
    }

    #[test]
    fn parse_noop() {
        let input = b"noop\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        assert!(matches!(&list.operations[0], Operation::Noop));
    }

    #[test]
    fn parse_merge_with_commit() {
        let input = b"merge -C aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa my-branch # Merge my-branch\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Merge { commit, label, oneline } => {
                let (prefix, amend) = commit.as_ref().expect("should have commit");
                assert_eq!(prefix.to_string(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(*amend, AmendMessage::Replace);
                assert_eq!(label, "my-branch");
                assert_eq!(oneline, "Merge my-branch");
            }
            other => panic!("expected Merge, got {other:?}"),
        }
    }

    #[test]
    fn parse_merge_without_commit() {
        let input = b"merge my-branch # Merge my-branch\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 1);
        match &list.operations[0] {
            Operation::Merge { commit, label, oneline } => {
                assert!(commit.is_none());
                assert_eq!(label, "my-branch");
                assert_eq!(oneline, "Merge my-branch");
            }
            other => panic!("expected Merge, got {other:?}"),
        }
    }

    #[test]
    fn parse_short_commands() {
        let input = b"p aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Pick\nr bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb Reword\ne cccccccccccccccccccccccccccccccccccccccc Edit\ns dddddddddddddddddddddddddddddddddddddddd Squash\nf eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee Fixup\nx echo hello\nb\nd 1111111111111111111111111111111111111111 Drop\nl my-label\nt my-label\nu refs/heads/main\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        assert_eq!(list.operations.len(), 11);
    }

    #[test]
    fn parse_unknown_operation_errors() {
        let input = b"unknown aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa Bad\n";
        let result = TodoList::parse(input.as_bstr(), Kind::Sha1);
        assert!(result.is_err());
    }
}
