use bstr::BString;

use crate::{
    extension::{FsMonitor, Signature},
    util::{read_u32, read_u64, split_at_byte_exclusive},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    V1 { nanos_since_1970: u64 },
    V2 { token: BString },
}

pub const SIGNATURE: Signature = *b"FSMN";

pub fn decode(data: &[u8]) -> Option<FsMonitor> {
    let (version, data) = read_u32(data)?;
    let (token, data) = match version {
        1 => {
            let (nanos_since_1970, data) = read_u64(data)?;
            (Token::V1 { nanos_since_1970 }, data)
        }
        2 => {
            let (token, data) = split_at_byte_exclusive(data, 0)?;
            (Token::V2 { token: token.into() }, data)
        }
        _ => return None,
    };

    let (ewah_size, data) = read_u32(data)?;
    let ((entry_dirty, extra), data) = data
        .split_at_checked(ewah_size as usize)
        .and_then(|(entry_dirty, data)| {
            gix_bitmap::ewah::decode(entry_dirty)
                .ok()
                .map(|entry_dirty| (entry_dirty, data))
        })?;
    if !extra.is_empty() {
        return None;
    }

    if !data.is_empty() {
        return None;
    }

    FsMonitor { token, entry_dirty }.into()
}

/// Serialize an fsmonitor extension to `out`.
pub fn write_to(fs_monitor: &FsMonitor, mut out: impl std::io::Write) -> Result<(), std::io::Error> {
    use std::io::Write as _;

    let mut data = Vec::new();
    match &fs_monitor.token {
        Token::V1 { nanos_since_1970 } => {
            data.write_all(&1_u32.to_be_bytes())?;
            data.write_all(&nanos_since_1970.to_be_bytes())?;
        }
        Token::V2 { token } => {
            data.write_all(&2_u32.to_be_bytes())?;
            data.write_all(token)?;
            data.write_all(b"\0")?;
        }
    }

    let mut bitmap = Vec::new();
    fs_monitor.entry_dirty.write_to(&mut bitmap)?;
    data.write_all(&(u32::try_from(bitmap.len()).expect("less than 4GB fsmonitor bitmap")).to_be_bytes())?;
    data.write_all(&bitmap)?;

    out.write_all(&SIGNATURE)?;
    out.write_all(&(u32::try_from(data.len()).expect("less than 4GB fsmonitor extension")).to_be_bytes())?;
    out.write_all(&data)
}
