use gix_traverse::commit::{
    simple::{CommitTimeOrder, Sorting},
    Parents, Simple,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(objects_dir) = args.next() else {
        eprintln!("usage: commit_ancestors <path-to-.git/objects> <tip-commit-id>");
        return Ok(());
    };
    let Some(tip) = args.next() else {
        eprintln!("usage: commit_ancestors <path-to-.git/objects> <tip-commit-id>");
        return Ok(());
    };

    let odb = gix_odb::at(objects_dir)?;
    let tip = gix_hash::ObjectId::from_hex(tip.to_string_lossy().as_bytes())?;
    let commit_graph = gix_commitgraph::at(odb.store_ref().path().join("info")).ok();

    for commit in Simple::new([tip], &odb)
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))?
        .parents(Parents::All)
        .commit_graph(commit_graph)
    {
        println!("{}", commit?.id);
    }

    Ok(())
}
