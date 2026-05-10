#[cfg(feature = "sha1")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let index_path = std::env::args_os().nth(1).unwrap_or_else(|| ".git/index".into());

    let index = gix_index::File::at(
        index_path,
        gix_index::hash::Kind::Sha1,
        false,
        gix_index::decode::Options::default(),
    )?;

    println!(
        "{:?} index with {} entries, {:?} object ids",
        index.version(),
        index.entries().len(),
        index.object_hash()
    );

    for entry in index.entries().iter().take(10) {
        println!("{:?} {} stage {}", entry.mode, entry.path(&index), entry.stage_raw());
    }

    Ok(())
}

#[cfg(not(feature = "sha1"))]
fn main() {
    eprintln!("enable the 'sha1' feature to run this example");
}
