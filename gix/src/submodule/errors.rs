///
pub mod open_modules_file {
    /// The error returned by [Repository::open_modules_file()](crate::Repository::open_modules_file()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        Configuration(#[from] gix_config::parse::Error),
        #[error("Could not read '.gitmodules' file")]
        Io(#[from] std::io::Error),
    }
}

///
pub mod modules {
    /// The error returned by [Repository::modules()](crate::Repository::modules()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        OpenModulesFile(#[from] crate::submodule::open_modules_file::Error),
        #[error(transparent)]
        OpenIndex(#[from] crate::worktree::open_index::Error),
        #[error("Could not find the .gitmodules file by id in the object database")]
        FindExistingBlob(#[from] crate::object::find::existing::Error),
        #[error(transparent)]
        FindHeadRef(#[from] crate::reference::find::existing::Error),
        #[error(transparent)]
        PeelHeadRef(#[from] crate::head::peel::Error),
        #[error(transparent)]
        PeelObjectToCommit(#[from] crate::object::peel::to_kind::Error),
        #[error(transparent)]
        TreeFromCommit(#[from] crate::object::commit::Error),
    }
}

///
pub mod is_active {
    /// The error returned by [Submodule::is_active()](crate::Submodule::is_active()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        InitIsActivePlatform(#[from] gix_submodule::is_active_platform::Error),
        #[error(transparent)]
        QueryIsActive(#[from] gix_config::value::Error),
        #[error(transparent)]
        InitAttributes(#[from] crate::config::attribute_stack::Error),
        #[error(transparent)]
        InitPathspecDefaults(#[from] gix_pathspec::defaults::from_environment::Error),
        #[error(transparent)]
        ObtainIndex(#[from] crate::repository::index_or_load_from_head::Error),
    }
}

///
pub mod fetch_recurse {
    /// The error returned by [Submodule::fetch_recurse()](crate::Submodule::fetch_recurse()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        ModuleBoolean(#[from] gix_submodule::config::Error),
        #[error(transparent)]
        ConfigurationFallback(#[from] crate::config::key::GenericErrorWithValue),
    }
}

///
pub mod open {
    /// The error returned by [Submodule::open()](crate::Submodule::open()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        OpenRepository(#[from] crate::open::Error),
        #[error(transparent)]
        PathConfiguration(#[from] gix_submodule::config::path::Error),
        #[error(transparent)]
        WorktreeDirInaccessible(#[from] std::io::Error),
    }
}

///
pub mod index_id {
    /// The error returned by [Submodule::index_id()](crate::Submodule::index_id()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        PathConfiguration(#[from] gix_submodule::config::path::Error),
        #[error(transparent)]
        Index(#[from] crate::repository::index_or_load_from_head::Error),
    }
}

///
pub mod head_id {
    /// The error returned by [Submodule::head_id()](crate::Submodule::head_id()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        HeadCommit(#[from] crate::reference::head_commit::Error),
        #[error("Could not get tree of head commit")]
        CommitTree(#[from] crate::object::commit::Error),
        #[error("Could not peel tree to submodule path")]
        PeelTree(#[from] crate::object::find::existing::Error),
        #[error(transparent)]
        PathConfiguration(#[from] gix_submodule::config::path::Error),
    }
}

///
pub mod init {
    /// The error returned by [`Submodule::init()`](crate::Submodule::init()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        PathConfiguration(#[from] gix_submodule::config::path::Error),
        #[error(transparent)]
        UrlConfiguration(#[from] gix_submodule::config::url::Error),
        #[error(transparent)]
        UpdateConfiguration(#[from] gix_submodule::config::update::Error),
        #[error("Could not write the local repository configuration")]
        WriteConfig(#[from] std::io::Error),
        #[error(transparent)]
        ReadConfig(#[from] gix_config::file::init::from_paths::Error),
        #[error("Submodule path '{}' contains a symbolic link at '{}'", path.display(), symlink.display())]
        SymlinkInPath {
            /// The submodule path that was being validated.
            path: std::path::PathBuf,
            /// The component that is a symbolic link.
            symlink: std::path::PathBuf,
        },
    }
}

///
#[cfg(feature = "blocking-network-client")]
pub mod update {
    /// The error returned by [`Submodule::update()`](crate::Submodule::update()).
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error(transparent)]
        Init(#[from] super::init::Error),
        #[error(transparent)]
        PathConfiguration(#[from] gix_submodule::config::path::Error),
        #[error(transparent)]
        UrlConfiguration(#[from] gix_submodule::config::url::Error),
        #[error(transparent)]
        UpdateConfiguration(#[from] gix_submodule::config::update::Error),
        #[error(transparent)]
        IndexId(#[from] super::index_id::Error),
        #[error(transparent)]
        Clone(#[from] crate::clone::Error),
        #[error(transparent)]
        CloneFetch(#[from] crate::clone::fetch::Error),
        #[error(transparent)]
        CloneCheckout(#[from] crate::clone::checkout::main_worktree::Error),
        #[error(transparent)]
        OpenRepository(#[from] super::open::Error),
        #[error(transparent)]
        Fetch(#[from] crate::remote::fetch::Error),
        #[error(transparent)]
        FetchConnect(#[from] crate::remote::connect::Error),
        #[error(transparent)]
        FetchPrepareFetch(#[from] crate::remote::fetch::prepare::Error),
        #[error(transparent)]
        FindRemote(#[from] crate::remote::find::existing::Error),
        #[error(transparent)]
        HeadSet(#[from] crate::reference::edit::Error),
        #[error("The submodule update strategy is '{command}' which requires running an external command, but this is not yet supported")]
        CommandUnsupported {
            /// The command that was configured.
            command: crate::bstr::BString,
        },
        #[error("The submodule update strategy 'rebase' is not yet supported")]
        RebaseUnsupported,
        #[error("The submodule update strategy 'merge' is not yet supported")]
        MergeUnsupported,
        #[error("Failed to create index from tree for submodule checkout")]
        IndexFromTree {
            /// The tree id that failed.
            id: gix_hash::ObjectId,
            /// The underlying error.
            source: gix_index::init::from_tree::Error,
        },
        #[error(transparent)]
        CheckoutOptions(#[from] crate::config::checkout_options::Error),
        #[error(transparent)]
        IndexCheckout(#[from] gix_worktree_state::checkout::Error),
        #[error(transparent)]
        WriteIndex(#[from] gix_index::file::write::Error),
        #[error(transparent)]
        BooleanConfig(#[from] crate::config::boolean::Error),
        #[error("Failed to reopen object database as Arc")]
        OpenArcOdb(std::io::Error),
        #[error(transparent)]
        SubmoduleModules(#[from] super::modules::Error),
        #[error(transparent)]
        IsActive(#[from] super::is_active::Error),
        #[error(transparent)]
        RefMap(#[from] crate::remote::ref_map::Error),
        #[error("Failed to find commit object in submodule repository")]
        FindObject(#[from] crate::object::find::existing::Error),
        #[error("Failed to peel commit to tree in submodule repository")]
        PeelToTree(#[from] crate::object::peel::to_kind::Error),
        #[error("Submodule repository has no working directory")]
        MissingWorkdir,
        #[error(transparent)]
        GitDirLayout(#[from] crate::submodule::git_dir_layout::Error),
        #[error("Submodule path '{}' contains a symbolic link at '{}'", path.display(), symlink.display())]
        SymlinkInPath {
            /// The submodule path that was being validated.
            path: std::path::PathBuf,
            /// The component that is a symbolic link.
            symlink: std::path::PathBuf,
        },
    }
}
