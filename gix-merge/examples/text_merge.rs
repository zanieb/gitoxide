fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(ancestor_path) = args.next() else {
        eprintln!("usage: text_merge <ancestor-file> <current-file> <other-file>");
        return Ok(());
    };
    let Some(current_path) = args.next() else {
        eprintln!("usage: text_merge <ancestor-file> <current-file> <other-file>");
        return Ok(());
    };
    let Some(other_path) = args.next() else {
        eprintln!("usage: text_merge <ancestor-file> <current-file> <other-file>");
        return Ok(());
    };

    let ancestor = std::fs::read(ancestor_path)?;
    let current = std::fs::read(current_path)?;
    let other = std::fs::read(other_path)?;

    let mut merged = Vec::new();
    let mut input = imara_diff::intern::InternedInput::default();
    let resolution = gix_merge::blob::builtin_driver::text(
        &mut merged,
        &mut input,
        Default::default(),
        &current,
        &ancestor,
        &other,
        Default::default(),
    );

    eprintln!("resolution: {resolution:?}");
    std::io::Write::write_all(&mut std::io::stdout().lock(), &merged)?;

    Ok(())
}
