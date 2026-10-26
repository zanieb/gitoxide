use bstr::{BStr, ByteSlice};
use gix_glob::{Pattern, pattern::Case, wildmatch};

/// Path selection from `lfs.fetchinclude`, `lfs.fetchexclude` and `core.ignoreCase`.
pub struct Filter {
    include: Vec<Pattern>,
    exclude: Vec<Pattern>,
    case: Case,
}

impl Filter {
    /// Parse the comma-separated lists of repository-relative wildmatch patterns.
    pub fn new(include: &[u8], exclude: &[u8], ignore_case: bool) -> Self {
        let patterns = |list: &[u8]| {
            list.split(|&b| b == b',')
                .filter_map(|s| Pattern::from_bytes_without_negation(s.trim_ascii()))
                .collect()
        };
        Self {
            include: patterns(include),
            exclude: patterns(exclude),
            case: if ignore_case { Case::Fold } else { Case::Sensitive },
        }
    }

    /// Return whether to fetch the object at a slash-separated path in the Git index.
    pub fn allows(&self, path: &BStr) -> bool {
        let matches = |pattern: &Pattern| {
            // Matching a directory selects its descendants as well.
            path.iter()
                .enumerate()
                .filter_map(|(i, &b)| (b == b'/').then_some(i))
                .chain([path.len()])
                .any(|end| {
                    let prefix = path[..end].as_bstr();
                    pattern.matches_repo_relative_path(
                        prefix,
                        prefix.rfind_byte(b'/').map(|i| i + 1),
                        Some(end != path.len()),
                        self.case,
                        wildmatch::Mode::NO_MATCH_SLASH_LITERAL,
                    )
                })
        };
        (self.include.is_empty() || self.include.iter().any(matches)) && !self.exclude.iter().any(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_lists_cover_directory_contents_and_exclusions() {
        let filter = Filter::new(b"/assets/**/a?.bin, data", b"tmp,*.old", true);
        for path in ["assets/a1.BIN", "assets/nested/a2.bin", "nested/DATA/keep"] {
            assert!(filter.allows(path.into()), "{path}");
        }
        for path in ["else/assets/a1.bin", "assets/ab.txt", "data/tmp/item", "data/a.old"] {
            assert!(!filter.allows(path.into()), "{path}");
        }
    }
}
