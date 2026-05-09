#[cfg(feature = "sha1")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use gix_odb::HeaderExt;

    let mut args = std::env::args_os().skip(1);
    let Some(objects_dir) = args.next() else {
        eprintln!("usage: header <path-to-.git/objects> <object-id>");
        return Ok(());
    };
    let Some(id) = args.next() else {
        eprintln!("usage: header <path-to-.git/objects> <object-id>");
        return Ok(());
    };

    let odb = gix_odb::at(objects_dir)?;
    let id = gix_hash::ObjectId::from_hex(id.to_string_lossy().as_bytes())?;
    let header = odb.header(id)?;

    println!("{} {}", header.kind(), header.size());
    Ok(())
}

#[cfg(not(feature = "sha1"))]
fn main() {
    eprintln!("enable the 'sha1' feature to run this example");
}
