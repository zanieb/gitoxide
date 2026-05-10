use std::str::FromStr;

use gix_fetchhead::Line;

fn sha1(hex: &str) -> gix_hash::ObjectId {
    gix_hash::ObjectId::from_hex(hex.as_bytes()).expect("valid object id")
}

#[test]
fn parse_fetch_head_lines() {
    let input = "\
1b8d9e6a408e480ae1912e919c37a26e5c46639d\t\tbranch 'main' of https://example.com/repo.git
43f695a9607f1f85f859f2ef944b785b5b6dd238\tnot-for-merge\tbranch 'topic' of https://example.com/repo.git
";

    let lines = gix_fetchhead::parse(input).expect("valid FETCH_HEAD");
    assert_eq!(
        lines,
        vec![
            Line::new(
                sha1("1b8d9e6a408e480ae1912e919c37a26e5c46639d"),
                true,
                "branch 'main' of https://example.com/repo.git"
            ),
            Line::new(
                sha1("43f695a9607f1f85f859f2ef944b785b5b6dd238"),
                false,
                "branch 'topic' of https://example.com/repo.git"
            ),
        ]
    );
}

#[test]
fn parse_and_write_round_trip() {
    let input = "\
1b8d9e6a408e480ae1912e919c37a26e5c46639d\t\tbranch 'main' of https://example.com/repo.git
43f695a9607f1f85f859f2ef944b785b5b6dd238\tnot-for-merge\tbranch 'topic' of https://example.com/repo.git
";

    let lines = gix_fetchhead::parse(input).expect("valid FETCH_HEAD");
    let mut out = Vec::new();
    gix_fetchhead::write_to(&lines, &mut out).expect("write succeeds");

    assert_eq!(String::from_utf8(out).expect("valid utf8"), input);
}

#[test]
fn line_from_str_reports_invalid_object_id() {
    let err = Line::from_str("invalid\tbranch 'main' of https://example.com/repo.git").unwrap_err();
    assert!(err.to_string().contains("line 1"));
    assert!(err.to_string().contains("invalid object id"));
}

#[test]
fn missing_description_is_an_error() {
    let err = gix_fetchhead::parse("1b8d9e6a408e480ae1912e919c37a26e5c46639d").unwrap_err();
    assert!(err.to_string().contains("missing its source description"));
}

#[test]
fn missing_merge_marker_is_an_error() {
    let err = Line::from_str("1b8d9e6a408e480ae1912e919c37a26e5c46639d\tbranch 'main' of https://example.com/repo.git")
        .unwrap_err();
    assert!(err.to_string().contains("invalid merge marker"));
}

#[test]
fn incomplete_not_for_merge_marker_is_an_error() {
    let err = Line::from_str("1b8d9e6a408e480ae1912e919c37a26e5c46639d\tnot-for-merge").unwrap_err();
    assert!(err.to_string().contains("invalid merge marker"));
}

#[cfg(feature = "sha256")]
#[test]
fn parse_sha256_object_ids() {
    let line = Line::from_str(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\t\tbranch 'main' of https://example.com/repo.git",
    )
    .expect("sha256 is enabled");

    assert_eq!(line.id.kind(), gix_hash::Kind::Sha256);
}
