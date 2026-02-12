use bstr::ByteSlice;
use proptest::prelude::*;

proptest! {
    #[test]
    fn parse_never_panics(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
        // URL parser must never panic on any byte sequence
        let _ = gix_url::parse(data.as_bstr());
    }

    #[test]
    fn parse_reparse_stability(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
        // If a URL parses successfully and serializes, re-parsing the serialized form
        // must also succeed
        if let Ok(url) = gix_url::parse(data.as_bstr()) {
            let serialized = url.to_bstring();
            let reparsed = gix_url::parse(serialized.as_bstr());
            prop_assert!(reparsed.is_ok(), "re-parsing serialized URL failed: {:?}", reparsed.err());
        }
    }

    #[test]
    fn expand_path_never_panics(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = gix_url::expand_path::parse(data.as_bstr());
    }

    #[test]
    fn host_argument_safe_blocks_dash_prefix(
        host in "-[a-zA-Z0-9]{1,20}",
    ) {
        // Hosts starting with '-' must be rejected by host_argument_safe()
        // to prevent command injection via SSH
        let url_str = format!("ssh://{host}/repo.git");
        if let Ok(url) = gix_url::parse(url_str.as_bytes().as_bstr()) {
            if let Some(safe_host) = url.host_argument_safe() {
                prop_assert!(!safe_host.starts_with("-"),
                    "host_argument_safe returned host starting with dash: {safe_host}");
            }
        }
    }

    #[test]
    fn valid_http_urls_roundtrip(
        host in "[a-z]{3,10}\\.[a-z]{2,4}",
        port in proptest::option::of(1u16..=65535),
        path in "/[a-z0-9/]{1,30}",
    ) {
        let url_str = if let Some(p) = port {
            format!("https://{host}:{p}{path}")
        } else {
            format!("https://{host}{path}")
        };
        let url = gix_url::parse(url_str.as_bytes().as_bstr())
            .expect("valid HTTP URL must parse");
        let serialized = url.to_bstring();
        let reparsed = gix_url::parse(serialized.as_bstr())
            .expect("roundtripped URL must parse");
        prop_assert_eq!(url.host(), reparsed.host(), "host mismatch after roundtrip");
        prop_assert_eq!(url.port, reparsed.port, "port mismatch after roundtrip");
        prop_assert_eq!(url.path, reparsed.path, "path mismatch after roundtrip");
    }
}
