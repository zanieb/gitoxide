use std::{borrow::Cow, fmt::Display};

use bstr::{BStr, BString};

use crate::{Boolean, Error, Permission};

const PERM_UMASK: i64 = 0;
const OLD_PERM_GROUP: i64 = 1;
const OLD_PERM_EVERYBODY: i64 = 2;

fn permission_err(input: impl Into<BString>) -> Error {
    Error::new(
        "Permissions need to be 'umask', 'group', 'all', 'world', 'everybody', a boolean, or an octal file mode",
        input,
    )
}

fn invalid_mode_err(input: impl Into<BString>) -> Error {
    Error::new(
        "Permission file modes must grant the owner read and write permissions",
        input,
    )
}

impl Permission {
    /// Parse an optional permission value as Git does for `core.sharedRepository`.
    ///
    /// A missing value is treated like `group`, matching Git's implicit value for this key.
    pub fn from_value(value: Option<&BStr>) -> Result<Self, Error> {
        value.map_or(Ok(Self::Group), Self::try_from)
    }

    /// Return the exact custom file mode, if this permission was parsed from one.
    pub fn as_mode(self) -> Option<u16> {
        match self {
            Permission::Mode(mode) => Some(mode),
            Permission::Umask | Permission::Group | Permission::Everybody => None,
        }
    }
}

impl TryFrom<&BStr> for Permission {
    type Error = Error;

    fn try_from(value: &BStr) -> Result<Self, Self::Error> {
        if value.eq_ignore_ascii_case(b"umask") {
            return Ok(Self::Umask);
        }
        if value.eq_ignore_ascii_case(b"group") {
            return Ok(Self::Group);
        }
        if value.eq_ignore_ascii_case(b"all")
            || value.eq_ignore_ascii_case(b"world")
            || value.eq_ignore_ascii_case(b"everybody")
        {
            return Ok(Self::Everybody);
        }

        let value_str = std::str::from_utf8(value).map_err(|err| permission_err(value).with_err(err))?;
        if let Some(permission) = parse_octal(value_str, value)? {
            return Ok(permission);
        }

        Boolean::try_from(value).map(|boolean| if boolean.0 { Self::Group } else { Self::Umask })
    }
}

impl TryFrom<Cow<'_, BStr>> for Permission {
    type Error = Error;

    fn try_from(c: Cow<'_, BStr>) -> Result<Self, Self::Error> {
        Self::try_from(c.as_ref())
    }
}

impl Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Permission::Umask => f.write_str("umask"),
            Permission::Group => f.write_str("group"),
            Permission::Everybody => f.write_str("everybody"),
            Permission::Mode(mode) => write!(f, "{mode:04o}"),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Permission {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

fn parse_octal(value: &str, input: &BStr) -> Result<Option<Permission>, Error> {
    let digits = value.strip_prefix('+').unwrap_or(value);
    if digits.is_empty() && !value.is_empty() {
        return Ok(None);
    }

    let Some(value) = digits.chars().try_fold(0i64, |acc, c| {
        c.to_digit(8)
            .map(|digit| acc.saturating_mul(8).saturating_add(digit as i64))
    }) else {
        return Ok(None);
    };

    let permission = match value {
        PERM_UMASK => Permission::Umask,
        OLD_PERM_GROUP => Permission::Group,
        OLD_PERM_EVERYBODY => Permission::Everybody,
        value => {
            if !(0..=0o777).contains(&value) || (value & 0o600) != 0o600 {
                return Err(invalid_mode_err(input));
            }
            Permission::Mode((value & 0o666) as u16)
        }
    };
    Ok(Some(permission))
}
