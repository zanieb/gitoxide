use std::time::{Duration, SystemTime};

use gix_config_value::Date;

use crate::{b, cow_str};

#[test]
fn from_str_with_absolute_formats() {
    for input in [
        "2018-12-24",
        "Thu, 18 Aug 2022 12:45:06 +0800",
        "2022-08-17T21:43:13+08:00",
        "1745582210 +0200",
    ] {
        assert_eq!(
            Date::try_from_with_now(b(input), None).unwrap().into_inner(),
            gix_date::parse(input, None).unwrap()
        );
    }
}

#[test]
fn relative_dates_can_use_a_stable_base_time() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let date = Date::try_from_with_now(b("2 minutes ago"), Some(now)).unwrap();

    assert_eq!(date.into_inner(), gix_date::Time::new(880, 0));
}

#[test]
fn display_uses_git_raw_date_format() {
    assert_eq!(
        Date::try_from_with_now(b("1745582210 +0200"), None)
            .unwrap()
            .to_string(),
        "1745582210 +0200"
    );
}

#[test]
fn invalid_from_str() {
    assert!(Date::try_from(b("")).is_err());
    assert!(Date::try_from(b("not a date")).is_err());
    assert!(Date::try_from(b("™️🤦‍♂️")).is_err());
}

#[test]
fn try_from_cow() {
    assert_eq!(
        Date::try_from(cow_str("1745582210 +0200")).unwrap().to_string(),
        "1745582210 +0200"
    );
}
