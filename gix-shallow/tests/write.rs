#![cfg(feature = "sha1")]

fn id(hex: &str) -> gix_hash::ObjectId {
    gix_hash::ObjectId::from_hex(hex.as_bytes()).expect("valid hex")
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "gix-shallow-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos()
    ));
    path
}

#[test]
fn write_with_prune_removes_missing_objects_and_duplicates() -> Result<(), Box<dyn std::error::Error>> {
    let existing = id("1111111111111111111111111111111111111111");
    let missing = id("2222222222222222222222222222222222222222");
    let path = temp_path("write-with-prune");
    let _cleanup = Cleanup(path.clone());
    let initial = nonempty::NonEmpty {
        head: existing,
        tail: vec![missing, existing],
    };
    let lock = gix_lock::File::acquire_to_update_resource(&path, gix_lock::acquire::Fail::Immediately, None)?;

    gix_shallow::write_with_prune(
        lock,
        Some(initial),
        &[
            gix_shallow::Update::Shallow(existing),
            gix_shallow::Update::Shallow(missing),
        ],
        |id| id == &existing,
    )?;

    assert_eq!(std::fs::read_to_string(&path)?, format!("{}\n", existing.to_hex()));
    Ok(())
}

struct Cleanup(std::path::PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
