use gix_object::{bstr::ByteSlice, Kind, ObjectRef};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(kind) = args.next() else {
        eprintln!("usage: decode_object <blob|tree|commit|tag> <path-to-raw-object-body>");
        return Ok(());
    };
    let Some(path) = args.next() else {
        eprintln!("usage: decode_object <blob|tree|commit|tag> <path-to-raw-object-body>");
        return Ok(());
    };

    let kind = Kind::from_bytes(kind.to_string_lossy().as_bytes())?;
    let data = std::fs::read(path)?;

    match ObjectRef::from_bytes(kind, &data)? {
        ObjectRef::Blob(blob) => println!("blob: {} bytes", blob.data.len()),
        ObjectRef::Tree(tree) => println!("tree: {} entries", tree.entries.len()),
        ObjectRef::Commit(commit) => {
            println!(
                "commit: tree {}, {} parent(s), {} byte message",
                commit.tree,
                commit.parents.len(),
                commit.message.len()
            );
        }
        ObjectRef::Tag(tag) => {
            println!(
                "tag: {} -> {}, {} byte message",
                tag.name.to_str_lossy(),
                tag.target_kind,
                tag.message.len()
            );
        }
    }

    Ok(())
}
