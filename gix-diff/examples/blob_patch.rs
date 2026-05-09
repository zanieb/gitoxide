#[cfg(feature = "blob")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(old_path) = args.next() else {
        eprintln!("usage: blob_patch <old-file> <new-file> [path-in-diff]");
        return Ok(());
    };
    let Some(new_path) = args.next() else {
        eprintln!("usage: blob_patch <old-file> <new-file> [path-in-diff]");
        return Ok(());
    };
    let diff_path = args
        .next()
        .map_or_else(|| "file".to_owned(), |path| path.to_string_lossy().into_owned());

    let old = std::fs::read(old_path)?;
    let new = std::fs::read(new_path)?;

    gix_diff::blob::patch::write(
        &mut std::io::stdout().lock(),
        None,
        None,
        diff_path.as_str(),
        diff_path.as_str(),
        &old,
        &new,
        Default::default(),
    )?;

    Ok(())
}

#[cfg(not(feature = "blob"))]
fn main() {
    eprintln!("enable the 'blob' feature to run this example");
}
