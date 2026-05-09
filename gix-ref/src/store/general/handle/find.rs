use crate::{PartialNameRef, Reference, store};

mod error {
    use std::convert::Infallible;

    /// The error returned by [`crate::file::Store::find_loose()`].
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error("An error occurred while finding a reference in the loose file database")]
        Loose(#[from] crate::file::find::Error),
        #[error("The ref name or path is not a valid ref name")]
        RefnameValidation(#[from] crate::name::Error),
    }

    impl From<Infallible> for Error {
        fn from(_: Infallible) -> Self {
            unreachable!("this impl is needed to allow passing a known valid partial path as parameter")
        }
    }
}

pub use error::Error;

use crate::store::handle;

impl store::Handle {
    /// Find a single reference by the given `partial` name.
    pub fn try_find<'a, Name, E>(&self, partial: Name) -> Result<Option<Reference>, Error>
    where
        Name: TryInto<&'a PartialNameRef, Error = E>,
        Error: From<E>,
    {
        let name = partial.try_into()?;
        match &self.state {
            handle::State::Loose { store, .. } => store.try_find(name).map_err(Error::Loose),
        }
    }
}

mod existing {
    mod error {
        use std::path::PathBuf;

        /// The error returned by [file::Store::find_existing()][crate::file::Store::find_existing()].
        #[derive(Debug, thiserror::Error)]
        #[allow(missing_docs)]
        pub enum Error {
            #[error("An error occurred while finding a reference in the database")]
            Find(#[from] crate::store::find::Error),
            #[error("The ref partially named {name:?} could not be found")]
            NotFound { name: PathBuf },
        }
    }

    pub use error::Error;

    use crate::{PartialNameRef, Reference, store};

    impl store::Handle {
        /// Similar to [`crate::file::Store::find()`] but a non-existing ref is treated as error.
        pub fn find<'a, Name, E>(&self, partial: Name) -> Result<Reference, Error>
        where
            Name: TryInto<&'a PartialNameRef, Error = E>,
            crate::name::Error: From<E>,
        {
            let name = partial
                .try_into()
                .map_err(|err| super::Error::RefnameValidation(err.into()))?;
            match self.try_find(name) {
                Ok(Some(reference)) => Ok(reference),
                Ok(None) => Err(Error::NotFound {
                    name: name.to_partial_path().to_owned(),
                }),
                Err(err) => Err(err.into()),
            }
        }
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
