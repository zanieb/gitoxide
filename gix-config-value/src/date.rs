use std::{borrow::Cow, fmt::Display, time::SystemTime};

use bstr::{BStr, BString};

use crate::{Date, Error};

impl Date {
    /// Parse a date from `input` using `now` as the base time for relative dates.
    ///
    /// This accepts the same formats as [`gix_date::parse()`], including raw commit-header dates,
    /// RFC 2822 dates, ISO 8601 variants, UNIX timestamps, and relative dates like `2 weeks ago`.
    pub fn try_from_with_now(input: &BStr, now: Option<SystemTime>) -> Result<Self, Error> {
        let date = std::str::from_utf8(input).map_err(|err| date_err(input).with_err(err))?;
        gix_date::parse(date, now).map(Date).map_err(|_| date_err(input))
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

fn date_err(input: impl Into<BString>) -> Error {
    Error::new(
        "Dates need to be parseable by git, like '1970-01-01', '1745582210 +0200', or '2 weeks ago'",
        input,
    )
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
