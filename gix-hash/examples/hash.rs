use gix_hash::{Kind, ObjectId, Prefix};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let expected = ObjectId::empty_blob(Kind::Sha1);
    let parsed = ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")?;
    parsed.verify(expected.as_ref())?;

    let short = Prefix::new(parsed.as_ref(), 7)?;
    println!("{} {parsed} {short}", parsed.kind());

    Ok(())
}
