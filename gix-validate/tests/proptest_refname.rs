//! Property-based tests for gix_validate reference name validation.
//!
//! Tests invariants of the ref name validation functions:
//! - Valid names pass validation
//! - Sanitized names always pass partial validation
//! - Specific invalid patterns are always rejected

use bstr::ByteSlice;
use proptest::prelude::*;

/// Generate strings that should be valid partial ref names.
fn arb_valid_partial_refname() -> impl Strategy<Value = String> {
    // Valid ref name components: alphanumeric, hyphen, underscore, dot (not leading/trailing)
    // Pattern: one or more path components separated by /
    let component = "[a-zA-Z][a-zA-Z0-9_-]{0,10}";
    proptest::string::string_regex(&format!("{component}(/{component}){{0,3}}")).expect("valid regex")
}

proptest! {
    /// Names matching the valid pattern should pass partial validation.
    #[test]
    fn valid_partial_names_pass(name in arb_valid_partial_refname()) {
        let result = gix_validate::reference::name_partial(name.as_bytes().as_bstr());
        prop_assert!(result.is_ok(), "expected valid partial refname: {name:?}, got error: {:?}", result.err());
    }

    /// Sanitized output of name_partial_or_sanitize must always pass name_partial.
    #[test]
    fn sanitize_then_validate_roundtrip(input in "\\PC{1,50}") {
        let sanitized = gix_validate::reference::name_partial_or_sanitize(input.as_bytes().as_bstr());
        let result = gix_validate::reference::name_partial(sanitized.as_bytes().as_bstr());
        prop_assert!(
            result.is_ok(),
            "sanitized name should pass validation: input={input:?}, sanitized={sanitized:?}, err={:?}",
            result.err()
        );
    }

    /// Empty input must always fail validation.
    #[test]
    fn empty_always_fails(_dummy in 0u8..1) {
        let result = gix_validate::reference::name_partial(b"".as_bstr());
        prop_assert!(result.is_err(), "empty name must fail validation");
        let result = gix_validate::reference::name(b"".as_bstr());
        prop_assert!(result.is_err(), "empty name must fail full validation");
    }

    /// Names containing control characters (0x00-0x1F, 0x7F) must fail validation.
    #[test]
    fn control_chars_rejected(
        prefix in "[a-z]{1,5}",
        ctrl in 0u8..0x20,
        suffix in "[a-z]{1,5}"
    ) {
        let name = format!("{prefix}{}{suffix}", ctrl as char);
        let result = gix_validate::reference::name_partial(name.as_bytes().as_bstr());
        prop_assert!(result.is_err(), "control char 0x{ctrl:02x} should be rejected in: {name:?}");
    }

    /// Names containing '..' must fail validation.
    #[test]
    fn double_dot_rejected(
        prefix in "[a-z]{1,5}",
        suffix in "[a-z]{1,5}"
    ) {
        let name = format!("{prefix}..{suffix}");
        let result = gix_validate::reference::name_partial(name.as_bytes().as_bstr());
        prop_assert!(result.is_err(), "double dot should be rejected in: {name:?}");
    }

    /// Names ending with '.lock' must fail validation.
    #[test]
    fn lock_suffix_rejected(prefix in "[a-zA-Z][a-zA-Z0-9]{0,10}") {
        let name = format!("{prefix}.lock");
        let result = gix_validate::reference::name_partial(name.as_bytes().as_bstr());
        prop_assert!(result.is_err(), ".lock suffix should be rejected in: {name:?}");
    }

    /// Names containing '@{' must fail validation.
    #[test]
    fn reflog_portion_rejected(
        prefix in "[a-z]{1,5}",
        suffix in "[a-z]{1,5}"
    ) {
        let name = format!("{prefix}@{{{suffix}");
        let result = gix_validate::reference::name_partial(name.as_bytes().as_bstr());
        prop_assert!(result.is_err(), "@{{ should be rejected in: {name:?}");
    }
}
