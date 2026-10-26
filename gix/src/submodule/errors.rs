///
pub mod init {
    /// The error returned by [`Submodule::init()`](crate::Submodule::init()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        Operation(#[from] gix_error::Error),
        #[error("Could not write the local repository configuration")]
        WriteConfig(#[from] std::io::Error),
        #[error("Submodule path '{}' contains a symbolic link at '{}'", path.display(), symlink.display())]
        SymlinkInPath {
            /// The submodule path that was being validated.
            path: std::path::PathBuf,
            /// The component that is a symbolic link.
            symlink: std::path::PathBuf,
        },
        #[error("Submodule name '{name}' cannot be used below .git/modules")]
        InvalidName {
            /// The invalid submodule name.
            name: crate::bstr::BString,
        },
    }
}

///
#[cfg(all(feature = "blocking-network-client", feature = "worktree-mutation"))]
pub mod update {
    /// The error returned by [`Submodule::update()`](crate::Submodule::update()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error("The submodule cannot fetch the requested commit from a local repository offline")]
        Offline,
        #[error(transparent)]
        Operation(#[from] gix_error::Error),
        #[error(transparent)]
        Init(#[from] super::init::Error),
        #[error(transparent)]
        GitDirLayout(#[from] crate::submodule::git_dir_layout::Error),
        #[error("Failed to spawn submodule update command '{command}'")]
        CommandSpawn {
            /// The command that was configured.
            command: crate::bstr::BString,
            /// The underlying IO error.
            source: std::io::Error,
        },
        #[error("Submodule update command '{command}' exited with status {status}")]
        CommandFailed {
            /// The command that was configured.
            command: crate::bstr::BString,
            /// The failing process status.
            status: std::process::ExitStatus,
        },
        #[error("Failed to update nested submodule '{name}'")]
        RecursiveUpdate {
            /// The nested submodule name.
            name: crate::bstr::BString,
            /// The nested update error.
            source: Box<Error>,
        },
        #[error(
            "The submodule update strategy '{strategy:?}' requires the 'revision' feature to compare commit ancestry"
        )]
        StrategyNeedsRevisionFeature {
            /// The update strategy that requires ancestry checks.
            strategy: gix_submodule::config::Update,
        },
        #[error(
            "The submodule update strategy '{strategy:?}' requires the 'merge' feature for non-fast-forward updates"
        )]
        StrategyNeedsMergeFeature {
            /// The update strategy that requires merge support.
            strategy: gix_submodule::config::Update,
        },
        #[error("The submodule update strategy '{strategy:?}' has unresolved conflicts from {head} to {target}")]
        StrategyConflict {
            /// The update strategy that produced conflicts.
            strategy: gix_submodule::config::Update,
            /// The current submodule HEAD.
            head: gix_hash::ObjectId,
            /// The commit recorded in the superproject.
            target: gix_hash::ObjectId,
        },
        #[error("The submodule update strategy '{strategy:?}' cannot replay merge commit {commit}")]
        StrategyNeedsLinearHistory {
            /// The update strategy that requires a linear branch history.
            strategy: gix_submodule::config::Update,
            /// The local merge commit that could not be replayed.
            commit: gix_hash::ObjectId,
        },
        #[error("Failed to create index from tree for submodule checkout")]
        IndexFromTree {
            /// The tree id that failed.
            id: gix_hash::ObjectId,
            /// The underlying error.
            source: gix_error::Error,
        },
        #[error("Local changes to '{path}' would be overwritten by submodule checkout")]
        CheckoutWouldOverwrite {
            /// The path with local changes.
            path: crate::bstr::BString,
        },
        #[error("Could not read worktree path '{path}' while checking for local submodule changes")]
        ReadWorktree {
            /// The path that could not be read.
            path: crate::bstr::BString,
            /// The underlying IO error.
            source: std::io::Error,
        },
        #[error("Could not hash worktree path '{path}' while checking for local submodule changes")]
        HashWorktree {
            /// The path that could not be hashed.
            path: crate::bstr::BString,
            /// The underlying hashing error.
            source: gix_error::Error,
        },
        #[error("Failed to reopen object database as Arc")]
        OpenArcOdb(std::io::Error),
        #[error("Failed to find commit object in submodule repository")]
        FindObject(gix_error::Error),
        #[error("Failed to peel commit to tree in submodule repository")]
        PeelToTree(gix_error::Error),
        #[error("Submodule repository has no working directory")]
        MissingWorkdir,
        #[error("Submodule path '{}' contains a symbolic link at '{}'", path.display(), symlink.display())]
        SymlinkInPath {
            /// The submodule path that was being validated.
            path: std::path::PathBuf,
            /// The component that is a symbolic link.
            symlink: std::path::PathBuf,
        },
        #[error("Submodule name '{name}' cannot be used below .git/modules")]
        InvalidName {
            /// The invalid submodule name.
            name: crate::bstr::BString,
        },
    }
}
