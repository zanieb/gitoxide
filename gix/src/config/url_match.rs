use crate::bstr::ByteSlice;

/// Rank matching URL sections by host, path and user specificity.
pub(crate) fn specificity(section: &crate::bstr::BStr, url: &gix_url::Url) -> Option<(usize, usize, bool)> {
    let mut pattern = parse_pattern(section)?;
    normalize(&mut pattern);
    let http = matches!(pattern.scheme, gix_url::Scheme::Https | gix_url::Scheme::Http);
    let ports = if http {
        (pattern.port_or_default(), url.port_or_default())
    } else {
        (pattern.port, url.port)
    };
    let path = (!(http && pattern.path_is_root())).then_some(&pattern.path);
    if pattern.scheme != url.scheme
        || !host_matches(pattern.host(), url.host())
        || ports.0 != ports.1
        || path.is_some_and(|p| !path_matches(p, &url.path))
        || pattern.user().is_some_and(|user| Some(user) != url.user())
    {
        return None;
    }
    let host_len = pattern
        .host()
        .map_or(0, |host| host.bytes().filter(|&c| c != b'*').count());
    Some((host_len, path.map_or(0, |p| p.len()), pattern.user().is_some()))
}

fn host_matches(pattern: Option<&str>, host: Option<&str>) -> bool {
    match (pattern, host) {
        (Some(pattern), Some(host)) => {
            let lfields = pattern.split('.');
            let rfields = host.split('.');
            if lfields.clone().count() != rfields.clone().count() {
                return false;
            }
            lfields
                .zip(rfields)
                .all(|(pat, value)| gix_glob::wildmatch(pat.into(), value.into(), gix_glob::wildmatch::Mode::empty()))
        }
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn path_matches(pattern: &crate::bstr::BString, path: &crate::bstr::BString) -> bool {
    let pattern = pattern.as_slice();
    let path = path.as_slice();
    path == pattern || path.starts_with(pattern) && path.get(pattern.len()) == Some(&b'/')
}

pub(crate) fn normalize(url: &mut gix_url::Url) {
    // Transport dispatch is case-sensitive, but Git's URL matcher follows URL semantics.
    url.scheme = match &url.scheme {
        gix_url::Scheme::HelperUrl(name) if name.eq_ignore_ascii_case("http") => gix_url::Scheme::Http,
        gix_url::Scheme::HelperUrl(name) if name.eq_ignore_ascii_case("https") => gix_url::Scheme::Https,
        scheme => scheme.clone(),
    };
    if matches!(url.scheme, gix_url::Scheme::Http | gix_url::Scheme::Https) && url.path.is_empty() {
        url.path = "/".into();
    }
    if !url.path_is_root() && url.path.ends_with(b"/") {
        url.path.pop();
    }
}

fn parse_pattern(pattern: &crate::bstr::BStr) -> Option<gix_url::Url> {
    let mut pattern = pattern.to_owned();
    if let Some(scheme_end) = pattern.find("://") {
        let scheme = &mut pattern[..scheme_end];
        if scheme.eq_ignore_ascii_case(b"http") || scheme.eq_ignore_ascii_case(b"https") {
            // Transport dispatch is case-sensitive, but Git's URL matcher follows URL semantics.
            scheme.make_ascii_lowercase();
        }
    }
    gix_url::parse(pattern).ok()
}
