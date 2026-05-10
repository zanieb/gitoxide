#[cfg(feature = "sha1")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use gix_object::bstr::ByteSlice;

    let mut args = std::env::args_os().skip(1);
    let Some(worktree_path) = args.next() else {
        eprintln!("usage: annotate <path-to-worktree> <path-in-worktree>");
        return Ok(());
    };
    let Some(file_path) = args.next() else {
        eprintln!("usage: annotate <path-to-worktree> <path-in-worktree>");
        return Ok(());
    };

    let worktree_path = std::path::PathBuf::from(worktree_path);
    let git_dir = worktree_path.join(".git");
    let file_path = file_path.to_string_lossy();

    let refs = gix_ref::file::Store::at(
        git_dir.clone(),
        gix_ref::store::init::Options {
            write_reflog: gix_ref::store::WriteReflog::Disable,
            ..Default::default()
        },
    );
    let objects = gix_odb::at(git_dir.join("objects"))?;
    let mut head = gix_ref::file::Store::find(&refs, "HEAD")?;

    use gix_ref::file::ReferenceExt;
    let head_id = head.peel_to_id(&refs, &objects)?;

    let index = gix_index::File::at(git_dir.join("index"), gix_hash::Kind::Sha1, false, Default::default())?;
    let attr_stack = gix_worktree::Stack::from_state_and_ignore_case(
        worktree_path,
        false,
        gix_worktree::stack::State::AttributesAndIgnoreStack {
            attributes: Default::default(),
            ignore: Default::default(),
        },
        &index,
        index.path_backing(),
    );
    let fs = gix_fs::Capabilities::probe(&git_dir);
    let mut resource_cache = gix_diff::blob::Platform::new(
        Default::default(),
        gix_diff::blob::Pipeline::new(
            gix_diff::blob::pipeline::WorktreeRoots {
                old_root: None,
                new_root: None,
            },
            gix_filter::Pipeline::new(Default::default(), Default::default()),
            vec![],
            gix_diff::blob::pipeline::Options {
                large_file_threshold_bytes: 0,
                fs,
            },
        ),
        gix_diff::blob::pipeline::Mode::ToGit,
        attr_stack,
    );
    let should_interrupt = std::sync::atomic::AtomicBool::new(false);
    let outcome = gix_blame::file(
        &objects,
        head_id,
        None,
        &mut resource_cache,
        file_path.as_bytes().as_bstr(),
        Default::default(),
        &should_interrupt,
    )?;

    for (entry, lines) in outcome.entries_with_lines() {
        println!(
            "{} lines {}-{}",
            entry.commit_id,
            entry.start_in_blamed_file + 1,
            entry.start_in_blamed_file + entry.len.get()
        );
        for line in lines {
            print!("    {}", String::from_utf8_lossy(line.as_ref()));
        }
    }

    Ok(())
}

#[cfg(not(feature = "sha1"))]
fn main() {
    eprintln!("enable the 'sha1' feature to run this example");
}
