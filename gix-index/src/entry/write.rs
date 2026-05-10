use bstr::BStr;

use crate::{Entry, State, entry, util::write_var_int};

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
        let id = self.id.as_bytes();
        if id.len() != state.object_hash.len_in_bytes() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "entry object id length does not match index object hash",
            ));
        }
        out.write_all(id)?;
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

#[cfg(test)]
mod tests {
    use crate::{Entry, State, entry};

    #[test]
    fn write_to_rejects_object_hash_mismatch() {
        let mut state = State::new(gix_hash::Kind::Sha256);
        state.path_backing.extend_from_slice(b"a");
        state.entries.push(Entry {
            stat: entry::Stat::default(),
            id: gix_hash::ObjectId::from_bytes_or_panic(&[0; 20]),
            flags: entry::Flags::empty(),
            mode: entry::Mode::FILE,
            path: 0..1,
        });

        let mut out = Vec::new();
        let err = state.entries[0].write_to(&mut out, &state).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}
