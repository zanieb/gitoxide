use bstr::BString;
use gix_hash::ObjectId;

use crate::{
    entry,
    extension::{Signature, UntrackedCache},
    util::{read_u32, split_at_byte_exclusive, var_int, write_var_int},
};

/// A structure to track filesystem stat information along with an object id, linking a worktree file with what's in our ODB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidStat {
    /// The file system stat information
    pub stat: entry::Stat,
    /// The id of the file in our ODB.
    pub id: ObjectId,
}

/// A directory with information about its untracked files, and its sub-directories
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    /// The directories name, or an empty string if this is the root directory.
    pub name: BString,
    /// Untracked files and directory names
    pub untracked_entries: Vec<BString>,
    /// indices for sub-directories similar to this one.
    pub sub_directories: Vec<usize>,

    /// The directories stat data, if available or valid // TODO: or is it the exclude file?
    pub stat: Option<entry::Stat>,
    /// The oid of a .gitignore file, if it exists
    pub exclude_file_oid: Option<ObjectId>,
    /// TODO: figure out what this really does
    pub check_only: bool,
}

/// Only used as an indicator
pub const SIGNATURE: Signature = *b"UNTR";

// #[allow(unused)]
/// Decode an untracked cache extension from `data`, assuming object hashes are of type `object_hash`.
pub fn decode(data: &[u8], object_hash: gix_hash::Kind, alloc_limit_bytes: Option<usize>) -> Option<UntrackedCache> {
    if data.last().is_none_or(|b| *b != 0) {
        return None;
    }
    let (identifier_len, data) = var_int(data)?;
    let (identifier, data) = data.split_at_checked(identifier_len.try_into().ok()?)?;

    let hash_len = object_hash.len_in_bytes();
    let (info_exclude, data) = decode_oid_stat(data, hash_len)?;
    let (excludes_file, data) = decode_oid_stat(data, hash_len)?;
    let (dir_flags, data) = read_u32(data)?;
    let (exclude_filename_per_dir, data) = split_at_byte_exclusive(data, 0)?;

    let (num_directory_blocks, data) = var_int(data)?;

    let mut res = UntrackedCache {
        identifier: identifier.into(),
        info_exclude: (!info_exclude.id.is_null()).then_some(info_exclude),
        excludes_file: (!excludes_file.id.is_null()).then_some(excludes_file),
        exclude_filename_per_dir: exclude_filename_per_dir.into(),
        dir_flags,
        directories: Vec::new(),
    };
    if num_directory_blocks == 0 {
        return data.is_empty().then_some(res);
    }

    let num_directory_blocks: usize = num_directory_blocks.try_into().ok()?;
    if num_directory_blocks > data.len() {
        return None;
    }
    if alloc_limit_bytes
        .is_some_and(|limit| num_directory_blocks.saturating_mul(std::mem::size_of::<Directory>()) > limit)
    {
        return None;
    }
    let directories = &mut res.directories;
    directories.try_reserve(num_directory_blocks).ok()?;

    let data = decode_directory_block(data, directories, alloc_limit_bytes)?;
    if directories.len() != num_directory_blocks {
        return None;
    }
    let (valid, data) = gix_bitmap::ewah::decode(data).ok()?;
    let (check_only, data) = gix_bitmap::ewah::decode(data).ok()?;
    let (hash_valid, mut data) = gix_bitmap::ewah::decode(data).ok()?;

    if valid.num_bits() > num_directory_blocks
        || check_only.num_bits() > num_directory_blocks
        || hash_valid.num_bits() > num_directory_blocks
    {
        return None;
    }

    check_only.for_each_set_bit(|index| {
        let directory = directories.get_mut(index)?;
        directory.check_only = true;
        Some(())
    })?;
    valid.for_each_set_bit(|index| {
        let directory = directories.get_mut(index)?;
        let (stat, rest) = crate::decode::stat(data)?;
        directory.stat = stat.into();
        data = rest;
        Some(())
    })?;
    hash_valid.for_each_set_bit(|index| {
        let directory = directories.get_mut(index)?;
        let (hash, rest) = data.split_at_checked(hash_len)?;
        data = rest;
        directory.exclude_file_oid = ObjectId::from_bytes_or_panic(hash).into();
        Some(())
    })?;

    // null-byte checked in the beginning
    if data.len() != 1 {
        return None;
    }
    res.into()
}

fn decode_directory_block<'a>(
    data: &'a [u8],
    directories: &mut Vec<Directory>,
    alloc_limit_bytes: Option<usize>,
) -> Option<&'a [u8]> {
    let (num_untracked, data) = var_int(data)?;
    let (num_dirs, data) = var_int(data)?;
    let (name, mut data) = split_at_byte_exclusive(data, 0)?;
    // Untracked names are encoded as `name\0name\0...`, and we assume names are non-empty:
    // `a\0b\0` is 4 bytes for 2 entries, so each entry needs at least 2 bytes.
    let max_entries_from_remaining_data = data.len() / 2;
    let num_untracked: usize = num_untracked.try_into().ok()?;
    let num_dirs: usize = num_dirs.try_into().ok()?;
    if num_untracked > max_entries_from_remaining_data || num_dirs > max_entries_from_remaining_data {
        return None;
    }
    if alloc_limit_bytes.is_some_and(|limit| {
        num_untracked.saturating_mul(std::mem::size_of::<BString>()) > limit
            || num_dirs.saturating_mul(std::mem::size_of::<usize>()) > limit
    }) {
        return None;
    }
    let mut untracked_entries = Vec::<BString>::new();
    untracked_entries.try_reserve(num_untracked).ok()?;
    for _ in 0..num_untracked {
        let (name, rest) = split_at_byte_exclusive(data, 0)?;
        data = rest;
        untracked_entries.push(name.into());
    }

    let index = directories.len();
    directories.push(Directory {
        name: name.into(),
        untracked_entries,
        sub_directories: {
            let mut sub_directories = Vec::new();
            sub_directories.try_reserve(num_dirs).ok()?;
            sub_directories
        },
        // the following are set later through their bitmaps
        stat: None,
        exclude_file_oid: None,
        check_only: false,
    });

    for _ in 0..num_dirs {
        let subdir_index = directories.len();
        let rest = decode_directory_block(data, directories, alloc_limit_bytes)?;
        data = rest;
        directories[index].sub_directories.push(subdir_index);
    }

    data.into()
}

fn decode_oid_stat(data: &[u8], hash_len: usize) -> Option<(OidStat, &[u8])> {
    let (stat, data) = crate::decode::stat(data)?;
    let (hash, data) = data.split_at_checked(hash_len)?;
    Some((
        OidStat {
            stat,
            id: ObjectId::from_bytes_or_panic(hash),
        },
        data,
    ))
}

/// Serialize an untracked-cache extension to `out`.
pub fn write_to(
    untracked: &UntrackedCache,
    object_hash: gix_hash::Kind,
    mut out: impl std::io::Write,
) -> Result<(), std::io::Error> {
    use std::io::Write as _;

    let mut data = Vec::new();
    write_var_int(
        untracked
            .identifier
            .len()
            .try_into()
            .expect("identifier length fits u64"),
        &mut data,
    )?;
    data.write_all(&untracked.identifier)?;
    write_oid_stat(untracked.info_exclude.as_ref(), object_hash, &mut data)?;
    write_oid_stat(untracked.excludes_file.as_ref(), object_hash, &mut data)?;
    data.write_all(&untracked.dir_flags.to_be_bytes())?;
    data.write_all(&untracked.exclude_filename_per_dir)?;
    data.write_all(b"\0")?;
    write_var_int(
        untracked
            .directories
            .len()
            .try_into()
            .expect("directory count fits u64"),
        &mut data,
    )?;

    if let Some(_root) = untracked.directories.first() {
        validate_directory_graph(&untracked.directories)?;
        write_directory_block(&untracked.directories, 0, &mut data)?;

        let num_directories = untracked.directories.len();
        write_bitmap(
            num_directories,
            untracked
                .directories
                .iter()
                .enumerate()
                .filter_map(|(index, dir)| dir.stat.is_some().then_some(index)),
            &mut data,
        )?;
        write_bitmap(
            num_directories,
            untracked
                .directories
                .iter()
                .enumerate()
                .filter_map(|(index, dir)| dir.check_only.then_some(index)),
            &mut data,
        )?;
        write_bitmap(
            num_directories,
            untracked
                .directories
                .iter()
                .enumerate()
                .filter_map(|(index, dir)| dir.exclude_file_oid.is_some().then_some(index)),
            &mut data,
        )?;

        for stat in untracked.directories.iter().filter_map(|dir| dir.stat.as_ref()) {
            write_stat(stat, &mut data)?;
        }
        for oid in untracked
            .directories
            .iter()
            .filter_map(|dir| dir.exclude_file_oid.as_ref())
        {
            write_object_id(oid, object_hash, &mut data)?;
        }
        data.write_all(b"\0")?;
    }

    out.write_all(&SIGNATURE)?;
    out.write_all(&(u32::try_from(data.len()).expect("less than 4GB untracked-cache extension")).to_be_bytes())?;
    out.write_all(&data)
}

fn validate_directory_graph(directories: &[Directory]) -> Result<(), std::io::Error> {
    fn invalid(message: &'static str) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
    }

    fn visit(
        index: usize,
        directories: &[Directory],
        active: &mut [bool],
        visited: &mut [bool],
    ) -> Result<(), std::io::Error> {
        let Some(directory) = directories.get(index) else {
            return Err(invalid("untracked-cache directory index is out of bounds"));
        };
        if active[index] {
            return Err(invalid("untracked-cache directory graph contains a cycle"));
        }
        if visited[index] {
            return Err(invalid(
                "untracked-cache directory graph references a directory more than once",
            ));
        }

        active[index] = true;
        for &subdir_index in &directory.sub_directories {
            visit(subdir_index, directories, active, visited)?;
        }
        active[index] = false;
        visited[index] = true;
        Ok(())
    }

    let mut active = vec![false; directories.len()];
    let mut visited = vec![false; directories.len()];
    visit(0, directories, &mut active, &mut visited)?;
    if visited.iter().any(|seen| !seen) {
        return Err(invalid(
            "untracked-cache directory graph contains unreachable directories",
        ));
    }
    Ok(())
}

fn write_oid_stat(
    stat: Option<&OidStat>,
    object_hash: gix_hash::Kind,
    out: &mut dyn std::io::Write,
) -> Result<(), std::io::Error> {
    match stat {
        Some(stat) => {
            write_stat(&stat.stat, out)?;
            write_object_id(&stat.id, object_hash, out)
        }
        None => {
            write_stat(&entry::Stat::default(), out)?;
            write_object_id(&ObjectId::null(object_hash), object_hash, out)
        }
    }
}

fn write_object_id(
    id: &ObjectId,
    object_hash: gix_hash::Kind,
    out: &mut dyn std::io::Write,
) -> Result<(), std::io::Error> {
    let bytes = id.as_bytes();
    let expected = object_hash.len_in_bytes();
    if bytes.len() != expected {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "object id length does not match index object hash",
        ));
    }
    out.write_all(bytes)
}

fn write_stat(stat: &entry::Stat, out: &mut dyn std::io::Write) -> Result<(), std::io::Error> {
    out.write_all(&stat.mtime.secs.to_be_bytes())?;
    out.write_all(&stat.mtime.nsecs.to_be_bytes())?;
    out.write_all(&stat.ctime.secs.to_be_bytes())?;
    out.write_all(&stat.ctime.nsecs.to_be_bytes())?;
    out.write_all(&stat.dev.to_be_bytes())?;
    out.write_all(&stat.ino.to_be_bytes())?;
    out.write_all(&stat.uid.to_be_bytes())?;
    out.write_all(&stat.gid.to_be_bytes())?;
    out.write_all(&stat.size.to_be_bytes())
}

fn write_directory_block(
    directories: &[Directory],
    index: usize,
    out: &mut dyn std::io::Write,
) -> Result<(), std::io::Error> {
    let directory = &directories[index];
    write_var_int(
        directory
            .untracked_entries
            .len()
            .try_into()
            .expect("untracked entry count fits u64"),
        &mut *out,
    )?;
    write_var_int(
        directory
            .sub_directories
            .len()
            .try_into()
            .expect("subdirectory count fits u64"),
        &mut *out,
    )?;
    out.write_all(&directory.name)?;
    out.write_all(b"\0")?;
    for entry in &directory.untracked_entries {
        out.write_all(entry)?;
        out.write_all(b"\0")?;
    }
    for subdir_index in &directory.sub_directories {
        write_directory_block(directories, *subdir_index, out)?;
    }
    Ok(())
}

fn write_bitmap(
    num_bits: usize,
    set_indices: impl IntoIterator<Item = usize>,
    out: &mut dyn std::io::Write,
) -> Result<(), std::io::Error> {
    let num_words = num_bits.div_ceil(64);
    let mut words = vec![0_u64; num_words];
    for index in set_indices {
        words[index / 64] |= 1_u64 << (index % 64);
    }

    out.write_all(&u32::try_from(num_bits).expect("less than 2^32 bits").to_be_bytes())?;
    out.write_all(
        &u32::try_from(num_words + 1)
            .expect("less than 2^32 EWAH words")
            .to_be_bytes(),
    )?;
    out.write_all(&((u64::try_from(num_words).expect("word count fits u64")) << 33).to_be_bytes())?;
    for word in words {
        out.write_all(&word.to_be_bytes())?;
    }
    out.write_all(&0_u32.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::{decode, write_bitmap, write_oid_stat, write_to, Directory, OidStat};
    use crate::entry;
    use crate::extension::UntrackedCache;
    use crate::util::write_var_int;
    use bstr::BString;
    use gix_hash::ObjectId;

    #[test]
    fn var_int_roundtrips_through_the_decoder() {
        for value in [0, 1, 42, 127, 128, 255, 16_383, 16_384, 1_000_000] {
            let mut out = Vec::new();
            write_var_int(value, &mut out).unwrap();
            let (actual, rest) = crate::util::var_int(&out).unwrap();
            assert_eq!(actual, value);
            assert!(rest.is_empty());
        }
    }

    #[test]
    fn bitmap_writer_roundtrips_as_ewah() {
        let mut out = Vec::new();
        write_bitmap(130, [0, 64, 129], &mut out).unwrap();
        let (bitmap, rest) = gix_bitmap::ewah::decode(&out).unwrap();
        assert!(rest.is_empty());

        let mut actual = Vec::new();
        bitmap
            .for_each_set_bit(|index| {
                actual.push(index);
                Some(())
            })
            .unwrap();
        assert_eq!(actual, [0, 64, 129]);
    }

    #[test]
    fn decode_rejects_truncated_stat_data_selected_by_bitmap() {
        let object_hash = gix_hash::Kind::Sha1;
        let mut data = Vec::new();

        write_var_int(0, &mut data).unwrap();
        write_oid_stat(None, object_hash, &mut data).unwrap();
        write_oid_stat(None, object_hash, &mut data).unwrap();
        data.extend_from_slice(&0_u32.to_be_bytes());
        data.push(0);
        write_var_int(1, &mut data).unwrap();

        write_var_int(0, &mut data).unwrap();
        write_var_int(0, &mut data).unwrap();
        data.push(0);

        write_bitmap(1, [0], &mut data).unwrap();
        write_bitmap(1, [], &mut data).unwrap();
        write_bitmap(1, [], &mut data).unwrap();
        data.push(0);

        assert!(decode(&data, object_hash).is_none());
    }

    #[test]
    fn write_oid_stat_rejects_object_hash_mismatch() {
        let stat = OidStat {
            stat: entry::Stat::default(),
            id: ObjectId::from_bytes_or_panic(&[0; 20]),
        };
        let mut out = Vec::new();

        let err = write_oid_stat(Some(&stat), gix_hash::Kind::Sha256, &mut out).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn write_to_rejects_malformed_directory_graphs() {
        for directories in [
            vec![directory([1])],
            vec![directory([0])],
            vec![directory([]), directory([])],
            vec![directory([1, 1]), directory([])],
        ] {
            let cache = cache(directories);
            let mut out = Vec::new();

            let err = write_to(&cache, gix_hash::Kind::Sha1, &mut out).unwrap_err();

            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    fn cache(directories: Vec<Directory>) -> UntrackedCache {
        UntrackedCache {
            identifier: BString::from("test"),
            info_exclude: None,
            excludes_file: None,
            exclude_filename_per_dir: BString::from(".gitignore"),
            dir_flags: 0,
            directories,
        }
    }

    fn directory<const N: usize>(sub_directories: [usize; N]) -> Directory {
        Directory {
            name: BString::new(Vec::new()),
            untracked_entries: Vec::new(),
            sub_directories: sub_directories.into(),
            stat: None,
            exclude_file_oid: None,
            check_only: false,
        }
    }
}
