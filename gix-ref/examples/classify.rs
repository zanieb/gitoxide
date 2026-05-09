use gix_ref::{bstr::ByteSlice, Category, FullName};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "main".to_owned());

    let name = if input.starts_with("refs/") || input == "HEAD" {
        FullName::try_from(input.as_str())?
    } else {
        Category::LocalBranch.to_full_name(input.as_bytes().as_bstr())?
    };

    println!("full name: {name}");
    println!("short name: {}", name.shorten());
    println!("path: {}", name.to_path().display());

    match name.category_and_short_name() {
        Some((category, short_name)) => {
            println!("category: {category:?}");
            println!("category-local name: {short_name}");
            println!("worktree private: {}", category.is_worktree_private());
        }
        None => println!("category: unclassified"),
    }

    Ok(())
}
