use crate::Result;

fn store() -> Result<crate::file::Store> {
    Ok(crate::file::Store::at_opts(
        crate::scripted_fixture_read_only("make_repo_for_reflog.sh")?.join(".git"),
        crate::fixture_hash_kind(),
        gix_ref::store::init::Options {
            write_reflog: gix_ref::store::WriteReflog::Disable,
            ..Default::default()
        },
    ))
}

mod iter_and_iter_rev {
    use crate::file::store::reflog::store;

    #[cfg(unix)]
    #[test]
    fn read_failures_preserve_context() -> gix_testtools::TestResult {
        let mut error_snapshots = Vec::new();
        let store = store()?;
        let name = "refs/heads/main/child";
        let mut buf = vec![0; 256];
        for err in [
            store.reflog_iter(name, &mut buf).err().expect("main is a file"),
            store.reflog_iter_rev(name, &mut buf).err().expect("main is a file"),
        ] {
            let details = err.metadata().next().expect("reflog read context");
            assert!(
                err.downcast_any_ref::<gix_error::Message>().is_some(),
                "the reflog read diagnostic retains its concrete type"
            );
            assert_eq!(details.len(), 1, "read context contains only the path");
            assert_eq!(
                details["path"],
                gix_error::MetadataValue::Path(store.git_dir().join("logs").join(name)),
                "both directions retain the resolved reflog path as a native path"
            );
            error_snapshots.push(gix_testtools::redact_debug_snapshot(
                &(err),
                &[(&(store.git_dir()).to_string_lossy(), "<git-dir>")],
            ));
            assert_eq!(
                err.downcast_any_ref::<std::io::Error>()
                    .expect("the original open error is retained")
                    .kind(),
                std::io::ErrorKind::NotADirectory,
                "a file at a path prefix retains its I/O error kind on Unix"
            );
            assert!(
                !err.is_corrupted(),
                "a path collision does not imply corrupt reflog contents"
            );
        }
        insta::assert_debug_snapshot!(error_snapshots, "read failures preserve context", @r#"
        [
            Could not read reflog, path="<git-dir>/logs/refs/heads/main/child"
            
            Caused by:
                0: NotADirectory,
            Could not read reflog, path="<git-dir>/logs/refs/heads/main/child"
            
            Caused by:
                0: NotADirectory,
        ]
        "#);
        Ok(())
    }

    #[test]
    fn non_existing_and_directory_returns_none() -> gix_testtools::TestResult {
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
    fn for_head_and_main() -> gix_testtools::TestResult {
        let store = store()?;
        let mut buf = Vec::new();

        let log = store.reflog_iter("HEAD", &mut buf)?.expect("exists");
        assert_eq!(log.filter_map(std::result::Result::ok).count(), 5);

        let log = store.reflog_iter("refs/heads/main", &mut buf)?.expect("exists");
        assert_eq!(log.filter_map(std::result::Result::ok).count(), 5);
        Ok(())
    }
}

mod iter_rev {
    use crate::file::store::reflog::store;

    #[test]
    fn non_existing_and_directory_returns_none() -> gix_testtools::TestResult {
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
    fn for_head_and_main() -> gix_testtools::TestResult {
        let store = store()?;
        let mut buf = [0u8; 256];

        let log = store.reflog_iter_rev("HEAD", &mut buf)?.expect("exists");
        assert_eq!(log.filter_map(std::result::Result::ok).count(), 5);

        let log = store.reflog_iter_rev("refs/heads/main", &mut buf)?.expect("exists");
        assert_eq!(log.filter_map(std::result::Result::ok).count(), 5);
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
