use bstr::BString;
use gix_hash::ObjectId;

use crate::{extension::Signature, util::split_at_byte_exclusive};

pub type Paths = Vec<ResolvePath>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvePath {
    /// relative to the root of the repository, or what would be stored in the index
    name: BString,

    /// 0 = ancestor/common, 1 = ours, 2 = theirs
    stages: [Option<Stage>; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage {
    mode: u32,
    id: ObjectId,
}

pub const SIGNATURE: Signature = *b"REUC";

pub fn decode(mut data: &[u8], object_hash: gix_hash::Kind) -> Option<Paths> {
    let hash_len = object_hash.len_in_bytes();
    let mut out = Vec::new();

    while !data.is_empty() {
        let (path, rest) = split_at_byte_exclusive(data, 0)?;
        data = rest;

        let mut modes = [0u32; 3];
        for mode in &mut modes {
            let (mode_ascii, rest) = split_at_byte_exclusive(data, 0)?;
            data = rest;
            *mode = u32::from_str_radix(std::str::from_utf8(mode_ascii).ok()?, 8).ok()?;
        }

        let mut stages = [None, None, None];
        for (mode, stage) in modes.iter().zip(stages.iter_mut()) {
            if *mode == 0 {
                continue;
            }
            let (hash, rest) = data.split_at_checked(hash_len)?;
            data = rest;
            *stage = Some(Stage {
                mode: *mode,
                id: ObjectId::from_bytes_or_panic(hash),
            });
        }

        out.push(ResolvePath {
            name: path.into(),
            stages,
        });
    }
    out.into()
}

/// Serialize a resolve-undo extension to `out`.
pub fn write_to(
    paths: &Paths,
    object_hash: gix_hash::Kind,
    mut out: impl std::io::Write,
) -> Result<(), std::io::Error> {
    use std::io::Write as _;

    let mut entries = Vec::new();
    for path in paths {
        entries.write_all(&path.name)?;
        entries.write_all(b"\0")?;
        for stage in &path.stages {
            let mode = stage.map_or(0, |stage| stage.mode);
            write!(entries, "{mode:o}\0")?;
        }
        for stage in path.stages.iter().flatten() {
            if stage.id.kind() != object_hash {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "resolve-undo object id length does not match index object hash",
                ));
            }
            entries.write_all(stage.id.as_bytes())?;
        }
    }

    out.write_all(&SIGNATURE)?;
    out.write_all(&(u32::try_from(entries.len()).expect("less than 4GB resolve-undo extension")).to_be_bytes())?;
    out.write_all(&entries)
}

#[cfg(test)]
mod tests {
    use super::{write_to, ResolvePath, Stage};

    #[test]
    fn write_to_rejects_object_hash_mismatch() {
        let paths = vec![ResolvePath {
            name: "file".into(),
            stages: [
                Some(Stage {
                    mode: 0o100644,
                    id: gix_hash::ObjectId::from_bytes_or_panic(&[0; 20]),
                }),
                None,
                None,
            ],
        }];
        let mut out = Vec::new();

        let err = write_to(&paths, gix_hash::Kind::Sha256, &mut out).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}
