//! Parse and write git bundle headers.

use bstr::BString;
use gix_hash::{Kind, ObjectId};

use crate::{Header, Prerequisite, Ref, Version};

/// Errors that can occur when parsing a bundle header.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("bundle signature not recognized; expected '# v2 git bundle' or '# v3 git bundle'")]
    InvalidSignature,
    #[error("unexpected end of input while parsing bundle header")]
    UnexpectedEof,
    #[error("invalid prerequisite line: {line:?}")]
    InvalidPrerequisite { line: BString },
    #[error("invalid reference line: {line:?}")]
    InvalidRef { line: BString },
    #[error("bundle contains no references")]
    NoRefs,
    #[error("bundle header exceeds maximum of {limit} entries (refs + prerequisites)")]
    TooManyEntries {
        /// The configured limit.
        limit: usize,
    },
    #[error("failed to parse object id from hex: {source}")]
    ObjectId {
        #[source]
        source: gix_hash::decode::Error,
    },
    #[error("unsupported bundle object format capability: {format:?}")]
    UnsupportedObjectFormat {
        /// The object format value from the `object-format` capability.
        format: BString,
    },
    #[error("unsupported bundle capability: {capability:?}")]
    UnsupportedCapability {
        /// The unsupported capability.
        capability: BString,
    },
    #[error("invalid bundle filter capability: {spec:?}")]
    InvalidFilter {
        /// The invalid filter specification.
        spec: BString,
    },
    #[error("bundle object format is {actual}, but {expected} was requested")]
    ObjectFormatMismatch {
        /// The object format requested by the caller.
        expected: Kind,
        /// The object format advertised by the bundle.
        actual: Kind,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Header {
    /// Parse a bundle header from the given reader.
    ///
    /// After a successful call, `reader` is positioned at the start of the packfile data
    /// (right after the blank line separator).
    ///
    /// The `object_hash` specifies which hash algorithm is used for object ids.
    pub fn from_read(mut reader: impl std::io::BufRead, object_hash: gix_hash::Kind) -> Result<Self, Error> {
        let hex_len = object_hash.len_in_hex();

        // Read signature line.
        let mut line_buf = Vec::new();
        if reader.read_until(b'\n', &mut line_buf)? == 0 {
            return Err(Error::UnexpectedEof);
        }
        let signature = trim_line_ending(&line_buf);
        let version = match signature {
            b"# v2 git bundle" => Version::V2,
            b"# v3 git bundle" => Version::V3,
            _ => return Err(Error::InvalidSignature),
        };

        let mut capabilities = Vec::new();
        let mut prerequisites = Vec::new();
        let mut refs = Vec::new();

        // For v3, read capabilities (lines starting with '@') until we hit a blank line or
        // a prerequisite/ref line. The common layout is:
        //   # v3 git bundle
        //   @capability1
        //   @capability2
        //
        //   -<prerequisite>
        //   <ref>
        //
        //   <packdata>
        //
        // For v2, there are no capabilities and we go straight to prerequisites/refs.

        // Track whether we've already consumed the v3 capabilities section separator.
        // This prevents an infinite loop if a malformed v3 bundle contains consecutive blank lines.
        let mut v3_caps_terminated = false;

        // Security: limit the number of entries to prevent OOM from crafted input.
        // A single bundle with more than 100,000 refs or prerequisites is pathological.
        const MAX_ENTRIES: usize = 100_000;

        loop {
            line_buf.clear();
            if reader.read_until(b'\n', &mut line_buf)? == 0 {
                return Err(Error::UnexpectedEof);
            }
            let line = trim_line_ending(&line_buf);

            // Empty line marks end of header.
            if line.is_empty() {
                if version == Version::V3 && !v3_caps_terminated && refs.is_empty() && prerequisites.is_empty() {
                    // For v3, first blank line ends capabilities section. Continue to refs.
                    v3_caps_terminated = true;
                    continue;
                }
                break;
            }

            if prerequisites.len() + refs.len() >= MAX_ENTRIES {
                return Err(Error::TooManyEntries { limit: MAX_ENTRIES });
            }

            if let Some(rest) = line.strip_prefix(b"-") {
                // Prerequisite line: -<hex-oid> [<comment>]
                if rest.len() < hex_len {
                    return Err(Error::InvalidPrerequisite {
                        line: BString::from(line),
                    });
                }
                let hex = &rest[..hex_len];
                let id = ObjectId::from_hex(hex).map_err(|source| Error::ObjectId { source })?;
                let comment = if rest.len() > hex_len && rest[hex_len] == b' ' {
                    Some(BString::from(&rest[hex_len + 1..]))
                } else if rest.len() > hex_len {
                    return Err(Error::InvalidPrerequisite {
                        line: BString::from(line),
                    });
                } else {
                    None
                };
                prerequisites.push(Prerequisite { id, comment });
            } else if version == Version::V3 && refs.is_empty() && prerequisites.is_empty() && line.starts_with(b"@") {
                // V3 capability line (before any refs or prerequisites).
                let cap = &line[1..];
                validate_capability(cap, object_hash)?;
                capabilities.push(BString::from(cap));
            } else {
                // Reference line: <hex-oid> <refname>
                if line.len() < hex_len + 1 {
                    return Err(Error::InvalidRef {
                        line: BString::from(line),
                    });
                }
                let hex = &line[..hex_len];
                let id = ObjectId::from_hex(hex).map_err(|source| Error::ObjectId { source })?;
                if line[hex_len] != b' ' {
                    return Err(Error::InvalidRef {
                        line: BString::from(line),
                    });
                }
                let refname = &line[hex_len + 1..];
                if refname.is_empty() {
                    return Err(Error::InvalidRef {
                        line: BString::from(line),
                    });
                }
                refs.push(Ref {
                    id,
                    name: BString::from(refname),
                });
            }
        }

        if refs.is_empty() {
            return Err(Error::NoRefs);
        }

        Ok(Header {
            version,
            prerequisites,
            refs,
            capabilities,
        })
    }

    /// Write the bundle header to the given writer.
    ///
    /// This writes the signature, capabilities (v3), prerequisites, references,
    /// and the terminating blank line. After this call, the packfile data should
    /// be written directly to the same writer.
    pub fn write_to(&self, mut writer: impl std::io::Write) -> Result<(), std::io::Error> {
        match self.version {
            Version::V2 => writer.write_all(b"# v2 git bundle\n")?,
            Version::V3 => {
                writer.write_all(b"# v3 git bundle\n")?;
                for cap in &self.capabilities {
                    writer.write_all(b"@")?;
                    writer.write_all(cap.as_ref())?;
                    writer.write_all(b"\n")?;
                }
            }
        }

        for prereq in &self.prerequisites {
            write!(writer, "-{}", prereq.id)?;
            if let Some(comment) = &prereq.comment {
                writer.write_all(b" ")?;
                writer.write_all(comment.as_ref())?;
            }
            writer.write_all(b"\n")?;
        }

        for r in &self.refs {
            write!(writer, "{}", r.id)?;
            writer.write_all(b" ")?;
            writer.write_all(r.name.as_ref())?;
            writer.write_all(b"\n")?;
        }

        writer.write_all(b"\n")?;
        Ok(())
    }
}

pub(crate) fn validate_capability(capability: &[u8], object_hash: Kind) -> Result<(), CapabilityError> {
    if let Some(format) = capability.strip_prefix(b"object-format=") {
        let actual = std::str::from_utf8(format)
            .ok()
            .and_then(|format| format.parse().ok())
            .ok_or_else(|| CapabilityError::UnsupportedObjectFormat {
                format: BString::from(format),
            })?;
        if actual != object_hash {
            return Err(CapabilityError::ObjectFormatMismatch {
                expected: object_hash,
                actual,
            });
        }
        return Ok(());
    }

    if let Some(spec) = capability.strip_prefix(b"filter=") {
        if validate_filter_spec(spec, 0) {
            return Ok(());
        }
        return Err(CapabilityError::InvalidFilter {
            spec: BString::from(spec),
        });
    }

    Err(CapabilityError::UnsupportedCapability {
        capability: BString::from(capability),
    })
}

#[derive(Debug)]
pub(crate) enum CapabilityError {
    UnsupportedObjectFormat { format: BString },
    ObjectFormatMismatch { expected: Kind, actual: Kind },
    UnsupportedCapability { capability: BString },
    InvalidFilter { spec: BString },
}

impl From<CapabilityError> for Error {
    fn from(err: CapabilityError) -> Self {
        match err {
            CapabilityError::UnsupportedObjectFormat { format } => Error::UnsupportedObjectFormat { format },
            CapabilityError::ObjectFormatMismatch { expected, actual } => {
                Error::ObjectFormatMismatch { expected, actual }
            }
            CapabilityError::UnsupportedCapability { capability } => Error::UnsupportedCapability { capability },
            CapabilityError::InvalidFilter { spec } => Error::InvalidFilter { spec },
        }
    }
}

fn validate_filter_spec(spec: &[u8], depth: usize) -> bool {
    if spec.is_empty() || depth > 8 {
        return false;
    }
    if spec == b"blob:none" {
        return true;
    }
    if let Some(size) = spec.strip_prefix(b"blob:limit=") {
        return validate_scaled_number(size);
    }
    if let Some(kind) = spec.strip_prefix(b"object:type=") {
        return matches!(kind, b"tag" | b"commit" | b"tree" | b"blob");
    }
    if let Some(depth) = spec.strip_prefix(b"tree:") {
        return validate_decimal(depth);
    }
    if let Some(oid) = spec.strip_prefix(b"sparse:oid=") {
        return !oid.is_empty() && !oid.contains(&0);
    }
    if let Some(specs) = spec.strip_prefix(b"combine:") {
        return validate_filter_combination(specs, depth + 1);
    }
    false
}

fn validate_filter_combination(specs: &[u8], depth: usize) -> bool {
    if specs.is_empty() {
        return false;
    }
    specs.split(|byte| *byte == b'+').all(|spec| {
        !spec.is_empty()
            && percent_decode(spec)
                .as_deref()
                .is_some_and(|decoded| validate_filter_spec(decoded, depth))
    })
}

fn validate_scaled_number(value: &[u8]) -> bool {
    let number = value
        .strip_suffix(b"k")
        .or_else(|| value.strip_suffix(b"K"))
        .or_else(|| value.strip_suffix(b"m"))
        .or_else(|| value.strip_suffix(b"M"))
        .or_else(|| value.strip_suffix(b"g"))
        .or_else(|| value.strip_suffix(b"G"))
        .unwrap_or(value);
    validate_decimal(number)
}

fn validate_decimal(value: &[u8]) -> bool {
    !value.is_empty() && value.iter().all(u8::is_ascii_digit)
}

fn percent_decode(value: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(value.len());
    let mut pos = 0;
    while pos < value.len() {
        match value[pos] {
            b'%' => {
                let hex = value.get(pos + 1..pos + 3)?;
                let high = hex_value(hex[0])?;
                let low = hex_value(hex[1])?;
                out.push(high << 4 | low);
                pos += 3;
            }
            byte => {
                out.push(byte);
                pos += 1;
            }
        }
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn trim_line_ending(mut line: &[u8]) -> &[u8] {
    if line.ends_with(b"\n") {
        line = &line[..line.len() - 1];
    }
    if line.ends_with(b"\r") {
        line = &line[..line.len() - 1];
    }
    line
}

/// Parse a bundle header from a byte slice.
///
/// Returns the header and the number of bytes consumed (i.e., the offset where the
/// packfile data begins).
pub fn decode(data: &[u8], object_hash: gix_hash::Kind) -> Result<(Header, usize), Error> {
    let mut reader = CountingBufRead::new(data);
    let header = Header::from_read(&mut reader, object_hash)?;
    Ok((header, reader.bytes_consumed()))
}

/// A `BufRead` implementation over a byte slice that tracks exactly how many bytes
/// have been consumed by `read_line` / `read` calls, without the read-ahead issue
/// that `BufReader<Cursor<&[u8]>>` has.
struct CountingBufRead<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CountingBufRead<'a> {
    fn new(data: &'a [u8]) -> Self {
        CountingBufRead { data, pos: 0 }
    }

    fn bytes_consumed(&self) -> usize {
        self.pos
    }
}

impl std::io::Read for CountingBufRead<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = &self.data[self.pos..];
        let n = std::cmp::min(buf.len(), remaining.len());
        buf[..n].copy_from_slice(&remaining[..n]);
        self.pos += n;
        Ok(n)
    }
}

impl std::io::BufRead for CountingBufRead<'_> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        Ok(&self.data[self.pos..])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = std::cmp::min(self.pos + amt, self.data.len());
    }
}

/// Parse a bundle header and return it along with a reader positioned at the packfile data.
pub fn from_path(
    path: impl AsRef<std::path::Path>,
    object_hash: gix_hash::Kind,
) -> Result<(Header, std::io::BufReader<std::fs::File>), Error> {
    let file = std::fs::File::open(path.as_ref())?;
    let mut reader = std::io::BufReader::new(file);
    let header = Header::from_read(&mut reader, object_hash)?;
    Ok((header, reader))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_v2() {
        let header = Header {
            version: Version::V2,
            prerequisites: vec![Prerequisite {
                id: ObjectId::from_hex(b"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef").unwrap(),
                comment: Some(BString::from("initial commit")),
            }],
            refs: vec![
                Ref {
                    id: ObjectId::from_hex(b"abcdef0123456789abcdef0123456789abcdef01").unwrap(),
                    name: BString::from("refs/heads/main"),
                },
                Ref {
                    id: ObjectId::from_hex(b"1234567890abcdef1234567890abcdef12345678").unwrap(),
                    name: BString::from("refs/tags/v1.0"),
                },
            ],
            capabilities: Vec::new(),
        };

        let mut buf = Vec::new();
        header.write_to(&mut buf).unwrap();

        let (parsed, consumed) = decode(&buf, gix_hash::Kind::Sha1).unwrap();
        assert_eq!(consumed, buf.len());
        assert_eq!(parsed, header);
    }

    #[test]
    fn roundtrip_v3_with_capabilities() {
        let header = Header {
            version: Version::V3,
            prerequisites: Vec::new(),
            refs: vec![Ref {
                id: ObjectId::from_hex(b"abcdef0123456789abcdef0123456789abcdef01").unwrap(),
                name: BString::from("refs/heads/main"),
            }],
            capabilities: vec![BString::from("object-format=sha1")],
        };

        let mut buf = Vec::new();
        header.write_to(&mut buf).unwrap();

        let (parsed, _) = decode(&buf, gix_hash::Kind::Sha1).unwrap();
        assert_eq!(parsed, header);
    }

    #[test]
    fn parse_v2_minimal() {
        let data = b"# v2 git bundle\nabcdef0123456789abcdef0123456789abcdef01 refs/heads/main\n\n";
        let (header, consumed) = decode(data.as_slice(), gix_hash::Kind::Sha1).unwrap();
        assert_eq!(header.version, Version::V2);
        assert_eq!(header.prerequisites.len(), 0);
        assert_eq!(header.refs.len(), 1);
        assert_eq!(consumed, data.len());
    }

    #[test]
    fn parse_v2_with_prerequisites() {
        let data = b"# v2 git bundle\n-deadbeefdeadbeefdeadbeefdeadbeefdeadbeef some comment\nabcdef0123456789abcdef0123456789abcdef01 refs/heads/main\n\n";
        let (header, _) = decode(data.as_slice(), gix_hash::Kind::Sha1).unwrap();
        assert_eq!(header.prerequisites.len(), 1);
        assert_eq!(
            header.prerequisites[0].comment.as_ref().map(|c| c.as_slice()),
            Some("some comment".as_bytes())
        );
    }

    #[test]
    fn error_on_no_refs() {
        let data = b"# v2 git bundle\n-deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\n\n";
        let result = decode(data.as_slice(), gix_hash::Kind::Sha1);
        assert!(result.is_err());
    }

    #[test]
    fn error_on_invalid_signature() {
        let data = b"# v1 git bundle\n";
        let result = decode(data.as_slice(), gix_hash::Kind::Sha1);
        assert!(matches!(result, Err(Error::InvalidSignature)));
    }
}
