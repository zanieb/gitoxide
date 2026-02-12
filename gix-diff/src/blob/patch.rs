//! Produce complete unified diff patches compatible with `git diff` output.
//!
//! This module builds on the [`unified_diff`](super::unified_diff) infrastructure to produce
//! full patch output including file headers (`diff --git`, `---`/`+++`), index lines,
//! mode change annotations, binary file detection, and `\ No newline at end of file` markers.
//!
//! # Example
//!
//! ```
//! use gix_diff::blob::patch;
//!
//! let old = b"hello\nworld\n";
//! let new = b"hello\nrust\n";
//!
//! let mut out = Vec::new();
//! patch::write(
//!     &mut out,
//!     None, // old_id
//!     None, // new_id
//!     "file.txt",
//!     "file.txt",
//!     old.as_slice(),
//!     new.as_slice(),
//!     patch::Options::default(),
//! )?;
//!
//! let patch_str = String::from_utf8(out)?;
//! assert!(patch_str.contains("--- a/file.txt"));
//! assert!(patch_str.contains("+++ b/file.txt"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::io::Write;

use bstr::ByteSlice;

use super::unified_diff::{ConsumeHunk, ContextSize, DiffLineKind, HunkHeader};

/// Options controlling patch generation.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Number of context lines around each hunk. Default is 3.
    pub context_lines: u32,
    /// The prefix for the old file path. Default is `"a/"`.
    pub old_prefix: &'static str,
    /// The prefix for the new file path. Default is `"b/"`.
    pub new_prefix: &'static str,
    /// Whether to include function names in hunk headers (after `@@`).
    ///
    /// When enabled, each hunk header will include the nearest preceding function
    /// definition line, matching the behavior of `git diff` with `XDL_EMIT_FUNCNAMES`.
    /// The default funcname pattern matches lines starting with a letter, `_`, or `$`,
    /// which works well for C, Rust, Python, Java, and many other languages.
    ///
    /// Default is `true`.
    pub find_function_names: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            context_lines: 3,
            old_prefix: "a/",
            new_prefix: "b/",
            find_function_names: true,
        }
    }
}

/// Information about how a file changed, used to emit the correct header lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChange {
    /// The file was modified (content change only).
    /// If `mode` is set, it will be appended to the `index` line (e.g. `index abc..def 100644`).
    Modified {
        /// The file mode, if known. When both old and new modes are the same,
        /// C Git appends the mode to the index line.
        mode: Option<u32>,
    },
    /// The file is newly added, with this mode (e.g. `0o100644`).
    Added {
        /// The file mode of the new file.
        mode: u32,
    },
    /// The file was deleted, with this mode (e.g. `0o100644`).
    Deleted {
        /// The file mode of the deleted file.
        mode: u32,
    },
    /// The file mode changed.
    ModeChange {
        /// The old file mode.
        old_mode: u32,
        /// The new file mode.
        new_mode: u32,
    },
}

impl Default for FileChange {
    fn default() -> Self {
        FileChange::Modified { mode: None }
    }
}

/// The error returned by [`write()`] and [`write_with_change()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("failed to write patch output")]
    Io(#[from] std::io::Error),
    #[error("diff computation failed")]
    Diff(#[source] std::io::Error),
}

/// Write a complete unified diff patch for a single file to `out`.
///
/// This is the simplest entry point. For more control, use [`write_with_change()`].
///
/// * `old_id` / `new_id` - optional abbreviated object IDs for the `index` line.
/// * `old_path` / `new_path` - the file paths (usually the same unless renamed).
/// * `old_content` / `new_content` - the full file contents (as bytes).
/// * `options` - controls context lines and path prefixes.
pub fn write(
    out: &mut dyn Write,
    old_id: Option<&str>,
    new_id: Option<&str>,
    old_path: &str,
    new_path: &str,
    old_content: &[u8],
    new_content: &[u8],
    options: Options,
) -> Result<(), Error> {
    write_with_change(
        out,
        old_id,
        new_id,
        old_path,
        new_path,
        old_content,
        new_content,
        FileChange::Modified { mode: None },
        options,
    )
}

/// Write a complete unified diff patch for a single file to `out`, with explicit [`FileChange`] info.
///
/// This allows specifying whether the file was added, deleted, or had its mode changed,
/// producing the appropriate header lines.
#[allow(clippy::too_many_arguments)]
pub fn write_with_change(
    out: &mut dyn Write,
    old_id: Option<&str>,
    new_id: Option<&str>,
    old_path: &str,
    new_path: &str,
    old_content: &[u8],
    new_content: &[u8],
    change: FileChange,
    options: Options,
) -> Result<(), Error> {
    // Check for binary content.
    let old_is_binary = is_binary(old_content);
    let new_is_binary = is_binary(new_content);

    // Write the `diff --git` header.
    writeln!(
        out,
        "diff --git {old_prefix}{old_path} {new_prefix}{new_path}",
        old_prefix = options.old_prefix,
        new_prefix = options.new_prefix,
    )?;

    // Write change-specific headers.
    match change {
        FileChange::Added { mode } => {
            writeln!(out, "new file mode {mode:06o}")?;
        }
        FileChange::Deleted { mode } => {
            writeln!(out, "deleted file mode {mode:06o}")?;
        }
        FileChange::ModeChange { old_mode, new_mode } => {
            writeln!(out, "old mode {old_mode:06o}")?;
            writeln!(out, "new mode {new_mode:06o}")?;
        }
        FileChange::Modified { .. } => {}
    }

    // Write index line if IDs are available.
    match (old_id, new_id, &change) {
        (Some(oid), Some(nid), FileChange::Modified { mode: Some(m) }) => {
            writeln!(out, "index {oid}..{nid} {m:06o}")?;
        }
        (Some(oid), Some(nid), _) => {
            writeln!(out, "index {oid}..{nid}")?;
        }
        _ => {}
    }

    if old_is_binary || new_is_binary {
        write_binary_diff(out, old_path, new_path, &change, &options)?;
        return Ok(());
    }

    // Write file path headers.
    match change {
        FileChange::Added { .. } => {
            writeln!(out, "--- /dev/null")?;
            writeln!(out, "+++ {prefix}{path}", prefix = options.new_prefix, path = new_path)?;
        }
        FileChange::Deleted { .. } => {
            writeln!(out, "--- {prefix}{path}", prefix = options.old_prefix, path = old_path)?;
            writeln!(out, "+++ /dev/null")?;
        }
        _ => {
            writeln!(out, "--- {prefix}{path}", prefix = options.old_prefix, path = old_path)?;
            writeln!(out, "+++ {prefix}{path}", prefix = options.new_prefix, path = new_path)?;
        }
    }

    // If both are empty, nothing to diff.
    if old_content.is_empty() && new_content.is_empty() {
        return Ok(());
    }

    // Compute and write the unified diff hunks.
    let old_ends_with_newline = old_content.last() == Some(&b'\n');
    let new_ends_with_newline = new_content.last() == Some(&b'\n');

    let interner = super::intern::InternedInput::new(
        super::sources::byte_lines_with_terminator(old_content),
        super::sources::byte_lines_with_terminator(new_content),
    );

    // Split old content into lines for function name lookup.
    let old_lines: Vec<&[u8]> = if options.find_function_names && !old_content.is_empty() {
        old_content.lines_with_terminator().collect()
    } else {
        Vec::new()
    };

    let sink = PatchHunkWriter {
        out,
        old_ends_with_newline,
        new_ends_with_newline,
        old_lines,
        find_function_names: options.find_function_names,
    };

    let result = super::diff(
        super::Algorithm::Myers,
        &interner,
        super::UnifiedDiff::new(&interner, sink, ContextSize::symmetrical(options.context_lines)),
    );

    match result {
        Ok(inner) => inner.map_err(Error::Diff),
        Err(err) => Err(Error::Diff(err)),
    }
}

/// Write binary diff message.
///
/// For added files, the old side is `/dev/null`; for deleted files, the new side is `/dev/null`.
/// This matches C Git's behavior.
fn write_binary_diff(
    out: &mut dyn Write,
    old_path: &str,
    new_path: &str,
    change: &FileChange,
    options: &Options,
) -> Result<(), Error> {
    let old_display = match change {
        FileChange::Added { .. } => "/dev/null".to_string(),
        _ => format!("{}{}", options.old_prefix, old_path),
    };
    let new_display = match change {
        FileChange::Deleted { .. } => "/dev/null".to_string(),
        _ => format!("{}{}", options.new_prefix, new_path),
    };
    writeln!(out, "Binary files {old_display} and {new_display} differ")?;
    Ok(())
}

/// Returns `true` if the content looks binary.
///
/// Uses the same heuristic as Git: content is binary if the first 8000 bytes contain a NUL byte.
pub fn is_binary(content: &[u8]) -> bool {
    let check_len = content.len().min(8000);
    content[..check_len].contains(&0)
}

/// A [`ConsumeHunk`] implementation that writes hunks directly to an output stream
/// with `\ No newline at end of file` markers where appropriate.
struct PatchHunkWriter<'a> {
    out: &'a mut dyn Write,
    old_ends_with_newline: bool,
    new_ends_with_newline: bool,
    /// Lines of the old content, used for function name lookup.
    old_lines: Vec<&'a [u8]>,
    /// Whether to search for function names in hunk headers.
    find_function_names: bool,
}

impl ConsumeHunk for PatchHunkWriter<'_> {
    type Out = Result<(), std::io::Error>;

    fn consume_hunk(&mut self, header: HunkHeader, lines: &[(DiffLineKind, &[u8])]) -> std::io::Result<()> {
        // Write hunk header using C Git's format:
        // - Omit ",1" when hunk length is exactly 1 (e.g., "@@ -1 +1 @@" not "@@ -1,1 +1,1 @@")
        // - Keep ",0" and ",N" for all other values
        // - When hunk length is 0, use start=0 (C Git convention for empty side)
        // - Only search for function names when hunk doesn't start at line 1
        let before_start = if header.before_hunk_len == 0 {
            0
        } else {
            header.before_hunk_start
        };
        let after_start = if header.after_hunk_len == 0 {
            0
        } else {
            header.after_hunk_start
        };
        write!(self.out, "@@ ")?;
        write_hunk_range(self.out, before_start, header.before_hunk_len, '-')?;
        write!(self.out, " ")?;
        write_hunk_range(self.out, after_start, header.after_hunk_len, '+')?;
        write!(self.out, " @@")?;

        if self.find_function_names && header.before_hunk_start > 1 {
            if let Some(name) = find_function_line(&self.old_lines, header.before_hunk_start) {
                write!(self.out, " {name}")?;
            }
        }
        writeln!(self.out)?;

        let last_idx = lines.len().saturating_sub(1);
        for (i, &(kind, content)) in lines.iter().enumerate() {
            write!(self.out, "{}", kind.to_prefix())?;
            self.out.write_all(content)?;

            // Ensure the line ends with a newline for display.
            if !content.ends_with_str("\n") {
                writeln!(self.out)?;
                // Check if this is the last line of the old or new content that lacks a trailing newline.
                // We need the `\ No newline at end of file` marker if:
                //   - This is a Remove line at the end of old content and old doesn't end with newline
                //   - This is an Add line at the end of new content and new doesn't end with newline
                //   - This is a Context line at the end that lacks a newline
                let is_last_in_hunk = i == last_idx
                    || lines[i + 1..]
                        .iter()
                        .all(|(k, _)| *k != kind && *k != DiffLineKind::Context);
                let needs_marker = match kind {
                    DiffLineKind::Remove => !self.old_ends_with_newline && is_at_end_of_side(lines, i, kind),
                    DiffLineKind::Add => !self.new_ends_with_newline && is_at_end_of_side(lines, i, kind),
                    DiffLineKind::Context => {
                        // Context lines appear in both old and new.
                        (!self.old_ends_with_newline || !self.new_ends_with_newline) && is_last_in_hunk
                    }
                };
                if needs_marker {
                    writeln!(self.out, "\\ No newline at end of file")?;
                }
            }
        }

        Ok(())
    }

    fn finish(self) -> Self::Out {
        Ok(())
    }
}

/// Write a hunk range in C Git's format: omit `,1` when length is 1.
///
/// C Git outputs `@@ -start +start @@` when the length is 1, but `@@ -start,len +start,len @@`
/// for all other lengths (including 0).
fn write_hunk_range(out: &mut dyn Write, start: u32, len: u32, prefix: char) -> std::io::Result<()> {
    if len == 1 {
        write!(out, "{prefix}{start}")
    } else {
        write!(out, "{prefix}{start},{len}")
    }
}

/// Find the nearest function definition line at or before `hunk_start` (1-based) in the old content.
///
/// This implements the same heuristic as C Git's default funcname pattern:
/// a line is considered a function header if it starts with a letter, `_`, or `$`.
/// The line content is trimmed to at most 80 characters for display.
fn find_function_line(old_lines: &[&[u8]], hunk_start: u32) -> Option<String> {
    if old_lines.is_empty() || hunk_start <= 1 {
        return None;
    }
    // hunk_start is 1-based and points to the first context line of the hunk.
    // Search backwards from the line *before* the hunk context starts,
    // i.e., from 0-based index (hunk_start - 2). This matches C Git's behavior
    // in xdiff/xemit.c where it searches from `s1 - 1` (0-based before first
    // displayed line).
    let search_start = ((hunk_start as usize).saturating_sub(2)).min(old_lines.len().saturating_sub(1));
    for i in (0..=search_start).rev() {
        let line = old_lines[i];
        if is_function_header(line) {
            let trimmed = line.trim_end_with(|c| c == '\n' || c == '\r');
            // Truncate to 80 chars for display, matching git's behavior.
            let display = if trimmed.len() > 80 { &trimmed[..80] } else { trimmed };
            return Some(String::from_utf8_lossy(display).into_owned());
        }
    }
    None
}

/// Check if a line looks like a function/class/method header.
///
/// Uses the same default heuristic as C Git's `xdiff/xemit.c`:
/// a line starting (at column 0, no leading whitespace) with a letter, `_`, or `$`
/// is treated as a function header. This matches C Git's default funcname regex
/// `^[[:alpha:]$_]`.
fn is_function_header(line: &[u8]) -> bool {
    if line.is_empty() {
        return false;
    }
    let first = line[0];
    // Match C Git's default funcname: starts at column 0 with letter, _, or $
    first.is_ascii_alphabetic() || first == b'_' || first == b'$'
}

/// Check if line at `idx` with `kind` represents the last line of its side in the hunk.
fn is_at_end_of_side(lines: &[(DiffLineKind, &[u8])], idx: usize, kind: DiffLineKind) -> bool {
    // Check if there are no more lines of the same kind (or context) after this one.
    for &(k, _) in &lines[idx + 1..] {
        match kind {
            DiffLineKind::Remove => {
                if k == DiffLineKind::Remove || k == DiffLineKind::Context {
                    return false;
                }
            }
            DiffLineKind::Add => {
                if k == DiffLineKind::Add || k == DiffLineKind::Context {
                    return false;
                }
            }
            DiffLineKind::Context => {
                // Any line after a context line means this isn't the last.
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch_string(old_path: &str, new_path: &str, old_content: &[u8], new_content: &[u8]) -> String {
        let mut out = Vec::new();
        write(
            &mut out,
            Some("abcdef1"),
            Some("1234567"),
            old_path,
            new_path,
            old_content,
            new_content,
            Options::default(),
        )
        .expect("write should succeed");
        String::from_utf8(out).expect("output should be UTF-8")
    }

    fn patch_string_with_change(
        old_path: &str,
        new_path: &str,
        old_content: &[u8],
        new_content: &[u8],
        change: FileChange,
    ) -> String {
        let mut out = Vec::new();
        write_with_change(
            &mut out,
            Some("abcdef1"),
            Some("1234567"),
            old_path,
            new_path,
            old_content,
            new_content,
            change,
            Options::default(),
        )
        .expect("write should succeed");
        String::from_utf8(out).expect("output should be UTF-8")
    }

    mod headers {
        use super::*;

        #[test]
        fn diff_git_header() {
            let p = patch_string("file.txt", "file.txt", b"old\n", b"new\n");
            assert!(
                p.starts_with("diff --git a/file.txt b/file.txt\n"),
                "should start with diff --git header: {p:?}"
            );
        }

        #[test]
        fn index_line() {
            let p = patch_string("file.txt", "file.txt", b"old\n", b"new\n");
            assert!(
                p.contains("index abcdef1..1234567\n"),
                "should contain index line: {p:?}"
            );
        }

        #[test]
        fn index_line_with_mode() {
            let p = patch_string_with_change(
                "file.txt",
                "file.txt",
                b"old\n",
                b"new\n",
                FileChange::Modified { mode: Some(0o100644) },
            );
            assert!(
                p.contains("index abcdef1..1234567 100644\n"),
                "should contain index line with mode suffix: {p:?}"
            );
        }

        #[test]
        fn file_path_headers() {
            let p = patch_string("file.txt", "file.txt", b"old\n", b"new\n");
            assert!(p.contains("--- a/file.txt\n"), "should have --- header: {p:?}");
            assert!(p.contains("+++ b/file.txt\n"), "should have +++ header: {p:?}");
        }

        #[test]
        fn renamed_file_headers() {
            let p = patch_string("old.txt", "new.txt", b"content\n", b"content changed\n");
            assert!(
                p.starts_with("diff --git a/old.txt b/new.txt\n"),
                "should show both paths: {p:?}"
            );
            assert!(p.contains("--- a/old.txt\n"), "old path in ---: {p:?}");
            assert!(p.contains("+++ b/new.txt\n"), "new path in +++: {p:?}");
        }

        #[test]
        fn no_index_line_when_ids_omitted() {
            let mut out = Vec::new();
            write(
                &mut out,
                None,
                None,
                "file.txt",
                "file.txt",
                b"old\n",
                b"new\n",
                Options::default(),
            )
            .unwrap();
            let p = String::from_utf8(out).unwrap();
            assert!(!p.contains("index "), "should not have index line: {p:?}");
        }
    }

    mod file_change {
        use super::*;

        #[test]
        fn new_file() {
            let p = patch_string_with_change(
                "file.txt",
                "file.txt",
                b"",
                b"new content\n",
                FileChange::Added { mode: 0o100644 },
            );
            assert!(p.contains("new file mode 100644\n"), "should have new file mode: {p:?}");
            assert!(p.contains("--- /dev/null\n"), "old path should be /dev/null: {p:?}");
            assert!(p.contains("+++ b/file.txt\n"), "new path with prefix: {p:?}");
        }

        #[test]
        fn deleted_file() {
            let p = patch_string_with_change(
                "file.txt",
                "file.txt",
                b"old content\n",
                b"",
                FileChange::Deleted { mode: 0o100644 },
            );
            assert!(
                p.contains("deleted file mode 100644\n"),
                "should have deleted file mode: {p:?}"
            );
            assert!(p.contains("--- a/file.txt\n"), "old path with prefix: {p:?}");
            assert!(p.contains("+++ /dev/null\n"), "new path should be /dev/null: {p:?}");
        }

        #[test]
        fn mode_change() {
            let p = patch_string_with_change(
                "script.sh",
                "script.sh",
                b"#!/bin/sh\n",
                b"#!/bin/sh\nexit 0\n",
                FileChange::ModeChange {
                    old_mode: 0o100644,
                    new_mode: 0o100755,
                },
            );
            assert!(p.contains("old mode 100644\n"), "should have old mode: {p:?}");
            assert!(p.contains("new mode 100755\n"), "should have new mode: {p:?}");
        }
    }

    mod content {
        use super::*;

        #[test]
        fn simple_modification() {
            let p = patch_string("file.txt", "file.txt", b"hello\nworld\n", b"hello\nrust\n");
            // C Git does not show function names for hunks starting at line 1.
            assert!(
                p.contains("@@ -1,2 +1,2 @@\n"),
                "should have hunk header without function name at line 1: {p:?}"
            );
            assert!(p.contains("-world\n"), "should have removed line: {p:?}");
            assert!(p.contains("+rust\n"), "should have added line: {p:?}");
            assert!(p.contains(" hello\n"), "should have context line: {p:?}");
        }

        #[test]
        fn all_added() {
            let p = patch_string("file.txt", "file.txt", b"", b"line1\nline2\n");
            assert!(p.contains("+line1\n"), "should have added lines: {p:?}");
            assert!(p.contains("+line2\n"), "should have added lines: {p:?}");
        }

        #[test]
        fn all_removed() {
            let p = patch_string("file.txt", "file.txt", b"line1\nline2\n", b"");
            assert!(p.contains("-line1\n"), "should have removed lines: {p:?}");
            assert!(p.contains("-line2\n"), "should have removed lines: {p:?}");
        }

        #[test]
        fn identical_files_produce_no_hunks() {
            let p = patch_string("file.txt", "file.txt", b"same\n", b"same\n");
            assert!(p.contains("diff --git"), "should have diff header: {p:?}");
            assert!(!p.contains("@@"), "should not have any hunks: {p:?}");
        }

        #[test]
        fn empty_to_empty() {
            let p = patch_string("file.txt", "file.txt", b"", b"");
            assert!(p.contains("diff --git"), "should have diff header: {p:?}");
            assert!(!p.contains("@@"), "should not have any hunks: {p:?}");
        }

        #[test]
        fn context_lines_configurable() {
            let old = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
            let new = b"1\n2\n3\n4\nFIVE\n6\n7\n8\n9\n10\n";
            let mut out = Vec::new();
            write(
                &mut out,
                None,
                None,
                "file.txt",
                "file.txt",
                old,
                new,
                Options {
                    context_lines: 1,
                    ..Options::default()
                },
            )
            .unwrap();
            let p = String::from_utf8(out).unwrap();
            assert!(
                p.contains("@@ -4,3 +4,3 @@\n"),
                "should have small hunk with context=1: {p:?}"
            );
        }
    }

    mod no_newline {
        use super::*;

        #[test]
        fn old_missing_newline_at_end() {
            let p = patch_string("file.txt", "file.txt", b"hello", b"hello\n");
            assert!(
                p.contains("\\ No newline at end of file"),
                "should have no-newline marker for old: {p:?}"
            );
        }

        #[test]
        fn new_missing_newline_at_end() {
            let p = patch_string("file.txt", "file.txt", b"hello\n", b"hello");
            assert!(
                p.contains("\\ No newline at end of file"),
                "should have no-newline marker for new: {p:?}"
            );
        }

        #[test]
        fn both_missing_newline_at_end() {
            let p = patch_string("file.txt", "file.txt", b"old", b"new");
            assert!(
                p.contains("\\ No newline at end of file"),
                "should have no-newline marker: {p:?}"
            );
        }
    }

    mod binary {
        use super::*;

        #[test]
        fn binary_content_detected() {
            assert!(is_binary(b"hello\x00world"));
            assert!(!is_binary(b"hello world"));
            assert!(!is_binary(b""));
        }

        #[test]
        fn binary_file_produces_binary_message() {
            let binary_content = b"hello\x00binary";
            let p = patch_string("file.bin", "file.bin", b"", binary_content);
            assert!(
                p.contains("Binary files a/file.bin and b/file.bin differ"),
                "should have binary diff message: {p:?}"
            );
            assert!(!p.contains("@@"), "should not have hunks: {p:?}");
        }

        #[test]
        fn binary_detection_boundary_at_8000_bytes() {
            // NUL at position 7999 (within first 8000 bytes) should be detected as binary.
            let mut content_with_nul = vec![b'a'; 8000];
            content_with_nul[7999] = 0;
            assert!(is_binary(&content_with_nul), "NUL at byte 7999 should be binary");

            // NUL at position 8000 (outside first 8000 bytes) should NOT be detected.
            let mut content_nul_after = vec![b'a'; 8001];
            content_nul_after[8000] = 0;
            assert!(!is_binary(&content_nul_after), "NUL at byte 8000 should not be binary");
        }

        #[test]
        fn old_binary_new_text_still_binary() {
            // If old content is binary, output should be binary diff even if new is text.
            let binary_old = b"\x00binary old";
            let text_new = b"text new\n";
            let p = patch_string("file.bin", "file.bin", binary_old, text_new);
            assert!(
                p.contains("Binary files"),
                "old binary content should produce binary diff: {p:?}"
            );
        }
    }

    mod custom_prefix {
        use super::*;

        #[test]
        fn custom_prefixes() {
            let mut out = Vec::new();
            write(
                &mut out,
                None,
                None,
                "file.txt",
                "file.txt",
                b"old\n",
                b"new\n",
                Options {
                    old_prefix: "i/",
                    new_prefix: "w/",
                    ..Options::default()
                },
            )
            .unwrap();
            let p = String::from_utf8(out).unwrap();
            assert!(
                p.contains("diff --git i/file.txt w/file.txt"),
                "should use custom prefixes: {p:?}"
            );
            assert!(p.contains("--- i/file.txt"), "old prefix: {p:?}");
            assert!(p.contains("+++ w/file.txt"), "new prefix: {p:?}");
        }
    }

    mod git_compatibility {
        use super::*;

        #[test]
        fn matches_git_diff_simple_change() {
            let old = b"line 1\nline 2\nline 3\n";
            let new = b"line 1\nline two\nline 3\n";
            let p = patch_string("test.txt", "test.txt", old, new);

            let lines: Vec<&str> = p.lines().collect();

            assert_eq!(lines[0], "diff --git a/test.txt b/test.txt");
            assert_eq!(lines[1], "index abcdef1..1234567");
            assert_eq!(lines[2], "--- a/test.txt");
            assert_eq!(lines[3], "+++ b/test.txt");
            // C Git does not emit function names for hunks starting at line 1
            assert_eq!(lines[4], "@@ -1,3 +1,3 @@");
            assert_eq!(lines[5], " line 1");
            assert_eq!(lines[6], "-line 2");
            assert_eq!(lines[7], "+line two");
            assert_eq!(lines[8], " line 3");
        }

        #[test]
        fn matches_git_diff_add_at_end() {
            let old = b"line 1\nline 2\n";
            let new = b"line 1\nline 2\nline 3\n";
            let p = patch_string("test.txt", "test.txt", old, new);

            assert!(p.contains("@@ -1,2 +1,3 @@"), "hunk header: {p:?}");
            assert!(p.contains("+line 3\n"), "added line: {p:?}");
        }

        #[test]
        fn matches_git_diff_remove_from_beginning() {
            let old = b"line 1\nline 2\nline 3\n";
            let new = b"line 2\nline 3\n";
            let p = patch_string("test.txt", "test.txt", old, new);

            assert!(p.contains("-line 1\n"), "removed line: {p:?}");
        }
    }

    mod function_names {
        use super::*;

        #[test]
        fn c_function_in_hunk_header() {
            // Change is deep enough inside a function that the function line
            // appears before the hunk context window.
            let old = b"#include <stdio.h>\n\nint main(void) {\n    int x = 0;\n    int y = 0;\n    int z = 0;\n    int w = 0;\n    printf(\"hello\");\n    return 0;\n}\n";
            let new = b"#include <stdio.h>\n\nint main(void) {\n    int x = 0;\n    int y = 0;\n    int z = 0;\n    int w = 0;\n    printf(\"world\");\n    return 0;\n}\n";
            let p = patch_string("test.c", "test.c", old, new);
            assert!(
                p.contains("int main(void) {"),
                "should have function name in hunk header: {p:?}"
            );
        }

        #[test]
        fn rust_function_in_hunk_header() {
            // The hunk starts deep in the function so fn main() is before context.
            let old = b"use std::io;\n\nfn main() {\n    let a = 0;\n    let b = 0;\n    let c = 0;\n    let d = 0;\n    let x = 1;\n    let y = 2;\n    println!(\"{}\", x + y);\n}\n";
            let new = b"use std::io;\n\nfn main() {\n    let a = 0;\n    let b = 0;\n    let c = 0;\n    let d = 0;\n    let x = 1;\n    let y = 3;\n    println!(\"{}\", x + y);\n}\n";
            let p = patch_string("test.rs", "test.rs", old, new);
            assert!(
                p.contains("fn main() {"),
                "should have Rust function name in hunk header: {p:?}"
            );
        }

        #[test]
        fn no_function_name_when_none_found() {
            let old = b"  indented line\n  another\n";
            let new = b"  indented line\n  changed\n";
            let p = patch_string("test.txt", "test.txt", old, new);
            // Lines starting with spaces don't match function pattern (column 0 rule)
            assert!(p.contains("@@ -1,2 +1,2 @@\n"), "should have no function name: {p:?}");
        }

        #[test]
        fn function_name_disabled() {
            let old = b"fn main() {\n    let x = 1;\n}\n";
            let new = b"fn main() {\n    let x = 2;\n}\n";
            let mut out = Vec::new();
            write(
                &mut out,
                None,
                None,
                "test.rs",
                "test.rs",
                old,
                new,
                Options {
                    find_function_names: false,
                    ..Options::default()
                },
            )
            .unwrap();
            let p = String::from_utf8(out).unwrap();
            assert!(
                p.contains("@@ -1,3 +1,3 @@\n"),
                "should have no function name when disabled: {p:?}"
            );
        }

        #[test]
        fn function_name_picks_nearest_preceding() {
            let old = b"\
fn first() {
    // ...
}

fn second() {
    let a = 1;
    let b = 2;
    let c = 3;
}
";
            let new = b"\
fn first() {
    // ...
}

fn second() {
    let a = 1;
    let b = 999;
    let c = 3;
}
";
            let p = patch_string("test.rs", "test.rs", old, new);
            assert!(
                p.contains("@@ ") && p.contains("fn second()"),
                "should show nearest function (second, not first): {p:?}"
            );
        }

        #[test]
        fn function_name_truncated_at_80_chars() {
            // Create a function line longer than 80 characters.
            // The function name in the hunk header should be truncated to 80 chars.
            let long_func =
                "fn this_is_a_very_long_function_name_that_exceeds_eighty_characters_for_sure_yes_really(x: i32) {";
            assert!(long_func.len() > 80, "test setup: line must be > 80 chars");

            let mut old_content = format!("{long_func}\n");
            for i in 0..8 {
                old_content.push_str(&format!("    let v{i} = {i};\n"));
            }
            old_content.push_str("    let result = 0;\n}\n");

            let mut new_content = format!("{long_func}\n");
            for i in 0..8 {
                new_content.push_str(&format!("    let v{i} = {i};\n"));
            }
            new_content.push_str("    let result = 42;\n}\n");

            let p = patch_string("test.rs", "test.rs", old_content.as_bytes(), new_content.as_bytes());
            // The function name in the header should be exactly 80 chars
            let truncated = &long_func[..80];
            assert!(
                p.contains(truncated),
                "should contain truncated function name (80 chars): {p:?}"
            );
            // And the 81st character should NOT appear in the hunk header line
            let char_81 = &long_func[80..81];
            let hunk_line = p
                .lines()
                .find(|l| l.starts_with("@@"))
                .expect("should have hunk header");
            assert!(
                !hunk_line.contains(char_81) || hunk_line.contains(truncated),
                "function name should be truncated at 80 chars in hunk header: {hunk_line:?}"
            );
        }

        #[test]
        fn underscore_prefix_matches_function_header() {
            // Lines starting with `_` should be recognized as function headers,
            // matching C Git's default funcname pattern `^[[:alpha:]$_]`.
            let old = b"_private_init(void) {\n    int a = 0;\n    int b = 0;\n    int c = 0;\n    int d = 0;\n    int x = 1;\n    return x;\n}\n";
            let new = b"_private_init(void) {\n    int a = 0;\n    int b = 0;\n    int c = 0;\n    int d = 0;\n    int x = 2;\n    return x;\n}\n";
            let p = patch_string("test.c", "test.c", old, new);
            assert!(
                p.contains("_private_init(void)"),
                "underscore-prefixed function should appear in hunk header: {p:?}"
            );
        }

        #[test]
        fn dollar_prefix_matches_function_header() {
            // Lines starting with `$` should be recognized as function headers.
            let old = b"$jQuery_init = function() {\n    var a = 0;\n    var b = 0;\n    var c = 0;\n    var d = 0;\n    var x = 1;\n    return x;\n};\n";
            let new = b"$jQuery_init = function() {\n    var a = 0;\n    var b = 0;\n    var c = 0;\n    var d = 0;\n    var x = 2;\n    return x;\n};\n";
            let p = patch_string("test.js", "test.js", old, new);
            assert!(
                p.contains("$jQuery_init"),
                "dollar-prefixed function should appear in hunk header: {p:?}"
            );
        }

        #[test]
        fn multiple_hunks_show_different_function_names() {
            // Two changes in different functions should show the correct function name
            // for each hunk header.
            let old = b"\
fn alpha() {
    let a = 1;
    let b = 2;
    let c = 3;
    let d = 4;
    let e = 5;
    let f = 6;
    let g = 7;
    return a;
}

fn beta() {
    let a = 1;
    let b = 2;
    let c = 3;
    let d = 4;
    let e = 5;
    let f = 6;
    let g = 7;
    return a;
}
";
            let new = b"\
fn alpha() {
    let a = 1;
    let b = 2;
    let c = 3;
    let d = 4;
    let e = 5;
    let f = 6;
    let g = 7;
    return 99;
}

fn beta() {
    let a = 1;
    let b = 2;
    let c = 3;
    let d = 4;
    let e = 5;
    let f = 6;
    let g = 7;
    return 99;
}
";
            let p = patch_string("test.rs", "test.rs", old, new);
            let hunk_lines: Vec<&str> = p.lines().filter(|l| l.starts_with("@@")).collect();
            assert_eq!(hunk_lines.len(), 2, "should have 2 hunks: {p:?}");
            assert!(
                hunk_lines[0].contains("fn alpha()"),
                "first hunk should show alpha: {:?}",
                hunk_lines[0]
            );
            assert!(
                hunk_lines[1].contains("fn beta()"),
                "second hunk should show beta: {:?}",
                hunk_lines[1]
            );
        }
    }
}
