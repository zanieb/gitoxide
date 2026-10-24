use std::{borrow::Cow, fmt::Display, time::SystemTime};

use bstr::BStr;
use gix_error::{Error, ResultExt, validation};

use crate::Date;

impl Date {
    /// Parse a date from `input` using `now` as the base time for relative dates.
    ///
    /// This accepts the same formats as [`gix_date::parse()`], including raw commit-header dates,
    /// RFC 2822 dates, ISO 8601 variants, UNIX timestamps, and relative dates like `2 weeks ago`.
    pub fn try_from_with_now(input: &BStr, now: Option<SystemTime>) -> Result<Self, Error> {
        let date = std::str::from_utf8(input).or_raise(|| date_err(input))?;
        let now = now
            .map(gix_date::Zoned::try_from)
            .transpose()
            .or_raise(|| date_err(input))?;
        gix_date::parse(date, now).map(Date).or_raise(|| date_err(input))
    }

    /// Return the contained time value.
    pub fn into_inner(self) -> gix_date::Time {
        self.0
    }
}

impl Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

fn date_err(input: &BStr) -> gix_error::Message {
    validation("Invalid Git date").with_input(input)
}

impl TryFrom<&BStr> for Date {
    type Error = Error;

    fn try_from(input: &BStr) -> Result<Self, Self::Error> {
        Date::try_from_with_now(input, Some(SystemTime::now()))
    }
}

impl TryFrom<Cow<'_, BStr>> for Date {
    type Error = Error;

    fn try_from(input: Cow<'_, BStr>) -> Result<Self, Self::Error> {
        Self::try_from(input.as_ref())
    }
}
