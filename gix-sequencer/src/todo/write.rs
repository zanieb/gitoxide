use std::io;

use crate::todo::{AmendMessage, Operation, TodoList};

impl TodoList {
    /// Write this todo list to the given writer in the standard git format.
    ///
    /// Each operation is written on its own line. The output is compatible with
    /// `git rebase --interactive` and `git sequencer`.
    ///
    /// Commit hashes are written at their original abbreviated length if they were
    /// parsed from abbreviated input, or as full hashes if they were full.
    pub fn write_to(&self, mut out: impl io::Write) -> io::Result<()> {
        for op in &self.operations {
            match op {
                Operation::Pick { commit, summary } => {
                    write_commit_operation(&mut out, "pick", commit, summary)?;
                }
                Operation::Reword { commit, summary } => {
                    write_commit_operation(&mut out, "reword", commit, summary)?;
                }
                Operation::Edit { commit, summary } => {
                    write_commit_operation(&mut out, "edit", commit, summary)?;
                }
                Operation::Squash { commit, summary } => {
                    write_commit_operation(&mut out, "squash", commit, summary)?;
                }
                Operation::Fixup {
                    commit,
                    summary,
                    amend_message,
                } => {
                    let flag = match amend_message {
                        AmendMessage::No => "",
                        AmendMessage::Replace => "-C ",
                        AmendMessage::Edit => "-c ",
                    };
                    if summary.is_empty() {
                        writeln!(out, "fixup {flag}{commit}")?;
                    } else {
                        writeln!(out, "fixup {flag}{commit} {summary}")?;
                    }
                }
                Operation::Exec { command } => {
                    writeln!(out, "exec {command}")?;
                }
                Operation::Break => {
                    writeln!(out, "break")?;
                }
                Operation::Noop => {
                    writeln!(out, "noop")?;
                }
                Operation::Drop { commit, summary } => {
                    write_commit_operation(&mut out, "drop", commit, summary)?;
                }
                Operation::Revert { commit, summary } => {
                    write_commit_operation(&mut out, "revert", commit, summary)?;
                }
                Operation::Merge { commit, label, oneline } => {
                    if let Some((prefix, amend)) = commit {
                        // For merge, `-C` = use commit message as-is, `-c` = open editor.
                        let flag = match amend {
                            AmendMessage::Replace => "-C",
                            AmendMessage::Edit => "-c",
                            AmendMessage::No => return Err(invalid_merge_amend_message()),
                        };
                        if oneline.is_empty() {
                            writeln!(out, "merge {flag} {prefix} {label}")?;
                        } else {
                            writeln!(out, "merge {flag} {prefix} {label} # {oneline}")?;
                        }
                    } else if oneline.is_empty() {
                        writeln!(out, "merge {label}")?;
                    } else {
                        writeln!(out, "merge {label} # {oneline}")?;
                    }
                }
                Operation::Label { name } => {
                    writeln!(out, "label {name}")?;
                }
                Operation::Reset { name } => {
                    writeln!(out, "reset {name}")?;
                }
                Operation::UpdateRef { reference } => {
                    writeln!(out, "update-ref {reference}")?;
                }
            }
        }
        Ok(())
    }
}

fn write_commit_operation(
    mut out: impl io::Write,
    command: &str,
    commit: &gix_hash::Prefix,
    summary: &bstr::BString,
) -> io::Result<()> {
    if summary.is_empty() {
        writeln!(out, "{command} {commit}")
    } else {
        writeln!(out, "{command} {commit} {summary}")
    }
}

fn invalid_merge_amend_message() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "AmendMessage::No is invalid for merge operations; merge requires -C or -c when a commit is specified",
    )
}

#[cfg(test)]
mod tests {
    use bstr::ByteSlice;
    use gix_hash::Kind;

    use crate::todo::TodoList;

    #[test]
    fn roundtrip() {
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
        assert_eq!(output.as_bstr(), input.as_bstr());
    }

    #[test]
    fn roundtrip_abbreviated() {
        let input = b"pick abcdef1 Short hash\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(output.as_bstr(), input.as_bstr());
    }

    #[test]
    fn commit_operation_without_summary_has_no_trailing_space() {
        let input = b"pick abcdef1\nfixup -C abcdef2\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(output.as_bstr(), input.as_bstr());
    }

    #[test]
    fn merge_with_edit_flag_roundtrips() {
        let input = b"merge -c 3333333333333333333333333333333333333333 my-branch # Merge branch\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(output.as_bstr(), input.as_bstr());
    }

    #[test]
    fn merge_without_commit_roundtrips() {
        let input = b"merge my-branch # Merge branch\n";
        let list = TodoList::parse(input.as_bstr(), Kind::Sha1).unwrap();
        let mut output = Vec::new();
        list.write_to(&mut output).unwrap();
        assert_eq!(output.as_bstr(), input.as_bstr());
    }

    #[test]
    fn merge_with_amend_no_returns_invalid_input() {
        // The parser never produces AmendMessage::No for merge operations.
        // If someone constructs this manually, return an error rather than
        // panicking or silently writing `-C` which changes semantics.
        use crate::todo::{AmendMessage, Operation};
        use gix_hash::Prefix;

        let list = TodoList {
            operations: [Operation::Merge {
                commit: Some((
                    Prefix::from_hex("3333333333333333333333333333333333333333").unwrap(),
                    AmendMessage::No,
                )),
                label: "my-branch".into(),
                oneline: "Merge branch".into(),
            }]
            .into(),
        };
        let mut output = Vec::new();
        let err = list.write_to(&mut output).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(
            err.to_string(),
            "AmendMessage::No is invalid for merge operations; merge requires -C or -c when a commit is specified"
        );
        assert!(
            output.is_empty(),
            "invalid operation should not produce partial todo output"
        );
    }
}
