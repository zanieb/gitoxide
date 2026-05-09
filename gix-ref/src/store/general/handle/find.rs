use gix_error::{Result, ResultExt, message};

use crate::{PartialNameRef, Reference, store};

use crate::store::handle;

impl store::Handle {
    /// TODO: actually implement this with handling of the packed buffer.
    pub fn try_find<'a, Name, E>(&self, partial: Name) -> Result<Option<Reference>>
    where
        Name: TryInto<&'a PartialNameRef, Error = E>,
        std::result::Result<&'a PartialNameRef, E>: ResultExt<Success = &'a PartialNameRef>,
    {
        let _name = partial.try_into().or_raise(|| message("Invalid reference name"))?;
        match &self.state {
            handle::State::Loose { .. } => {
                todo!()
            }
        }
    }
}

impl store::Handle {
    /// Similar to [`crate::file::Store::find()`] but a non-existing ref is treated as error.
    pub fn find<'a, Name, E>(&self, _partial: Name) -> Result<Reference>
    where
        Name: TryInto<&'a PartialNameRef, Error = E>,
        std::result::Result<&'a PartialNameRef, E>: ResultExt<Success = &'a PartialNameRef>,
    {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use crate::store::{init::Options, WriteReflog};

    type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn handle_delegates_find_to_loose_store() -> Result {
        let tmp = gix_testtools::tempfile::TempDir::new()?;
        let refs_dir = tmp.path().join("refs/heads");
        std::fs::create_dir_all(&refs_dir)?;
        std::fs::write(refs_dir.join("main"), "28ce6a8b26aa170e1de65536fe8abe1832bd3242\n")?;

        let store = crate::Store::at(
            tmp.path().to_owned(),
            Options {
                write_reflog: WriteReflog::Normal,
                object_hash: gix_hash::Kind::Sha1,
                ..Default::default()
            },
        )?;
        let handle = store.to_handle();

        assert_eq!(
            handle.try_find("main")?.expect("present").name.as_bstr(),
            "refs/heads/main"
        );
        assert!(handle.try_find("missing")?.is_none());

        match handle.find("missing") {
            Err(super::existing::Error::NotFound { name }) => assert_eq!(name, std::path::Path::new("missing")),
            other => panic!("expected not found, got {other:?}"),
        }
        assert!(matches!(
            handle.try_find("../escaping"),
            Err(super::Error::RefnameValidation(_))
        ));

        Ok(())
    }
}
