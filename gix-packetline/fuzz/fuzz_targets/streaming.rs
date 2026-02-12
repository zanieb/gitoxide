#![no_main]

use libfuzzer_sys::fuzz_target;
use std::hint::black_box;

fuzz_target!(|data: &[u8]| {
    // Fuzz the streaming packet line decoder.
    //
    // The streaming decoder is designed for incremental parsing where data arrives
    // in chunks. It either returns a complete packet line or indicates how many
    // more bytes are needed.
    //
    // This is the primary entry point for parsing git protocol data from network
    // streams, making it critical for security.

    // Test streaming decode - parse as many packet lines as possible from the input
    let mut remaining = data;
    let mut parsed_count = 0u32;

    while !remaining.is_empty() && parsed_count < 1000 {
        match black_box(gix_packetline::decode::streaming(remaining)) {
            Ok(gix_packetline::decode::Stream::Complete { line, bytes_consumed }) => {
                assert!(bytes_consumed > 0, "must consume at least one byte on success");
                assert!(
                    bytes_consumed <= remaining.len(),
                    "consumed bytes must not exceed input length"
                );

                // Exercise the parsed line
                _ = black_box(line.as_slice());
                _ = black_box(line.as_bstr());
                _ = black_box(line.as_text());
                if line.as_slice().is_some() {
                    _ = black_box(line.decode_band());
                }

                remaining = &remaining[bytes_consumed..];
                parsed_count += 1;
            }
            Ok(gix_packetline::decode::Stream::Incomplete { bytes_needed }) => {
                assert!(bytes_needed > 0, "must need at least one byte");
                break;
            }
            Err(_e) => {
                _ = black_box(_e);
                break;
            }
        }
    }
});
