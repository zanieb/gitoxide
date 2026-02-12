#![no_main]
use libfuzzer_sys::fuzz_target;
use std::hint::black_box;

fuzz_target!(|data: &[u8]| {
    // Fuzz the ref record parser: must not panic on arbitrary input.
    // Provide an empty prefix (first record in block), SHA1 hash size, and update_index=0.
    let _ = black_box(gix_reftable::parse_ref_record(data, b"", 20, 0));
});
