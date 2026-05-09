#[cfg(feature = "blocking-client")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "file:///path/to/repository.git".to_owned());

    let options = gix_transport::client::blocking_io::connect::Options {
        version: gix_transport::Protocol::V2,
        ..Default::default()
    };
    let _transport = gix_transport::client::blocking_io::connect::connect(url.as_str(), options)?;

    println!("connected to {url}");
    Ok(())
}

#[cfg(not(feature = "blocking-client"))]
fn main() {
    eprintln!("enable the 'blocking-client' feature to run this example");
}
