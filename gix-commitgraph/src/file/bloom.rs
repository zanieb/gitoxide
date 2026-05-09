/// Bloom filter parameters shared by all filters in a commit-graph file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    /// Version of the hash algorithm used by the filter.
    pub hash_version: u32,
    /// Number of hash probes used per changed path.
    pub num_hashes: u32,
    /// Minimum number of bits per changed-path entry.
    pub bits_per_entry: u32,
}

/// A changed-path Bloom filter for one commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Filter<'a> {
    /// Settings shared by all filters in the graph file.
    pub settings: Settings,
    data: &'a [u8],
}

impl<'a> Filter<'a> {
    pub(crate) fn new(settings: Settings, data: &'a [u8]) -> Self {
        Filter { settings, data }
    }

    /// Return the raw filter bytes as stored in the commit-graph file.
    pub fn bytes(&self) -> &'a [u8] {
        self.data
    }
}
