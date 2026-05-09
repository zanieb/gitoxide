#[cfg(feature = "blocking-client")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let capabilities = gix_protocol::transport::client::Capabilities::from_lines(
        "version 2\nfetch=shallow filter sideband-all packfile-uris\nls-refs=unborn".into(),
    )?;

    let fetch_features =
        gix_protocol::Command::Fetch.default_features(gix_protocol::transport::Protocol::V2, &capabilities);
    let fetch_arguments = gix_protocol::Command::Fetch.initial_v2_arguments(&fetch_features);
    gix_protocol::Command::Fetch.validate_argument_prefixes(
        gix_protocol::transport::Protocol::V2,
        &capabilities,
        &fetch_arguments,
        &fetch_features,
    )?;

    println!("fetch features:");
    for (name, value) in &fetch_features {
        match value {
            Some(value) => println!("  {name}={value}"),
            None => println!("  {name}"),
        }
    }
    println!("fetch arguments:");
    for argument in &fetch_arguments {
        println!("  {}", String::from_utf8_lossy(argument.as_ref()));
    }

    let ls_refs_features =
        gix_protocol::Command::LsRefs.default_features(gix_protocol::transport::Protocol::V2, &capabilities);
    let ls_refs_arguments = gix_protocol::Command::LsRefs.initial_v2_arguments(&ls_refs_features);
    gix_protocol::Command::LsRefs.validate_argument_prefixes(
        gix_protocol::transport::Protocol::V2,
        &capabilities,
        &ls_refs_arguments,
        &ls_refs_features,
    )?;

    println!("ls-refs arguments:");
    for argument in &ls_refs_arguments {
        println!("  {}", String::from_utf8_lossy(argument.as_ref()));
    }

    Ok(())
}

#[cfg(not(feature = "blocking-client"))]
fn main() {
    eprintln!("enable the 'blocking-client' feature to run this example");
}
