#![no_main]
use libfuzzer_sys::fuzz_target;
use std::hint::black_box;

fuzz_target!(|data: &[u8]| {
    // Fuzz the varint decoder: must not panic on arbitrary input.
    let _ = black_box(gix_reftable::read_varint(data));
});
