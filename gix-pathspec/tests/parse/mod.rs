use std::collections::HashMap;

use bstr::{BStr, BString, ByteSlice};
use gix_attributes::State;
use gix_pathspec::{MagicSignature, Pattern, SearchMode, TryIntoPathspec};
use std::sync::LazyLock;

#[test]
fn baseline() {
    for (pattern, exit_code) in BASELINE.iter() {
        let res = gix_pathspec::parse(pattern, Default::default());
        assert_eq!(
            res.is_ok(),
            *exit_code == 0,
            "{pattern:?} disagrees with baseline: {res:?}"
        );
        if let Ok(pat) = res {
            let actual = pat.to_bstring();
            assert_eq!(
                pat,
                gix_pathspec::parse(actual.as_ref(), Default::default()).expect("still valid"),
                "{pattern} != {actual}: display must roundtrip into actual pattern"
            );
        }
        let p = gix_pathspec::Pattern::from_literal(pattern, Default::default());
        assert!(matches!(p.search_mode, SearchMode::Literal));
    }
}

#[test]
fn construct_programmatically() {
    let pattern = Pattern::from_components(
        "src/**",
        MagicSignature::TOP | MagicSignature::ICASE,
        SearchMode::PathAwareGlob,
        Vec::new(),
    );

    assert_eq!(pattern.path(), "src/**");
    assert_eq!(pattern.signature, MagicSignature::TOP | MagicSignature::ICASE);
    assert_eq!(pattern.search_mode, SearchMode::PathAwareGlob);
    assert_eq!(
        gix_pathspec::parse(pattern.to_bstring().as_ref(), Default::default()).expect("valid display form"),
        pattern,
        "structured patterns still round-trip through the display form"
    );

    let nil = Pattern::nil();
    assert!(nil.is_nil());
    assert_eq!(nil.to_bstring(), ":");
}

#[test]
fn try_into_pathspec_accepts_parsed_and_ready_made_patterns() {
    let defaults = Default::default();
    let parsed = "src/**"
        .try_into_pathspec(defaults)
        .expect("string-like values are parsed");
    assert_eq!(parsed.path(), "src/**");

    let ready = Pattern::from_components("README.md", MagicSignature::ICASE, SearchMode::Literal, Vec::new());
    assert_eq!(ready.clone().try_into_pathspec(defaults).expect("infallible"), ready);
    assert_eq!((&ready).try_into_pathspec(defaults).expect("infallible"), ready);
}

#[test]
fn from_file_parses_lf_crlf_and_quoted_entries() {
    let patterns = gix_pathspec::parse::from_file(
        br#"src/*.rs
"spaced path"
"quoted" trailing bytes ignored by git
:(literal)raw[chars]
trailing-cr
"#,
        Default::default(),
        Default::default(),
    )
    .expect("valid pathspec file");

    let paths = patterns.iter().map(gix_pathspec::Pattern::path).collect::<Vec<_>>();
    assert_eq!(paths, ["src/*.rs", "spaced path", "quoted", "raw[chars]", "trailing-cr"]);
    assert_eq!(patterns[3].search_mode, SearchMode::Literal);

    let patterns =
        gix_pathspec::parse::from_file(b"Cargo.toml\r\nREADME.md\r\n", Default::default(), Default::default())
            .expect("valid CRLF pathspec file");
    let paths = patterns.iter().map(gix_pathspec::Pattern::path).collect::<Vec<_>>();
    assert_eq!(paths, ["Cargo.toml", "README.md"]);
}

#[test]
fn from_file_parses_nul_separated_entries_without_unquoting() {
    let options = gix_pathspec::parse::file::Options {
        separator: gix_pathspec::parse::file::Separator::Nul,
        allow_quoted_strings: false,
    };

    let patterns = gix_pathspec::parse::from_file(
        b"\"literal quotes\"\0line\nbreak\0:(glob)src/**\0",
        Default::default(),
        options,
    )
    .expect("valid NUL-separated pathspec file");

    let paths = patterns.iter().map(gix_pathspec::Pattern::path).collect::<Vec<_>>();
    assert_eq!(paths, ["\"literal quotes\"", "line\nbreak", "src/**"]);
    assert_eq!(patterns[2].search_mode, SearchMode::PathAwareGlob);
}

#[test]
fn from_file_reports_entry_errors() {
    let err = gix_pathspec::parse::from_file(b"valid\n\n", Default::default(), Default::default()).unwrap_err();
    assert!(err.to_string().contains("entry 2"));
}

mod invalid;
mod valid;

/// A way to specify expectations more easily by simplifying assignments.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedPattern {
    path: BString,
    signature: MagicSignature,
    search_mode: SearchMode,
    attributes: Vec<(BString, State)>,
}

impl From<Pattern> for NormalizedPattern {
    fn from(p: Pattern) -> Self {
        NormalizedPattern {
            path: p.path().to_owned(),
            signature: p.signature,
            search_mode: p.search_mode,
            attributes: p
                .attributes
                .into_iter()
                .map(|attr| (attr.name.as_str().into(), attr.state))
                .collect(),
        }
    }
}

static BASELINE: LazyLock<HashMap<BString, usize>> = LazyLock::new(|| {
    let base = gix_testtools::scripted_fixture_read_only("parse_baseline.sh").unwrap();

    (|| -> crate::Result<_> {
        let mut map = HashMap::new();
        let baseline = std::fs::read(base.join("baseline.git"))?;
        let mut lines = baseline.lines();
        while let Some(spec) = lines.next() {
            let exit_code = lines.next().expect("two lines per baseline").to_str()?.parse()?;
            map.insert(spec.into(), exit_code);
        }
        Ok(map)
    })()
    .unwrap()
});

fn check_valid_inputs<'a>(inputs: impl IntoIterator<Item = (&'a str, NormalizedPattern)>) {
    for (input, expected) in inputs.into_iter() {
        assert!(
            check_against_baseline(input),
            "This pathspec is invalid in git: {input}"
        );

        let pattern = gix_pathspec::parse(input.as_bytes(), Default::default())
            .unwrap_or_else(|_| panic!("parsing should not fail with pathspec {input}"));
        let pattern: NormalizedPattern = pattern.into();
        assert_eq!(pattern, expected, "while checking input: \"{input}\"");
    }
}

fn check_against_baseline(pathspec: &str) -> bool {
    let key: &BStr = pathspec.into();
    let base = BASELINE
        .get(key)
        .unwrap_or_else(|| panic!("missing baseline for pathspec: {pathspec:?}"));
    *base == 0
}
