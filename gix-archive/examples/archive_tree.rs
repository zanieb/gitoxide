#[cfg(all(feature = "sha1", feature = "tar"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(objects_dir) = args.next() else {
        eprintln!("usage: archive_tree <path-to-.git/objects> <tree-id> <output.tar>");
        return Ok(());
    };
    let Some(tree_id) = args.next() else {
        eprintln!("usage: archive_tree <path-to-.git/objects> <tree-id> <output.tar>");
        return Ok(());
    };
    let Some(output_path) = args.next() else {
        eprintln!("usage: archive_tree <path-to-.git/objects> <tree-id> <output.tar>");
        return Ok(());
    };

    let tree_id = gix_hash::ObjectId::from_hex(tree_id.to_string_lossy().as_bytes())?;
    let objects = gix_odb::at(objects_dir, tree_id.kind())?.into_arc()?;
    let pipeline = gix_filter::Pipeline::new(Default::default(), tree_id.kind(), Default::default());
    let mut stream = gix_worktree_stream::from_tree(
        tree_id,
        objects,
        pipeline,
        |_, _, _| -> Result<(), std::convert::Infallible> { Ok(()) },
    );

    let output = std::fs::File::create(output_path)?;
    gix_archive::write_stream(
        &mut stream,
        gix_worktree_stream::Stream::next_entry,
        std::io::BufWriter::new(output),
        gix_archive::Options {
            format: gix_archive::Format::Tar,
            tree_prefix: None,
            modification_time: 0,
        },
    )?;

    Ok(())
}

#[cfg(not(all(feature = "sha1", feature = "tar")))]
fn main() {
    eprintln!("enable the 'sha1' and 'tar' features to run this example");
}
