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
                    writeln!(out, "pick {commit} {summary}")?;
                }
                Operation::Reword { commit, summary } => {
                    writeln!(out, "reword {commit} {summary}")?;
                }
                Operation::Edit { commit, summary } => {
                    writeln!(out, "edit {commit} {summary}")?;
                }
                Operation::Squash { commit, summary } => {
                    writeln!(out, "squash {commit} {summary}")?;
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
                    writeln!(out, "fixup {flag}{commit} {summary}")?;
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
                    writeln!(out, "drop {commit} {summary}")?;
                }
                Operation::Revert { commit, summary } => {
                    writeln!(out, "revert {commit} {summary}")?;
                }
                Operation::Merge { commit, label, oneline } => {
                    if let Some((prefix, amend)) = commit {
                        // For merge, `-C` = use commit message as-is, `-c` = open editor.
                        let flag = match amend {
                            AmendMessage::Replace => "-C",
                            AmendMessage::Edit => "-c",
                            AmendMessage::No => {
                                panic!("AmendMessage::No is invalid for merge operations; merge requires -C or -c when a commit is specified")
                            }
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
    #[should_panic(expected = "AmendMessage::No is invalid for merge operations")]
    fn merge_with_amend_no_panics() {
        // The parser never produces AmendMessage::No for merge operations.
        // If someone constructs this manually, it should panic rather than
        // silently writing `-C` which changes semantics.
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
        list.write_to(&mut output).unwrap();
    }
}
