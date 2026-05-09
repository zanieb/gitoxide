fn store() -> crate::Result<crate::file::Store> {
    Ok(crate::file::Store::at(
        crate::scripted_fixture_read_only("make_repo_for_reflog.sh")?.join(".git"),
        gix_ref::store::init::Options {
            write_reflog: gix_ref::store::WriteReflog::Disable,
            object_hash: crate::fixture_hash_kind(),
            ..Default::default()
        },
    ))
}

mod iter_and_iter_rev {
    use crate::file::store::reflog::store;

    #[test]
    fn non_existing_and_directory_returns_none() -> crate::Result {
        let store = store()?;
        let mut buf = Vec::new();
        for name in &["FAILURE_NONEXISTING", "refs/heads"] {
            assert!(
                matches!(store.reflog_iter(*name, &mut buf), Ok(None)),
                "this one does not exist"
            );
        }
        Ok(())
    }

    #[test]
    fn for_head_and_main() -> crate::Result {
        let store = store()?;
        let mut buf = Vec::new();

        let log = store.reflog_iter("HEAD", &mut buf)?.expect("exists");
        assert_eq!(log.filter_map(Result::ok).count(), 5);

        let log = store.reflog_iter("refs/heads/main", &mut buf)?.expect("exists");
        assert_eq!(log.filter_map(Result::ok).count(), 5);
        Ok(())
    }
}

mod iter_rev {
    use crate::file::store::reflog::store;

    #[test]
    fn non_existing_and_directory_returns_none() -> crate::Result {
        let store = store()?;
        let mut buf = [0u8; 256];
        for name in &["FAILURE_NONEXISTING", "refs/heads"] {
            assert!(
                matches!(store.reflog_iter_rev(*name, &mut buf), Ok(None)),
                "this one does not exist"
            );
        }
        Ok(())
    }

    #[test]
    fn for_head_and_main() -> crate::Result {
        let store = store()?;
        let mut buf = [0u8; 256];

        let log = store.reflog_iter_rev("HEAD", &mut buf)?.expect("exists");
        assert_eq!(log.filter_map(Result::ok).count(), 5);

        let log = store.reflog_iter_rev("refs/heads/main", &mut buf)?.expect("exists");
        assert_eq!(log.filter_map(Result::ok).count(), 5);
        Ok(())
    }
}

mod expire {
    use gix_lock::acquire::Fail;
    use gix_object::bstr::BString;

    use crate::file::store_writable;

    fn messages(store: &gix_ref::file::Store, name: &str) -> crate::Result<Vec<BString>> {
        let mut buf = Vec::new();
        Ok(store
            .reflog_iter(name, &mut buf)?
            .expect("existing reflog")
            .map(|line| line.map(|line| line.message.to_owned()))
            .collect::<Result<Vec<_>, _>>()?)
    }

    #[test]
    fn rewrites_reflog_with_retained_entries() -> crate::Result {
        let (_keep, store) = store_writable("make_repo_for_reflog.sh")?;
        let before = messages(&store, "HEAD")?;
        assert_eq!(before.len(), 5);

        let mut index = 0;
        let removed = store.reflog_expire("HEAD", Fail::Immediately, |_| {
            index += 1;
            index % 2 == 1
        })?;

        assert_eq!(removed, 2);
        assert_eq!(
            messages(&store, "HEAD")?,
            before.into_iter().step_by(2).collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn missing_reflog_is_unchanged() -> crate::Result {
        let (_keep, store) = store_writable("make_repo_for_reflog.sh")?;
        let removed = store.reflog_expire("FAILURE_NONEXISTING", Fail::Immediately, |_| {
            unreachable!("missing reflog has no entries")
        })?;

        assert_eq!(removed, 0);
        Ok(())
    }
}
