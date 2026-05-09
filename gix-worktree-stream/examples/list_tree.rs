#[cfg(feature = "sha1")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    let mut args = std::env::args_os().skip(1);
    let Some(objects_dir) = args.next() else {
        eprintln!("usage: list_tree <path-to-.git/objects> <tree-id>");
        return Ok(());
    };
    let Some(tree_id) = args.next() else {
        eprintln!("usage: list_tree <path-to-.git/objects> <tree-id>");
        return Ok(());
    };

    let objects = gix_odb::at(objects_dir)?.into_arc()?;
    let tree_id = gix_hash::ObjectId::from_hex(tree_id.to_string_lossy().as_bytes())?;
    let pipeline = gix_filter::Pipeline::new(Default::default(), Default::default());
    let mut stream = gix_worktree_stream::from_tree(
        tree_id,
        objects,
        pipeline,
        |_, _, _| -> Result<(), std::convert::Infallible> { Ok(()) },
    );

    while let Some(mut entry) = stream.next_entry().map_err(gix_error::Exn::into_error)? {
        let mut out = Vec::new();
        entry.read_to_end(&mut out)?;
        println!(
            "{} {:?} {} {} byte(s)",
            entry.relative_path(),
            entry.mode.kind(),
            entry.id,
            out.len()
        );
    }

    Ok(())
}

#[cfg(not(feature = "sha1"))]
fn main() {
    eprintln!("enable the 'sha1' feature to run this example");
}
