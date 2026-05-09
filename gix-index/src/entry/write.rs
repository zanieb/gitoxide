use bstr::BStr;

use crate::{entry, util::write_var_int, Entry, State};

impl Entry {
    /// Serialize ourselves to `out` with path access via `state`, without padding.
    pub fn write_to(&self, mut out: impl std::io::Write, state: &State) -> std::io::Result<()> {
        self.write_header_to(&mut out, state)?;
        let path = self.path(state);
        out.write_all(path)?;
        out.write_all(b"\0")
    }

    /// Serialize ourselves as a V4 entry with a path delta against `previous_path`, without padding.
    pub fn write_v4_to(
        &self,
        mut out: impl std::io::Write,
        state: &State,
        previous_path: Option<&BStr>,
    ) -> std::io::Result<()> {
        self.write_header_to(&mut out, state)?;
        let path = self.path(state);
        let common_prefix_len = previous_path
            .map(|previous_path| common_prefix_len(previous_path, path))
            .unwrap_or_default();
        let strip_len = previous_path
            .map(|previous_path| previous_path.len() - common_prefix_len)
            .unwrap_or_default();
        write_var_int(strip_len.try_into().expect("path length fits u64"), &mut out)?;
        out.write_all(&path[common_prefix_len..])?;
        out.write_all(b"\0")
    }

    fn write_header_to(&self, mut out: impl std::io::Write, state: &State) -> std::io::Result<()> {
        let stat = self.stat;
        out.write_all(&stat.ctime.secs.to_be_bytes())?;
        out.write_all(&stat.ctime.nsecs.to_be_bytes())?;
        out.write_all(&stat.mtime.secs.to_be_bytes())?;
        out.write_all(&stat.mtime.nsecs.to_be_bytes())?;
        out.write_all(&stat.dev.to_be_bytes())?;
        out.write_all(&stat.ino.to_be_bytes())?;
        out.write_all(&self.mode.bits().to_be_bytes())?;
        out.write_all(&stat.uid.to_be_bytes())?;
        out.write_all(&stat.gid.to_be_bytes())?;
        out.write_all(&stat.size.to_be_bytes())?;
        out.write_all(self.id.as_bytes())?;
        let path = self.path(state);
        let path_len: u16 = if path.len() >= entry::Flags::PATH_LEN.bits() as usize {
            entry::Flags::PATH_LEN.bits() as u16
        } else {
            path.len()
                .try_into()
                .expect("we just checked that the length is smaller than 0xfff")
        };
        out.write_all(&(self.flags.to_storage().bits() | path_len).to_be_bytes())?;
        if self.flags.contains(entry::Flags::EXTENDED) {
            out.write_all(
                &entry::at_rest::FlagsExtended::from_flags(self.flags)
                    .bits()
                    .to_be_bytes(),
            )?;
        }
        Ok(())
    }
}

fn common_prefix_len(previous: &BStr, current: &BStr) -> usize {
    previous
        .iter()
        .zip(current.iter())
        .take_while(|(previous, current)| previous == current)
        .count()
}
