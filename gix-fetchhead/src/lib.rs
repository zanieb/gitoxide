//! Parse and write `.git/FETCH_HEAD` files.
//!
//! `FETCH_HEAD` consists of one line per fetched head. Each line starts with an
//! object id, followed by tab-separated metadata that tells Git if the entry is
//! eligible for merge and describes where it came from.
#![deny(missing_docs, rust_2018_idioms)]
#![forbid(unsafe_code)]

use std::{error, fmt, io};

/// A single `FETCH_HEAD` line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    /// The fetched object id.
    pub id: gix_hash::ObjectId,
    /// If true, this entry is eligible for merge.
    ///
    /// Lines with Git's `not-for-merge` marker set this to `false`.
    pub for_merge: bool,
    /// The raw source description after the merge marker.
    ///
    /// Typical descriptions are phrases like `branch 'main' of <url>`, but this
    /// field is intentionally preserved as raw text.
    pub description: String,
}

/// The error returned by [`parse()`] or [`Line::from_str()`][std::str::FromStr::from_str].
#[derive(Debug)]
pub enum Error {
    /// The line is empty.
    EmptyLine {
        /// The one-based line number.
        line: usize,
    },
    /// The line does not have the tab separator after the object id.
    MissingDescription {
        /// The one-based line number.
        line: usize,
    },
    /// The object id could not be decoded.
    InvalidObjectId {
        /// The one-based line number.
        line: usize,
        /// The underlying object-id decode error.
        source: gix_hash::decode::Error,
    },
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Error::InvalidObjectId { source, .. } => Some(source),
            Error::EmptyLine { .. } | Error::MissingDescription { .. } => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EmptyLine { line } => write!(f, "FETCH_HEAD line {line} is empty"),
            Error::MissingDescription { line } => {
                write!(f, "FETCH_HEAD line {line} is missing its source description")
            }
            Error::InvalidObjectId { line, source } => {
                write!(f, "FETCH_HEAD line {line} has an invalid object id: {source}")
            }
        }
    }
}

impl Line {
    /// Create a new line from its components.
    pub fn new(id: gix_hash::ObjectId, for_merge: bool, description: impl Into<String>) -> Self {
        Line {
            id,
            for_merge,
            description: description.into(),
        }
    }

    /// Write this line in `FETCH_HEAD` format, including the trailing newline.
    pub fn write_to(&self, out: &mut impl io::Write) -> io::Result<()> {
        write!(out, "{}", self.id)?;
        out.write_all(b"\t")?;
        if !self.for_merge {
            out.write_all(b"not-for-merge")?;
        }
        out.write_all(b"\t")?;
        out.write_all(self.description.as_bytes())?;
        out.write_all(b"\n")
    }

    fn from_line(line: &str, line_number: usize) -> Result<Self, Error> {
        if line.is_empty() {
            return Err(Error::EmptyLine { line: line_number });
        }

        let (id_hex, metadata) = line
            .split_once('\t')
            .ok_or(Error::MissingDescription { line: line_number })?;
        let id = gix_hash::ObjectId::from_hex(id_hex.as_bytes()).map_err(|source| Error::InvalidObjectId {
            line: line_number,
            source,
        })?;
        let (for_merge, description) = match metadata.strip_prefix("not-for-merge\t") {
            Some(description) => (false, description),
            None => (true, metadata.strip_prefix('\t').unwrap_or(metadata)),
        };

        Ok(Line::new(id, for_merge, description))
    }
}

impl std::str::FromStr for Line {
    type Err = Error;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        Line::from_line(line, 1)
    }
}

/// Parse the entire contents of a `FETCH_HEAD` file.
pub fn parse(input: &str) -> Result<Vec<Line>, Error> {
    input
        .lines()
        .enumerate()
        .map(|(index, line)| Line::from_line(line, index + 1))
        .collect()
}

/// Write all `lines` in `FETCH_HEAD` format.
pub fn write_to<'a>(lines: impl IntoIterator<Item = &'a Line>, out: &mut impl io::Write) -> io::Result<()> {
    for line in lines {
        line.write_to(out)?;
    }
    Ok(())
}
