use gix_error::{Result, ResultExt};

use crate::{PartialNameRef, Reference, store};

use crate::store::handle;

impl store::Handle {
    /// Find a reference by its full or partial name.
    pub fn try_find<'a, Name, E>(&self, partial: Name) -> Result<Option<Reference>>
    where
        Name: TryInto<&'a PartialNameRef, Error = E>,
        std::result::Result<&'a PartialNameRef, E>: ResultExt<Success = &'a PartialNameRef>,
    {
        match &self.state {
            handle::State::Loose { store } => store.try_find(partial),
        }
    }
}

impl store::Handle {
    /// Similar to [`crate::file::Store::find()`] but a non-existing ref is treated as error.
    pub fn find<'a, Name, E>(&self, partial: Name) -> Result<Reference>
    where
        Name: TryInto<&'a PartialNameRef, Error = E>,
        std::result::Result<&'a PartialNameRef, E>: ResultExt<Success = &'a PartialNameRef>,
    {
        match &self.state {
            handle::State::Loose { store } => store.find(partial),
        }
    }
}

#[cfg(test)]
mod tests {

    type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn handle_delegates_find_to_loose_store() -> Result {
        let tmp = gix_testtools::tempfile::TempDir::new()?;
        let refs_dir = tmp.path().join("refs/heads");
        std::fs::create_dir_all(&refs_dir)?;
        std::fs::write(refs_dir.join("main"), "28ce6a8b26aa170e1de65536fe8abe1832bd3242\n")?;

        let store = crate::Store::at(tmp.path().to_owned(), gix_hash::Kind::Sha1)?;
        let handle = store.to_handle();

        assert_eq!(
            handle.try_find("main")?.expect("present").name.as_bstr(),
            "refs/heads/main"
        );
        assert!(handle.try_find("missing")?.is_none());

        assert!(handle.find("missing").expect_err("missing reference").is_not_found());
        assert!(
            handle
                .try_find("../escaping")
                .expect_err("invalid name")
                .is_validation()
        );

        Ok(())
    }
}
