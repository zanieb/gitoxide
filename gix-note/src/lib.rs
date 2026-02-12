#![deny(rust_2018_idioms)]
#![forbid(unsafe_code)]

//! A crate for reading and manipulating git notes.
//!
//! Git notes are annotations attached to objects (typically commits) without modifying
//! the objects themselves. They are stored as blobs in a notes tree, referenced by a
//! special ref (default: `refs/notes/commits`).
//!
//! In the notes tree, each note is a blob whose path is derived from the hex
//! representation of the annotated object's hash. Git uses a fanout scheme where
//! the path may be split into directory components (e.g., `aa/bbccddee...` for
//! 2/38 fanout).

use bstr::BString;
use gix_hash::ObjectId;

/// The default notes ref used by git.
pub const DEFAULT_REF: &str = "refs/notes/commits";

/// A note entry: a mapping from an annotated object to its note content blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The object this note annotates.
    pub target: ObjectId,
    /// The object id of the blob containing the note text.
    pub note_id: ObjectId,
}

/// An error that can occur during notes operations.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("Invalid note path component '{path}': not a valid hex object id")]
    InvalidNotePath { path: BString },
}

/// Parse a hex path from the notes tree into an `ObjectId`.
///
/// Notes trees use a fanout structure where the object's hex hash is split into
/// directory components. For example, with 2/38 fanout, the note for object
/// `aabbccddee...` is stored at path `aa/bbccddee...`.
///
/// This function concatenates all path components and parses the result as a hex hash.
pub fn target_from_path(path: &[u8], hash_kind: gix_hash::Kind) -> Result<ObjectId, Error> {
    // Strip directory separators to recover the full hex hash.
    let hex: Vec<u8> = path.iter().copied().filter(|&b| b != b'/').collect();
    if hex.len() != hash_kind.len_in_hex() {
        return Err(Error::InvalidNotePath {
            path: BString::from(path),
        });
    }
    ObjectId::from_hex(&hex).map_err(|_| Error::InvalidNotePath {
        path: BString::from(path),
    })
}

/// Convert an `ObjectId` to a path in the notes tree.
///
/// With `fanout_depth` of 0, returns the full hex hash as a single component.
/// With `fanout_depth` of 1, returns `aa/bbccddee...` (2/remaining).
/// With `fanout_depth` of 2, returns `aa/bb/ccddee...` (2/2/remaining).
pub fn path_from_target(target: &ObjectId, fanout_depth: u8) -> BString {
    let hex = target.to_hex().to_string();
    if fanout_depth == 0 {
        return BString::from(hex);
    }
    let mut result = String::with_capacity(hex.len() + fanout_depth as usize);
    let mut offset = 0;
    for _ in 0..fanout_depth {
        if offset + 2 <= hex.len() {
            if !result.is_empty() {
                result.push('/');
            }
            result.push_str(&hex[offset..offset + 2]);
            offset += 2;
        }
    }
    if offset < hex.len() {
        result.push('/');
        result.push_str(&hex[offset..]);
    }
    BString::from(result)
}

/// Iterate over entries in a notes tree.
///
/// This takes a parsed tree and returns note entries. It handles fanout
/// by recursively processing sub-trees. The `find_tree` closure is called
/// to load sub-trees when fanout directories are encountered.
///
/// Returns a list of `(target_oid, note_blob_oid)` pairs.
pub fn list_notes<E>(
    tree: &gix_object::Tree,
    hash_kind: gix_hash::Kind,
    hex_prefix: &str,
    find_tree: &mut impl FnMut(&ObjectId) -> Result<gix_object::Tree, E>,
) -> Result<Vec<Entry>, ListError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let mut entries = Vec::new();

    for entry in &tree.entries {
        if entry.mode.is_tree() {
            // This is a fanout directory. The directory name is part of the hex hash.
            // Fanout directory names are always valid ASCII hex, so lossy conversion is safe,
            // but we use the raw bytes directly to avoid any potential data loss.
            let component = std::str::from_utf8(&entry.filename).unwrap_or("");
            let sub_prefix = format!("{hex_prefix}{component}");
            let sub_tree = find_tree(&entry.oid).map_err(ListError::FindTree)?;
            let sub_entries = list_notes(&sub_tree, hash_kind, &sub_prefix, find_tree)?;
            entries.extend(sub_entries);
        } else if entry.mode.is_blob() {
            // This is a note blob. The filename is the remaining hex of the target.
            // Use raw bytes to avoid lossy UTF-8 conversion.
            let mut full_hex = Vec::with_capacity(hex_prefix.len() + entry.filename.len());
            full_hex.extend_from_slice(hex_prefix.as_bytes());
            full_hex.extend_from_slice(&entry.filename);
            if full_hex.len() == hash_kind.len_in_hex() {
                if let Ok(target) = ObjectId::from_hex(&full_hex) {
                    entries.push(Entry {
                        target,
                        note_id: entry.oid,
                    });
                }
            }
        }
    }

    Ok(entries)
}

/// An error from [`list_notes()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ListError<E: std::error::Error + Send + Sync + 'static> {
    #[error("Could not find sub-tree in notes tree")]
    FindTree(#[source] E),
}

/// Find the note for a specific target object in a notes tree.
///
/// Returns the blob OID of the note if found, or `None` if the target has no note.
pub fn find_note<E>(
    tree: &gix_object::Tree,
    target: &ObjectId,
    _hash_kind: gix_hash::Kind,
    find_tree: &mut impl FnMut(&ObjectId) -> Result<gix_object::Tree, E>,
) -> Result<Option<ObjectId>, FindError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let hex = target.to_hex().to_string();
    find_note_by_hex(tree, hex.as_bytes(), find_tree)
}

fn find_note_by_hex<E>(
    tree: &gix_object::Tree,
    remaining_hex: &[u8],
    find_tree: &mut impl FnMut(&ObjectId) -> Result<gix_object::Tree, E>,
) -> Result<Option<ObjectId>, FindError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    // Try direct match first (no fanout or final level).
    for entry in &tree.entries {
        if entry.filename.as_ref() as &[u8] == remaining_hex {
            if entry.mode.is_blob() {
                return Ok(Some(entry.oid));
            }
        }
    }

    // Try fanout: look for a 2-character directory prefix.
    if remaining_hex.len() > 2 {
        let prefix = &remaining_hex[..2];
        let rest = &remaining_hex[2..];
        for entry in &tree.entries {
            if entry.mode.is_tree() && entry.filename.as_ref() as &[u8] == prefix {
                let sub_tree = find_tree(&entry.oid).map_err(FindError::FindTree)?;
                return find_note_by_hex(&sub_tree, rest, find_tree);
            }
        }
    }

    Ok(None)
}

/// An error from [`find_note()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum FindError<E: std::error::Error + Send + Sync + 'static> {
    #[error("Could not find sub-tree in notes tree")]
    FindTree(#[source] E),
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix_object::tree::{Entry as TreeEntry, EntryKind, EntryMode};

    fn oid(hex: &str) -> ObjectId {
        ObjectId::from_hex(hex.as_bytes()).unwrap()
    }

    fn blob_mode() -> EntryMode {
        EntryKind::Blob.into()
    }

    fn tree_mode() -> EntryMode {
        EntryKind::Tree.into()
    }

    #[test]
    fn default_ref_value() {
        assert_eq!(DEFAULT_REF, "refs/notes/commits");
    }

    // --- target_from_path tests ---

    #[test]
    fn target_from_path_no_fanout() {
        let hex = "aabbccdd00112233445566778899aabbccddeeff";
        let result = target_from_path(hex.as_bytes(), gix_hash::Kind::Sha1).unwrap();
        assert_eq!(result, oid(hex));
    }

    #[test]
    fn target_from_path_with_fanout() {
        // 2/38 fanout: "aa/bbccdd00112233445566778899aabbccddeeff"
        let path = b"aa/bbccdd00112233445566778899aabbccddeeff";
        let result = target_from_path(path, gix_hash::Kind::Sha1).unwrap();
        assert_eq!(result, oid("aabbccdd00112233445566778899aabbccddeeff"));
    }

    #[test]
    fn target_from_path_with_double_fanout() {
        // 2/2/36 fanout: "aa/bb/ccdd00112233445566778899aabbccddeeff"
        let path = b"aa/bb/ccdd00112233445566778899aabbccddeeff";
        let result = target_from_path(path, gix_hash::Kind::Sha1).unwrap();
        assert_eq!(result, oid("aabbccdd00112233445566778899aabbccddeeff"));
    }

    #[test]
    fn target_from_path_wrong_length() {
        let result = target_from_path(b"aabb", gix_hash::Kind::Sha1);
        assert!(result.is_err());
    }

    #[test]
    fn target_from_path_invalid_hex() {
        let path = b"gghhccdd00112233445566778899aabbccddeeff";
        let result = target_from_path(path, gix_hash::Kind::Sha1);
        assert!(result.is_err());
    }

    // --- path_from_target tests ---

    #[test]
    fn path_from_target_no_fanout() {
        let id = oid("aabbccdd00112233445566778899aabbccddeeff");
        let path = path_from_target(&id, 0);
        assert_eq!(path, BString::from("aabbccdd00112233445566778899aabbccddeeff"));
    }

    #[test]
    fn path_from_target_fanout_1() {
        let id = oid("aabbccdd00112233445566778899aabbccddeeff");
        let path = path_from_target(&id, 1);
        assert_eq!(path, BString::from("aa/bbccdd00112233445566778899aabbccddeeff"));
    }

    #[test]
    fn path_from_target_fanout_2() {
        let id = oid("aabbccdd00112233445566778899aabbccddeeff");
        let path = path_from_target(&id, 2);
        assert_eq!(path, BString::from("aa/bb/ccdd00112233445566778899aabbccddeeff"));
    }

    #[test]
    fn path_roundtrip() {
        let id = oid("deadbeef00112233445566778899aabbccddeeff");
        for fanout in 0..=3 {
            let path = path_from_target(&id, fanout);
            let recovered = target_from_path(path.as_ref(), gix_hash::Kind::Sha1).unwrap();
            assert_eq!(recovered, id, "roundtrip failed for fanout={fanout}");
        }
    }

    // --- list_notes tests ---

    #[test]
    fn list_notes_flat_tree() {
        // A flat notes tree (no fanout): each entry is a blob named by the full hex hash.
        let target1 = oid("1111111111111111111111111111111111111111");
        let note1 = oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let target2 = oid("2222222222222222222222222222222222222222");
        let note2 = oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

        let tree = gix_object::Tree {
            entries: vec![
                TreeEntry {
                    mode: blob_mode(),
                    filename: target1.to_hex().to_string().into(),
                    oid: note1,
                },
                TreeEntry {
                    mode: blob_mode(),
                    filename: target2.to_hex().to_string().into(),
                    oid: note2,
                },
            ],
        };

        let mut find_tree =
            |_: &ObjectId| -> Result<gix_object::Tree, std::io::Error> { unreachable!("no sub-trees in flat layout") };

        let entries = list_notes(&tree, gix_hash::Kind::Sha1, "", &mut find_tree).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.contains(&Entry {
            target: target1,
            note_id: note1
        }));
        assert!(entries.contains(&Entry {
            target: target2,
            note_id: note2
        }));
    }

    #[test]
    fn list_notes_with_fanout() {
        // Notes tree with 2/38 fanout.
        let target = oid("aabbccdd00112233445566778899aabbccddeeff");
        let note_blob = oid("1234567890abcdef1234567890abcdef12345678");

        // Sub-tree "aa" contains blob "bbccdd00112233445566778899aabbccddeeff"
        let sub_tree_oid = oid("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
        let sub_tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: blob_mode(),
                filename: "bbccdd00112233445566778899aabbccddeeff".into(),
                oid: note_blob,
            }],
        };

        // Root tree has directory "aa"
        let root_tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: tree_mode(),
                filename: "aa".into(),
                oid: sub_tree_oid,
            }],
        };

        let mut find_tree = move |id: &ObjectId| -> Result<gix_object::Tree, std::io::Error> {
            if *id == sub_tree_oid {
                Ok(sub_tree.clone())
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unknown tree"))
            }
        };

        let entries = list_notes(&root_tree, gix_hash::Kind::Sha1, "", &mut find_tree).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].target, target);
        assert_eq!(entries[0].note_id, note_blob);
    }

    #[test]
    fn list_notes_empty_tree() {
        let tree = gix_object::Tree::empty();
        let mut find_tree = |_: &ObjectId| -> Result<gix_object::Tree, std::io::Error> { unreachable!() };
        let entries = list_notes(&tree, gix_hash::Kind::Sha1, "", &mut find_tree).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn list_notes_skips_wrong_length_filenames() {
        // An entry whose filename is not a valid hex hash length should be skipped.
        let tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: blob_mode(),
                filename: "tooshort".into(),
                oid: oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            }],
        };

        let mut find_tree = |_: &ObjectId| -> Result<gix_object::Tree, std::io::Error> { unreachable!() };

        let entries = list_notes(&tree, gix_hash::Kind::Sha1, "", &mut find_tree).unwrap();
        assert!(entries.is_empty());
    }

    // --- find_note tests ---

    #[test]
    fn find_note_flat() {
        let target = oid("aabbccdd00112233445566778899aabbccddeeff");
        let note_blob = oid("1234567890abcdef1234567890abcdef12345678");

        let tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: blob_mode(),
                filename: target.to_hex().to_string().into(),
                oid: note_blob,
            }],
        };

        let mut find_tree = |_: &ObjectId| -> Result<gix_object::Tree, std::io::Error> { unreachable!() };

        let result = find_note(&tree, &target, gix_hash::Kind::Sha1, &mut find_tree).unwrap();
        assert_eq!(result, Some(note_blob));
    }

    #[test]
    fn find_note_with_fanout() {
        let target = oid("aabbccdd00112233445566778899aabbccddeeff");
        let note_blob = oid("1234567890abcdef1234567890abcdef12345678");
        let sub_tree_oid = oid("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");

        let sub_tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: blob_mode(),
                filename: "bbccdd00112233445566778899aabbccddeeff".into(),
                oid: note_blob,
            }],
        };

        let root_tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: tree_mode(),
                filename: "aa".into(),
                oid: sub_tree_oid,
            }],
        };

        let mut find_tree = move |id: &ObjectId| -> Result<gix_object::Tree, std::io::Error> {
            if *id == sub_tree_oid {
                Ok(sub_tree.clone())
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unknown tree"))
            }
        };

        let result = find_note(&root_tree, &target, gix_hash::Kind::Sha1, &mut find_tree).unwrap();
        assert_eq!(result, Some(note_blob));
    }

    #[test]
    fn find_note_missing() {
        let target = oid("aabbccdd00112233445566778899aabbccddeeff");
        let other = oid("1111111111111111111111111111111111111111");

        let tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: blob_mode(),
                filename: other.to_hex().to_string().into(),
                oid: oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            }],
        };

        let mut find_tree = |_: &ObjectId| -> Result<gix_object::Tree, std::io::Error> { unreachable!() };

        let result = find_note(&tree, &target, gix_hash::Kind::Sha1, &mut find_tree).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn find_note_empty_tree() {
        let target = oid("aabbccdd00112233445566778899aabbccddeeff");
        let tree = gix_object::Tree::empty();

        let mut find_tree = |_: &ObjectId| -> Result<gix_object::Tree, std::io::Error> { unreachable!() };

        let result = find_note(&tree, &target, gix_hash::Kind::Sha1, &mut find_tree).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn find_note_double_fanout() {
        // 2/2/36 fanout
        let target = oid("aabbccdd00112233445566778899aabbccddeeff");
        let note_blob = oid("1234567890abcdef1234567890abcdef12345678");
        let inner_tree_oid = oid("dddddddddddddddddddddddddddddddddddddddd");
        let outer_tree_oid = oid("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");

        let inner_tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: blob_mode(),
                filename: "ccdd00112233445566778899aabbccddeeff".into(),
                oid: note_blob,
            }],
        };

        let outer_tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: tree_mode(),
                filename: "bb".into(),
                oid: inner_tree_oid,
            }],
        };

        let root_tree = gix_object::Tree {
            entries: vec![TreeEntry {
                mode: tree_mode(),
                filename: "aa".into(),
                oid: outer_tree_oid,
            }],
        };

        let mut find_tree = move |id: &ObjectId| -> Result<gix_object::Tree, std::io::Error> {
            if *id == outer_tree_oid {
                Ok(outer_tree.clone())
            } else if *id == inner_tree_oid {
                Ok(inner_tree.clone())
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unknown tree"))
            }
        };

        let result = find_note(&root_tree, &target, gix_hash::Kind::Sha1, &mut find_tree).unwrap();
        assert_eq!(result, Some(note_blob));
    }
}
