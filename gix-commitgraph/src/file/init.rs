use std::path::{Path, PathBuf};

use gix_error::{ErrorExt, Exn, Message, ResultExt, message};

use crate::{
    File,
    file::{
        bloom, BASE_GRAPHS_LIST_CHUNK_ID, BLOOM_FILTER_DATA_CHUNK_ID, BLOOM_FILTER_INDEX_CHUNK_ID,
        COMMIT_DATA_CHUNK_ID, COMMIT_DATA_ENTRY_SIZE_SANS_HASH, EXTENDED_EDGES_LIST_CHUNK_ID, FAN_LEN,
        GENERATION_DATA_CHUNK_ID, GENERATION_DATA_OVERFLOW_CHUNK_ID, GENERATION_DATA_OVERFLOW_MASK, HEADER_LEN,
        OID_FAN_CHUNK_ID, OID_LOOKUP_CHUNK_ID, SIGNATURE,
    },
};

const MIN_FILE_SIZE: usize = HEADER_LEN
    + gix_chunk::file::Index::size_for_entries(3 /*OIDF, OIDL, CDAT*/)
    + FAN_LEN * 4 /* FANOUT TABLE CHUNK OIDF */
    + gix_hash::Kind::shortest().len_in_bytes();

impl File {
    /// Try to parse the commit graph file at `path`.
    pub fn at(path: impl AsRef<Path>) -> Result<File, Exn<Message>> {
        Self::try_from(path.as_ref())
    }

    /// A lower-level constructor which constructs a new instance directly from the mapping in `data`,
    /// assuming that it originated from `path`.
    ///
    /// Note that `path` is only used for verification of the hash its basename contains, but otherwise
    /// is not of importance.
    pub fn new(data: memmap2::Mmap, path: PathBuf) -> Result<File, Exn<Message>> {
        let data_size = data.len();
        if data_size < MIN_FILE_SIZE {
            return Err(message("Commit-graph file too small even for an empty graph").raise());
        }

        let mut ofs = 0;
        if &data[ofs..ofs + SIGNATURE.len()] != SIGNATURE {
            return Err(message("Commit-graph file does not start with expected signature").raise());
        }
        ofs += SIGNATURE.len();

        match data[ofs] {
            1 => (),
            x => {
                return Err(message!("Unsupported commit-graph file version: {x}").raise());
            }
        }
        ofs += 1;

        let object_hash = gix_hash::Kind::try_from(data[ofs])
            .map_err(|v| message!("Commit-graph file uses unsupported hash version: {v}").raise())?;
        ofs += 1;

        let chunk_count = data[ofs];
        // Can assert chunk_count >= MIN_CHUNKS here, but later OIDF+OIDL+CDAT presence checks make
        // it redundant.
        ofs += 1;

        let base_graph_count = data[ofs];
        ofs += 1;

        let chunks = gix_chunk::file::Index::from_bytes(&data, ofs, u32::from(chunk_count))
            .or_raise(|| message!("Couldn't read commit-graph file with {chunk_count} chunks at offset {ofs}"))?;

        let base_graphs_list_offset = chunks
            .validated_usize_offset_by_id(BASE_GRAPHS_LIST_CHUNK_ID, |chunk_range| {
                let chunk_size = chunk_range.len();
                if chunk_size % object_hash.len_in_bytes() != 0 {
                    return Err(message!("Commit-graph chunk {BASE_GRAPHS_LIST_CHUNK_ID:?} has invalid size: {msg}",
                        msg = format!(
                            "chunk size {} is not a multiple of {}",
                            chunk_size,
                            object_hash.len_in_bytes()
                        ),
                    ).raise());
                }
                let chunk_base_graph_count: u32 = (chunk_size / object_hash.len_in_bytes())
                    .try_into()
                    .expect("base graph count to fit in 32-bits");
                if chunk_base_graph_count != u32::from(base_graph_count) {
                    return Err(message!("Commit-graph {BASE_GRAPHS_LIST_CHUNK_ID:?} chunk contains {chunk_base_graph_count} base graphs, but commit-graph file header claims {base_graph_count} base graphs").raise())
                }
                Ok(chunk_range.start)
            })
            .ok()
            .transpose()?;

        let (commit_data_offset, commit_data_count): (_, u32) = chunks
            .validated_usize_offset_by_id(COMMIT_DATA_CHUNK_ID, |chunk_range| {
                let chunk_size = chunk_range.len();

                let entry_size = object_hash.len_in_bytes() + COMMIT_DATA_ENTRY_SIZE_SANS_HASH;
                if chunk_size % entry_size != 0 {
                    return Err(message!("Commit-graph chunk {COMMIT_DATA_CHUNK_ID:?} has invalid size: chunk size {chunk_size} is not a multiple of {entry_size}").raise())
                }
                Ok((
                    chunk_range.start,
                    (chunk_size / entry_size)
                        .try_into()
                        .expect("number of commits in CDAT chunk to fit in 32 bits"),
                ))
            })??;

        let fan_offset = chunks
            .validated_usize_offset_by_id(OID_FAN_CHUNK_ID, |chunk_range| {
                let chunk_size = chunk_range.len();

                let expected_size = 4 * FAN_LEN;
                if chunk_size != expected_size {
                    return Err(message!("Commit-graph chunk {OID_FAN_CHUNK_ID:?} has invalid size: expected chunk length {expected_size}, got {chunk_size}").raise())
                }
                Ok(chunk_range.start)
            })?
            .or_raise(|| message("Error getting offset for OID fan chunk"))?;

        let (oid_lookup_offset, oid_lookup_count): (_, u32) = chunks
            .validated_usize_offset_by_id(OID_LOOKUP_CHUNK_ID, |chunk_range| {
                let chunk_size = chunk_range.len();

                if chunk_size % object_hash.len_in_bytes() != 0 {
                    return Err(message!("Commit-graph chunk {OID_LOOKUP_CHUNK_ID:?} has invalid size: chunk size {chunk_size} is not a multiple of {hash_len}", hash_len = object_hash.len_in_bytes()).raise())
                }
                Ok((
                    chunk_range.start,
                    (chunk_size / object_hash.len_in_bytes())
                        .try_into()
                        .expect("number of commits in OIDL chunk to fit in 32 bits"),
                ))
            })?
            .or_raise(|| message("Error getting offset for OID lookup chunk"))?;

        let extra_edges_list_range = chunks.usize_offset_by_id(EXTENDED_EDGES_LIST_CHUNK_ID).ok();
        let bloom_filter_index_range = chunks
            .validated_usize_offset_by_id(BLOOM_FILTER_INDEX_CHUNK_ID, |chunk_range| {
                let expected_size = usize::try_from(commit_data_count).expect("commit count fits usize") * 4;
                if chunk_range.len() != expected_size {
                    return Err(message!(
                        "Commit-graph chunk {BLOOM_FILTER_INDEX_CHUNK_ID:?} has invalid size: expected chunk length {expected_size}, got {}",
                        chunk_range.len()
                    )
                    .raise());
                }
                Ok(chunk_range)
            })
            .ok()
            .transpose()?;
        let bloom_filter_data_range = chunks
            .validated_usize_offset_by_id(BLOOM_FILTER_DATA_CHUNK_ID, |chunk_range| {
                if chunk_range.len() < 12 {
                    return Err(message!(
                        "Commit-graph chunk {BLOOM_FILTER_DATA_CHUNK_ID:?} has invalid size: expected at least 12 bytes, got {}",
                        chunk_range.len()
                    )
                    .raise());
                }
                Ok(chunk_range)
            })
            .ok()
            .transpose()?;
        let bloom_filter_settings = match (&bloom_filter_index_range, &bloom_filter_data_range) {
            (Some(index_range), Some(data_range)) => {
                let bloom_data = &data[data_range.clone()];
                let settings = bloom::Settings {
                    hash_version: read_u32(&bloom_data[..4]),
                    num_hashes: read_u32(&bloom_data[4..8]),
                    bits_per_entry: read_u32(&bloom_data[8..12]),
                };
                if settings.hash_version != 1 {
                    return Err(message!(
                        "Commit-graph chunk {BLOOM_FILTER_DATA_CHUNK_ID:?} uses unsupported Bloom filter hash version {}",
                        settings.hash_version
                    )
                    .raise());
                }

                let filter_data_len = bloom_data.len() - 12;
                let mut previous_end = 0usize;
                for raw_end in data[index_range.clone()].chunks_exact(4).map(read_u32) {
                    let end = usize::try_from(raw_end).expect("u32 fits usize");
                    if end < previous_end {
                        return Err(message!(
                            "Commit-graph chunk {BLOOM_FILTER_INDEX_CHUNK_ID:?} contains a decreasing Bloom filter offset"
                        )
                        .raise());
                    }
                    if end > filter_data_len {
                        return Err(message!(
                            "Commit-graph chunk {BLOOM_FILTER_INDEX_CHUNK_ID:?} references Bloom filter byte {end}, but {BLOOM_FILTER_DATA_CHUNK_ID:?} has {filter_data_len} filter bytes"
                        )
                        .raise());
                    }
                    previous_end = end;
                }
                Some(settings)
            }
            (Some(_), None) => None,
            (None, Some(_)) => {
                return Err(message!(
                    "Chunk named {BLOOM_FILTER_DATA_CHUNK_ID:?} requires {BLOOM_FILTER_INDEX_CHUNK_ID:?}"
                )
                .raise());
            }
            (None, None) => None,
        };
        let bloom_filter_index_range = bloom_filter_settings.and(bloom_filter_index_range);
        let bloom_filter_data_range = bloom_filter_settings.and(bloom_filter_data_range);
        let generation_data_range = chunks
            .validated_usize_offset_by_id(GENERATION_DATA_CHUNK_ID, |chunk_range| {
                let expected_size = usize::try_from(commit_data_count).expect("commit count fits usize") * 4;
                if chunk_range.len() != expected_size {
                    return Err(message!(
                        "Commit-graph chunk {GENERATION_DATA_CHUNK_ID:?} has invalid size: expected chunk length {expected_size}, got {}",
                        chunk_range.len()
                    )
                    .raise());
                }
                Ok(chunk_range)
            })
            .ok()
            .transpose()?;
        let generation_data_overflow_range = chunks
            .validated_usize_offset_by_id(GENERATION_DATA_OVERFLOW_CHUNK_ID, |chunk_range| {
                if chunk_range.len() % 8 != 0 {
                    return Err(message!(
                        "Commit-graph chunk {GENERATION_DATA_OVERFLOW_CHUNK_ID:?} has invalid size: chunk size {} is not a multiple of 8",
                        chunk_range.len()
                    )
                    .raise());
                }
                Ok(chunk_range)
            })
            .ok()
            .transpose()?;
        if generation_data_overflow_range.is_some() && generation_data_range.is_none() {
            return Err(message!(
                "Chunk named {GENERATION_DATA_OVERFLOW_CHUNK_ID:?} requires {GENERATION_DATA_CHUNK_ID:?}"
            )
            .raise());
        }
        if let Some(range) = &generation_data_range {
            let overflow_entries = generation_data_overflow_range
                .as_ref()
                .map(|range| range.len() / 8)
                .unwrap_or_default();
            for raw_offset in data[range.clone()].chunks_exact(4).map(read_u32) {
                if raw_offset & GENERATION_DATA_OVERFLOW_MASK != 0 {
                    let overflow_index = (raw_offset & !GENERATION_DATA_OVERFLOW_MASK) as usize;
                    if overflow_index >= overflow_entries {
                        return Err(message!(
                            "Commit-graph chunk {GENERATION_DATA_CHUNK_ID:?} references corrected-date overflow index {overflow_index}, but {GENERATION_DATA_OVERFLOW_CHUNK_ID:?} has {overflow_entries} entries"
                        )
                        .raise());
                    }
                }
            }
        }

        let trailer = &data[chunks.highest_offset() as usize..];
        if trailer.len() != object_hash.len_in_bytes() {
            return Err(message!(
                "Expected commit-graph trailer to contain {} bytes, got {}",
                object_hash.len_in_bytes(),
                trailer.len()
            )
            .raise());
        }

        if base_graph_count > 0 && base_graphs_list_offset.is_none() {
            return Err(message!("Chunk named {BASE_GRAPHS_LIST_CHUNK_ID:?} was not found in chunk file index").into());
        }

        let (fan, _) = read_fan(&data[fan_offset..]);
        if oid_lookup_count != fan[255] {
            return Err(message!("Commit-graph {OID_FAN_CHUNK_ID:?} chunk contains {chunk1_commits} commits, but {OID_LOOKUP_CHUNK_ID:?} chunk contains {chunk2_commits} commits",
                chunk1_commits = fan[255],
                chunk2_commits = oid_lookup_count,
            ).raise());
        }
        if commit_data_count != fan[255] {
            return Err(
                message!("Commit-graph {OID_FAN_CHUNK_ID:?} chunk contains {chunk1_commits} commits, but {COMMIT_DATA_CHUNK_ID:?} chunk contains {chunk2_commits} commits",
                    chunk1_commits = fan[255],
                    chunk2_commits = commit_data_count,
                ).raise(),
            );
        }
        Ok(File {
            base_graph_count,
            base_graphs_list_offset,
            bloom_filter_data_range,
            bloom_filter_index_range,
            bloom_filter_settings,
            commit_data_offset,
            data,
            extra_edges_list_range,
            fan,
            generation_data_range,
            generation_data_overflow_range,
            oid_lookup_offset,
            path,
            hash_len: object_hash.len_in_bytes(),
            object_hash,
        })
    }
}

impl TryFrom<&Path> for File {
    type Error = Exn<Message>;

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        let data = std::fs::File::open(path)
            .and_then(|file| {
                // SAFETY: we have to take the risk of somebody changing the file underneath. Git never writes into the same file.
                #[allow(unsafe_code)]
                unsafe {
                    memmap2::MmapOptions::new().map_copy_read_only(&file)
                }
            })
            .or_raise(|| message!("Could not open commit-graph file at '{path}'", path = path.display()))?;
        Self::new(data, path.to_owned())
    }
}

// Copied from gix-odb/pack/index/init.rs
fn read_u32(d: &[u8]) -> u32 {
    u32::from_be_bytes(d.try_into().unwrap())
}

fn read_fan(d: &[u8]) -> ([u32; FAN_LEN], usize) {
    assert!(d.len() >= FAN_LEN * 4);

    let mut fan = [0; FAN_LEN];
    for (c, f) in d.chunks_exact(4).zip(fan.iter_mut()) {
        *f = u32::from_be_bytes(c.try_into().unwrap());
    }
    (fan, FAN_LEN * 4)
}
