#![no_main]

use libfuzzer_sys::fuzz_target;
use std::hint::black_box;
use std::io::Write;

fuzz_target!(|data: &[u8]| {
    // Fuzz pack data file parsing by writing the fuzzed data to a temporary file
    // and attempting to open it as a pack file.
    //
    // This exercises:
    // - Pack header validation (magic "PACK", version, num_objects)
    // - Memory-mapped file handling with potentially corrupt data
    // - Size validation logic
    //
    // The pack data file format:
    // - 4 bytes: "PACK" magic
    // - 4 bytes: version (2 or 3) as u32 BE
    // - 4 bytes: number of objects as u32 BE
    // - N bytes: packed objects (entries with headers + compressed data)
    // - 20/32 bytes: trailing checksum hash

    if data.len() < 12 {
        return;
    }

    // Write to a temp file and try to open as pack
    let dir = tempfile::tempdir().expect("tempdir");
    let pack_path = dir.path().join("test.pack");
    {
        let mut f = std::fs::File::create(&pack_path).expect("create");
        f.write_all(data).expect("write");
    }

    // Try opening with SHA-1
    if let Ok(file) = black_box(gix_pack::data::File::at(&pack_path, gix_hash::Kind::Sha1)) {
        _ = black_box(file.version());
        _ = black_box(file.num_objects());
        _ = black_box(file.data_len());
        _ = black_box(file.pack_end());
        _ = black_box(file.object_hash());

        // Try to decode entries at various offsets
        let entry_result = gix_pack::data::Entry::from_bytes(
            &data[12..],
            12,
            gix_hash::Kind::Sha1.len_in_bytes(),
        );
        if let Ok(entry) = black_box(entry_result) {
            _ = black_box(entry.header_size());
            _ = black_box(entry.pack_offset());
        }
    }

    // Try opening with the longest supported hash kind
    _ = black_box(gix_pack::data::File::at(&pack_path, gix_hash::Kind::longest()));
});
