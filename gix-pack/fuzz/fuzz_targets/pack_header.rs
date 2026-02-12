#![no_main]

use libfuzzer_sys::fuzz_target;
use std::hint::black_box;

fuzz_target!(|data: &[u8]| {
    // Fuzz the pack header decoder with arbitrary data.
    // Pack headers are exactly 12 bytes: "PACK" + version(u32 BE) + num_objects(u32 BE)
    if data.len() >= 12 {
        let header: &[u8; 12] = data[..12].try_into().unwrap();
        if let Ok((version, num_objects)) = black_box(gix_pack::data::header::decode(header)) {
            // If decode succeeds, roundtrip through encode and verify
            let encoded = gix_pack::data::header::encode(version, num_objects);
            let (version2, num_objects2) = gix_pack::data::header::decode(&encoded)
                .expect("roundtrip encode->decode must succeed");
            assert_eq!(version, version2, "version roundtrip mismatch");
            assert_eq!(num_objects, num_objects2, "num_objects roundtrip mismatch");
        }
    }
});
