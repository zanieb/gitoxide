use bstr::BString;

/// The status of a single reference update as reported by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusV1 {
    /// The reference update was accepted.
    Ok {
        /// The name of the reference.
        ref_name: BString,
    },
    /// The reference update was rejected.
    Ng {
        /// The name of the reference.
        ref_name: BString,
        /// The reason for the rejection.
        reason: BString,
    },
}

impl StatusV1 {
    /// Return `true` if the status indicates success.
    pub fn is_ok(&self) -> bool {
        matches!(self, StatusV1::Ok { .. })
    }

    /// Return the reference name regardless of the status.
    pub fn ref_name(&self) -> &bstr::BStr {
        use bstr::ByteSlice;
        match self {
            StatusV1::Ok { ref_name } | StatusV1::Ng { ref_name, .. } => ref_name.as_bstr(),
        }
    }
}

/// The result of parsing the unpack status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnpackStatus {
    /// The unpack was successful.
    Ok,
    /// The unpack failed with the given reason.
    Failed {
        /// The error reason reported by the server.
        reason: BString,
    },
}

impl UnpackStatus {
    /// Return `true` if the unpack was successful.
    pub fn is_ok(&self) -> bool {
        matches!(self, UnpackStatus::Ok)
    }
}

/// Parse the server's push response.
///
/// The response format (after the sideband is peeled) is:
///
/// ```text
/// unpack ok\n
/// ok <refname>\n
/// ...
/// ```
///
/// or
///
/// ```text
/// unpack <error-message>\n
/// ng <refname> <reason>\n
/// ...
/// ```
pub fn parse_v1(response: &[u8]) -> Result<(UnpackStatus, Vec<StatusV1>), Error> {
    use bstr::ByteSlice;

    let mut lines = response.lines();
    let first_line = lines.next().ok_or(Error::MissingUnpackStatus)?;

    let unpack_status = if first_line == b"unpack ok" {
        UnpackStatus::Ok
    } else if let Some(reason) = first_line.strip_prefix(b"unpack ") {
        UnpackStatus::Failed { reason: reason.into() }
    } else {
        return Err(Error::MissingUnpackStatus);
    };

    let mut statuses = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.starts_with(b"ok ") {
            statuses.push(StatusV1::Ok {
                ref_name: line[3..].into(),
            });
        } else if line.starts_with(b"ng ") {
            let rest = &line[3..];
            if let Some(space_pos) = rest.find_byte(b' ') {
                statuses.push(StatusV1::Ng {
                    ref_name: rest[..space_pos].into(),
                    reason: rest[space_pos + 1..].into(),
                });
            } else {
                statuses.push(StatusV1::Ng {
                    ref_name: rest.into(),
                    reason: BString::from("unknown reason"),
                });
            }
        }
    }
    Ok((unpack_status, statuses))
}

/// The error returned when parsing a push response.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("the server did not send the expected 'unpack' status line")]
    MissingUnpackStatus,
}
