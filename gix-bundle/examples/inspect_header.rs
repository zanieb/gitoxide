fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: inspect_header <path-to-bundle>");
        return Ok(());
    };

    let (header, _pack_reader) = gix_bundle::header::from_path(path, gix_hash::Kind::Sha1)?;

    println!("{:?} bundle", header.version);
    if !header.capabilities.is_empty() {
        println!("capabilities:");
        for capability in &header.capabilities {
            println!("  {capability}");
        }
    }
    if !header.prerequisites.is_empty() {
        println!("prerequisites:");
        for prerequisite in &header.prerequisites {
            match &prerequisite.comment {
                Some(comment) => println!("  {} {comment}", prerequisite.id),
                None => println!("  {}", prerequisite.id),
            }
        }
    }
    println!("references:");
    for reference in &header.refs {
        println!("  {} {}", reference.id, reference.name);
    }

    Ok(())
}
