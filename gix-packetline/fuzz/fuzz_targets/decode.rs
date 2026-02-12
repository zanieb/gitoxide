#![no_main]

use libfuzzer_sys::fuzz_target;
use std::hint::black_box;

fuzz_target!(|data: &[u8]| {
    // Fuzz packet line decoding - this parses the git protocol wire format.
    //
    // Packet line format:
    // - 4 hex bytes indicating length (including the 4 prefix bytes)
    // - Special values: "0000" (flush), "0001" (delimiter), "0002" (response end)
    // - Length 1-3 is invalid, length 4 means empty data (also invalid)
    // - Data follows the 4-byte prefix
    //
    // This is security-critical: packet lines are the first thing parsed from
    // a remote git server's response.

    // Test the all-at-once decoder
    match black_box(gix_packetline::decode(data)) {
        Ok(line) => {
            // Exercise all accessors
            _ = black_box(line.as_slice());
            _ = black_box(line.as_bstr());
            _ = black_box(line.as_text());
            _ = black_box(line.as_error());
            _ = black_box(line.check_error());

            // If it's a data line, try decoding as sideband
            if line.as_slice().is_some() {
                _ = black_box(line.decode_band());
            }
        }
        Err(_e) => {
            _ = black_box(_e);
        }
    }

    // Also test hex_prefix if we have at least 4 bytes
    if data.len() >= 4 {
        match black_box(gix_packetline::decode::hex_prefix(&data[..4])) {
            Ok(gix_packetline::decode::PacketLineOrWantedSize::Line(line)) => {
                _ = black_box(line);
            }
            Ok(gix_packetline::decode::PacketLineOrWantedSize::Wanted(size)) => {
                _ = black_box(size);
            }
            Err(_e) => {
                _ = black_box(_e);
            }
        }
    }

    // Test to_data_line
    _ = black_box(gix_packetline::decode::to_data_line(data));
});
