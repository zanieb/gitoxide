fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "git@github.com:GitoxideLabs/gitoxide.git".to_owned());
    let url = gix_url::Url::try_from(input.as_str())?;

    println!("scheme: {}", url.scheme);
    println!("host: {}", url.host().unwrap_or("(none)"));
    println!("path: {}", String::from_utf8_lossy(url.path.as_ref()));
    println!("display: {url}");
    let serialized = url.to_bstring();
    println!("serialized: {}", String::from_utf8_lossy(serialized.as_ref()));

    Ok(())
}
