#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use std::hint::black_box;

#[derive(Debug, Arbitrary)]
struct Ctx<'a> {
    data: &'a [u8],
    pack_offset: u64,
}

fuzz_target!(|ctx: Ctx| {
    // The entry decoder reads a variable-length header from the byte stream.
    // It encodes object type (3 bits) and decompressed size (variable-length encoding).
    // For OFS_DELTA, it also reads a LEB128-encoded base distance.
    // For REF_DELTA, it reads a hash_len-sized object ID.
    //
    // This is the core pack parser that processes untrusted remote data.
    if ctx.data.is_empty() {
        return;
    }

    // Test with SHA-1 hash length (20 bytes)
    let hash_len = gix_hash::Kind::Sha1.len_in_bytes();
    if let Ok(entry) = black_box(gix_pack::data::Entry::from_bytes(ctx.data, ctx.pack_offset, hash_len)) {
        // Exercise all accessors on the parsed entry
        _ = black_box(entry.header);
        _ = black_box(entry.decompressed_size);
        _ = black_box(entry.data_offset);
        _ = black_box(entry.header_size());
        _ = black_box(entry.pack_offset());
    }

    // Test the streaming reader path too
    let mut cursor = std::io::Cursor::new(ctx.data);
    _ = black_box(gix_pack::data::Entry::from_read(&mut cursor, ctx.pack_offset, hash_len));
});
