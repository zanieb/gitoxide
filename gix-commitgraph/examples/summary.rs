use std::path::PathBuf;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map_or_else(|| ".git/objects/info".into(), PathBuf::from);
    let graph = match gix_commitgraph::Graph::at(&path) {
        Ok(graph) => graph,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    println!("hash: {:?}", graph.object_hash());
    println!("commits: {}", graph.num_commits());
    for (idx, id) in graph.iter_ids().take(5).enumerate() {
        println!("{idx}: {id}");
    }
}
