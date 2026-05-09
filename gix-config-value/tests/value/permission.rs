use gix_config_value::Permission;

use crate::b;

#[test]
fn names_and_missing_value() -> crate::Result {
    assert_eq!(Permission::from_value(None)?, Permission::Group);
    assert_eq!(Permission::try_from(b("umask"))?, Permission::Umask);
    assert_eq!(Permission::try_from(b("group"))?, Permission::Group);
    assert_eq!(Permission::try_from(b("all"))?, Permission::Everybody);
    assert_eq!(Permission::try_from(b("world"))?, Permission::Everybody);
    assert_eq!(Permission::try_from(b("everybody"))?, Permission::Everybody);
    Ok(())
}

#[test]
fn names_ignore_case() -> crate::Result {
    assert_eq!(Permission::try_from(b("UMASK"))?, Permission::Umask);
    assert_eq!(Permission::try_from(b("GROUP"))?, Permission::Group);
    assert_eq!(Permission::try_from(b("EVERYBODY"))?, Permission::Everybody);
    Ok(())
}

#[test]
fn legacy_numeric_values() -> crate::Result {
    assert_eq!(Permission::try_from(b("0"))?, Permission::Umask);
    assert_eq!(Permission::try_from(b("1"))?, Permission::Group);
    assert_eq!(Permission::try_from(b("2"))?, Permission::Everybody);
    Ok(())
}

#[test]
fn boolean_values_match_git() -> crate::Result {
    assert_eq!(Permission::try_from(b("true"))?, Permission::Group);
    assert_eq!(Permission::try_from(b("yes"))?, Permission::Group);
    assert_eq!(Permission::try_from(b("false"))?, Permission::Umask);
    assert_eq!(Permission::try_from(b("no"))?, Permission::Umask);
    Ok(())
}

#[test]
fn octal_modes_are_restrictive_modes() -> crate::Result {
    assert_eq!(Permission::try_from(b("0660"))?, Permission::Mode(0o660));
    assert_eq!(Permission::try_from(b("660"))?, Permission::Mode(0o660));
    assert_eq!(Permission::try_from(b("0664"))?, Permission::Mode(0o664));
    assert_eq!(Permission::try_from(b("0775"))?, Permission::Mode(0o664));
    Ok(())
}

#[test]
fn octal_modes_need_owner_read_and_write() {
    assert!(Permission::try_from(b("0003")).is_err());
    assert!(Permission::try_from(b("0400")).is_err());
    assert!(Permission::try_from(b("0200")).is_err());
}

#[test]
fn invalid_values_fail() {
    assert!(Permission::try_from(b("maybe")).is_err());
    assert!(Permission::try_from(b("0660x")).is_err());
}

#[test]
fn invalid_octal_decimal_numbers_fall_back_to_boolean_values() -> crate::Result {
    assert_eq!(Permission::try_from(b("0888"))?, Permission::Group);
    assert_eq!(Permission::try_from(b("0008"))?, Permission::Group);
    Ok(())
}
