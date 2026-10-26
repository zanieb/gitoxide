//! Read Git LFS pointer files stored in Git blobs.
#![forbid(unsafe_code)]

/// Build basic Git LFS download batch requests, validate responses and verify object contents.
#[cfg(feature = "download")]
pub mod download;

/// Select paths for LFS download and checkout.
#[cfg(feature = "download")]
pub mod filter;

/// An extension-free Git LFS pointer identifying an object and its size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "download", derive(serde::Serialize))]
pub struct Pointer<'a> {
    /// The lowercase SHA-256 digest of the contents, encoded in hexadecimal.
    pub oid: &'a str,
    /// The size of the contents in bytes.
    pub size: u64,
}

impl<'a> Pointer<'a> {
    /// Git LFS pointer files must be smaller than this many bytes.
    pub const SIZE_CUTOFF: usize = 1024;

    /// Read an extension-free v1 pointer, including the two historical version URLs.
    /// Treat an empty blob as empty contents. Return `None` for unrecognized or extended pointers.
    pub fn from_bytes(data: &'a [u8]) -> Option<Self> {
        if data.is_empty() {
            return Some(Self {
                oid: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                size: 0,
            });
        }
        if data.len() >= Self::SIZE_CUTOFF {
            return None;
        }
        let mut lines = std::str::from_utf8(data).ok()?.strip_suffix('\n')?.split('\n');
        match lines.next()? {
            "version https://git-lfs.github.com/spec/v1"
            | "version https://hawser.github.com/spec/v1"
            | "version http://git-media.io/v/2" => {}
            _ => return None,
        }
        let oid = lines.next()?.strip_prefix("oid sha256:")?;
        if oid.len() != 64 || !oid.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return None;
        }
        let size = lines.next()?.strip_prefix("size ")?;
        if lines.next().is_some()
            || !size.bytes().all(|b| b.is_ascii_digit())
            || (size.len() > 1 && size.starts_with('0'))
        {
            return None;
        }
        Some(Self {
            oid,
            size: size.parse().ok()?,
        })
    }
}
