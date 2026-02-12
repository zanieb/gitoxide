//! Property-based tests for gix_date::Time format/parse roundtrip.
//!
//! The "raw" format used in git commit headers is: `<seconds> +HHMM` or `<seconds> -HHMM`.
//! We verify that Time -> write_to -> parse_header roundtrips correctly.

use gix_date::{parse::TimeBuf, Time};
use proptest::prelude::*;

/// Generate a valid Time with reasonable offset constraints.
///
/// Git offsets are stored as total seconds, but serialized as +/-HHMM (max +/-9959).
/// Hours must be <= 99, minutes 0..59, so max absolute offset = 99*3600 + 59*60 = 360_000.
fn arb_time() -> impl Strategy<Value = Time> {
    (
        // Seconds since epoch: git supports negative timestamps too
        -1_000_000_000i64..2_000_000_000i64,
        // Offset in seconds: constrained to valid HHMM range
        0u32..6000u32,
    )
        .prop_map(|(seconds, total_minutes)| {
            let hours = (total_minutes / 60).min(99);
            let minutes = total_minutes % 60;
            let offset_secs = (hours * 3600 + minutes * 60) as i32;
            // Randomly make negative by using the seconds LSB
            let offset = if seconds % 2 == 0 { offset_secs } else { -offset_secs };
            Time::new(seconds, offset)
        })
}

proptest! {
    /// Time -> write_to -> parse_header must roundtrip.
    #[test]
    fn time_raw_format_roundtrip(time in arb_time()) {
        let mut buf = TimeBuf::default();
        let formatted = time.to_str(&mut buf);

        // Parse it back
        let parsed = gix_date::parse_header(formatted);
        let parsed = match parsed {
            Some(t) => t,
            None => {
                return Err(proptest::test_runner::TestCaseError::fail(
                    format!("parse_header failed for: {:?}", formatted)
                ));
            }
        };

        prop_assert_eq!(time.seconds, parsed.seconds);
        prop_assert_eq!(time.offset, parsed.offset);
    }

    /// Time::to_str must produce a well-formed "<seconds> <+/->HHMM" string.
    #[test]
    fn time_format_structure(time in arb_time()) {
        let mut buf = TimeBuf::default();
        let formatted = time.to_str(&mut buf);

        let parts: Vec<&str> = formatted.split(' ').collect();
        prop_assert_eq!(parts.len(), 2);

        // First part must be a valid integer
        let _seconds: i64 = parts[0].parse()
            .map_err(|e| proptest::test_runner::TestCaseError::fail(
                format!("seconds part not a valid integer: {}", e)
            ))?;

        // Second part must be +HHMM or -HHMM (5 chars)
        let offset_str = parts[1];
        prop_assert_eq!(offset_str.len(), 5);

        let sign = &offset_str[..1];
        prop_assert!(sign == "+" || sign == "-", "offset must start with + or -, got {:?}", sign);

        let hh: u32 = offset_str[1..3].parse()
            .map_err(|e| proptest::test_runner::TestCaseError::fail(
                format!("hours not valid: {}", e)
            ))?;
        let mm: u32 = offset_str[3..5].parse()
            .map_err(|e| proptest::test_runner::TestCaseError::fail(
                format!("minutes not valid: {}", e)
            ))?;

        prop_assert!(hh <= 99, "hours must be <= 99, got {}", hh);
        prop_assert!(mm <= 59, "minutes must be <= 59, got {}", mm);
    }
}
