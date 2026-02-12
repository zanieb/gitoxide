//! Parse and write git bundle headers.

use bstr::BString;
use gix_hash::ObjectId;

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
        let mut line_buf = String::new();
        if reader.read_line(&mut line_buf)? == 0 {
            return Err(Error::UnexpectedEof);
        }
        let signature = line_buf.trim_end_matches('\n').trim_end_matches('\r');
        let version = match signature {
            "# v2 git bundle" => Version::V2,
            "# v3 git bundle" => Version::V3,
            _ => return Err(Error::InvalidSignature),
        };

        let mut capabilities = Vec::new();
        let mut prerequisites = Vec::new();
        let mut refs = Vec::new();

        // For v3, read capabilities (lines starting with '@') until we hit a blank line or
        // a prerequisite/ref line. Capabilities end with a blank line.
        // Actually the v3 format is: after the signature, capabilities are lines that don't
        // start with '-' or a hex char. They end at a blank line, then prerequisites and refs
        // follow. But the most common layout is:
        //   # v3 git bundle
        //   @capability1
        //   @capability2
        //
        //   -<prerequisite>
        //   <ref>
        //
        //   <packdata>
        //
        // However, looking at git source (bundle.c), v3 capabilities are key=value lines
        // between the signature and the first blank line (the capabilities section).
        // For v2, there are no capabilities and we go straight to prerequisites/refs.

        // Track whether we've already consumed the v3 capabilities section separator.
        // This prevents an infinite loop if a malformed v3 bundle contains consecutive blank lines.
        let mut v3_caps_terminated = false;

        // Security: limit the number of entries to prevent OOM from crafted input.
        // A single bundle with more than 100,000 refs or prerequisites is pathological.
        const MAX_ENTRIES: usize = 100_000;

        loop {
            line_buf.clear();
            if reader.read_line(&mut line_buf)? == 0 {
                return Err(Error::UnexpectedEof);
            }
            let line = line_buf.trim_end_matches('\n').trim_end_matches('\r');

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

            if line.starts_with('-') {
                // Prerequisite line: -<hex-oid> [<comment>]
                let rest = &line[1..];
                if rest.len() < hex_len {
                    return Err(Error::InvalidPrerequisite {
                        line: BString::from(line.as_bytes()),
                    });
                }
                let hex = &rest[..hex_len];
                let id = ObjectId::from_hex(hex.as_bytes()).map_err(|source| Error::ObjectId { source })?;
                let comment = if rest.len() > hex_len && rest.as_bytes()[hex_len] == b' ' {
                    Some(BString::from(rest[hex_len + 1..].as_bytes()))
                } else {
                    None
                };
                prerequisites.push(Prerequisite { id, comment });
            } else if version == Version::V3
                && refs.is_empty()
                && prerequisites.is_empty()
                && !line.as_bytes()[0].is_ascii_hexdigit()
            {
                // V3 capability line (before any refs or prerequisites).
                // Multiple capabilities are allowed, e.g. @object-format=sha1 and @filter=blob:none.
                let cap = line.strip_prefix('@').unwrap_or(line);
                capabilities.push(BString::from(cap.as_bytes()));
            } else {
                // Reference line: <hex-oid> <refname>
                if line.len() < hex_len + 1 {
                    return Err(Error::InvalidRef {
                        line: BString::from(line.as_bytes()),
                    });
                }
                let hex = &line[..hex_len];
                let id = ObjectId::from_hex(hex.as_bytes()).map_err(|source| Error::ObjectId { source })?;
                if line.as_bytes()[hex_len] != b' ' {
                    return Err(Error::InvalidRef {
                        line: BString::from(line.as_bytes()),
                    });
                }
                let refname = &line[hex_len + 1..];
                refs.push(Ref {
                    id,
                    name: BString::from(refname.as_bytes()),
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
                if !self.capabilities.is_empty() {
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
