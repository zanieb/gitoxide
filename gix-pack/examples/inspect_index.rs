#[cfg(feature = "sha1")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(index_path) = args.next() else {
        eprintln!("usage: inspect_index <path-to-pack-index.idx>");
        return Ok(());
    };

    let index = gix_pack::index::File::at(index_path, gix_hash::Kind::Sha1)?;

    println!(
        "{:?} index, {} object(s), {:?} object ids",
        index.version(),
        index.num_objects(),
        index.object_hash()
    );

    for entry in index.iter().take(5) {
        match entry.crc32 {
            Some(crc32) => println!("{} @ {} crc32={crc32:08x}", entry.oid, entry.pack_offset),
            None => println!("{} @ {}", entry.oid, entry.pack_offset),
        }
    }

    Ok(())
}

#[cfg(not(feature = "sha1"))]
fn main() {
    eprintln!("enable the 'sha1' feature to run this example");
}
