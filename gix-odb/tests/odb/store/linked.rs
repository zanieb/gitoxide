//! These are old tests of the now removed linked odb, but executed on the general store
//! to be sure we don't loose coverage. This might, however, be overlapping with much more thorough
//! tests o the general store itself, so they can possibly be removed at some point.
mod iter {
    use gix_odb::Header;
    use gix_pack::Find;

    use crate::db;

    #[test]
    fn a_bunch_of_loose_and_packed_objects() -> crate::Result {
        let db = db();
        let iter = db.iter()?;
        assert_eq!(
            iter.size_hint(),
            (139, None),
            "we only count packs and have no upper bound"
        );
        assert_eq!(iter.count(), 146, "it sees the correct amount of objects");
        for id in db.iter()? {
            let id = id?;
            assert!(db.contains(&id), "each object exists");
            assert!(db.try_header(&id)?.is_some(), "header is readable");
        }
        Ok(())
    }
}

mod locate {
    use gix_odb::Handle;
    use gix_pack::Find;

    use crate::{db, hex_to_id};

    fn can_locate(db: &Handle, hex_id: &str) {
        let mut buf = vec![];
        assert!(
            db.try_find(&hex_to_id(hex_id), &mut buf)
                .expect("no read error")
                .is_some()
        );
    }

    #[test]
    fn loose_object() {
        can_locate(&db(), "37d4e6c5c48ba0d245164c4e10d5f41140cab980");
    }

    #[test]
    fn pack_object() {
        can_locate(&db(), "501b297447a8255d3533c6858bb692575cdefaa0"); // pack 11fd
        can_locate(&db(), "4dac9989f96bc5b5b1263b582c08f0c5f0b58542"); // pack a2bf
        can_locate(&db(), "dd25c539efbb0ab018caa4cda2d133285634e9b5"); // pack c043
    }
}

mod init {
    use gix_hash::ObjectId;
    use gix_object::{Exists, Write};

    use crate::{alternate::alternate, db};

    #[test]
    fn multiple_linked_repositories_via_alternates() -> crate::Result {
        let tmp = gix_testtools::tempfile::TempDir::new()?;
        let (object_path, _linked_object_path) = alternate(tmp.path().join("a"), tmp.path().join("b"))?;
        let db = gix_odb::at(object_path.clone())?;
        db.exists(&ObjectId::null(gix_hash::Kind::Sha1)); // trigger load

        assert_eq!(db.store_ref().metrics().loose_dbs, 2);
        assert_eq!(db.iter()?.count(), 0, "the test locations are actually empty");
        assert_eq!(db.store_ref().path(), object_path);
        Ok(())
    }

    #[test]
    fn a_db_without_alternates() -> crate::Result {
        let tmp = gix_testtools::tempfile::TempDir::new()?;
        let db = gix_odb::at(tmp.path())?;
        db.exists(&ObjectId::null(gix_hash::Kind::Sha1)); // trigger load
        assert_eq!(db.store_ref().metrics().loose_dbs, 1);
        assert_eq!(db.store_ref().path(), tmp.path());
        Ok(())
    }

    #[test]
    fn alternates_can_be_disabled() -> crate::Result {
        let tmp = gix_testtools::tempfile::TempDir::new()?;
        let (object_path, linked_object_path) = alternate(tmp.path().join("a"), tmp.path().join("b"))?;
        let alternate_store = gix_odb::loose::Store::at(linked_object_path, gix_hash::Kind::Sha1, None);
        let alternate_id = alternate_store.write_buf(gix_object::Kind::Blob, b"alternate")?;

        let db_with_alternates = gix_odb::at(object_path.clone())?;
        assert!(
            db_with_alternates.exists(&alternate_id),
            "default stores see objects from alternates"
        );

        let db_without_alternates = gix_odb::at_opts(
            object_path,
            Vec::new(),
            gix_odb::store::init::Options {
                use_alternates: false,
                ..Default::default()
            },
        )?;
        assert!(
            !db_without_alternates.exists(&alternate_id),
            "primary-only stores ignore alternate objects"
        );
        assert_eq!(db_without_alternates.store_ref().metrics().loose_dbs, 1);
        assert!(
            db_without_alternates.store_ref().alternate_db_paths()?.is_empty(),
            "alternate paths are not resolved"
        );
        assert!(!db_without_alternates.store_ref().use_alternates());
        Ok(())
    }

    #[test]
    fn has_packs() {
        let db = db();
        db.exists(&ObjectId::null(gix_hash::Kind::Sha1)); // trigger load
        assert_eq!(db.store_ref().metrics().known_packs, 3);
    }
}
