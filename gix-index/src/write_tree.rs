use bstr::{BStr, ByteSlice};
use gix_hash::ObjectId;
use gix_object::tree::{EntryKind, EntryMode};
use smallvec::SmallVec;

use crate::{entry, extension, State};

/// The error returned by [`State::write_tree_to()`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("Entry at '{path}' has an invalid mode {mode:#o} that cannot be represented in a tree")]
    InvalidEntryMode { path: bstr::BString, mode: u32 },
    #[error("Entry at '{path}' has a null object id")]
    NullObjectId { path: bstr::BString },
    #[error("Cannot write tree: entry at '{path}' is at stage {stage} (unresolved conflict)")]
    UnmergedEntry { path: bstr::BString, stage: u32 },
    #[error("Could not write tree object")]
    WriteTree(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// A tree and its computed object id as produced by [`State::write_tree_to()`].
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The object id of the root tree that was written.
    pub tree_id: ObjectId,
}

impl State {
    /// Create tree objects from the current index entries and write them using `write`.
    ///
    /// The `write` closure receives each [`Tree`](gix_object::Tree) object and must persist it,
    /// returning the [`ObjectId`] that was assigned to it. Trees are presented in depth-first
    /// order (leaves before parents), so the last call to `write` will be for the root tree.
    ///
    /// Only entries at [`Stage::Unconflicted`](entry::Stage::Unconflicted) (stage 0) are
    /// considered. Entries with higher stages (merge conflicts) cause an error.
    ///
    /// The [`TREE` extension](extension::Tree) is updated as a side-effect, caching the
    /// computed tree ids for future use.
    ///
    /// # Errors
    ///
    /// - If any entry is at a non-zero stage (conflict marker)
    /// - If any entry has a null object id
    /// - If any entry mode cannot be converted to a valid tree entry mode
    /// - If `write` returns an error
    pub fn write_tree_to<E>(
        &mut self,
        mut write: impl FnMut(&gix_object::Tree) -> Result<ObjectId, E>,
    ) -> Result<Outcome, Error>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        let _span = gix_features::trace::coarse!("gix_index::State::write_tree_to()");

        // Validate: no conflicts allowed
        for entry in &self.entries {
            if entry.flags.contains(entry::Flags::REMOVE) {
                continue;
            }
            if entry.stage() != entry::Stage::Unconflicted {
                return Err(Error::UnmergedEntry {
                    path: bstr::BString::from(entry.path(self).as_bytes()),
                    stage: entry.stage_raw(),
                });
            }
        }

        // Build the tree hierarchy from the flat sorted entries.
        // We use a stack-based approach: maintain a stack of (directory_name, entries, children)
        // where children are sub-tree results.
        let root_node = build_tree_hierarchy(self)?;
        let (tree_id, tree_ext) = write_trees_recursive(&root_node, &mut write)?;

        self.tree = Some(tree_ext);

        Ok(Outcome { tree_id })
    }
}

/// Represents a node in the tree hierarchy built from index entries.
struct TreeNode {
    /// The name of this directory component (empty for root).
    name: SmallVec<[u8; 23]>,
    /// Direct blob/symlink/submodule entries in this directory.
    entries: Vec<(SmallVec<[u8; 23]>, EntryMode, ObjectId)>,
    /// Sub-directories.
    children: Vec<TreeNode>,
    /// Total number of index entries reachable from this node (including sub-trees, recursively).
    num_entries: u32,
    /// If this node represents a sparse directory, its tree OID is already known.
    /// In that case, `entries` and `children` are empty and this OID is used directly.
    sparse_tree_oid: Option<ObjectId>,
}

fn build_tree_hierarchy(state: &State) -> Result<TreeNode, Error> {
    // We process the sorted entries and build a tree structure.
    // The stack tracks the current path of directories we're inside.
    // stack[0] is the root, stack[n] is the deepest directory.
    let mut stack: Vec<TreeNode> = vec![TreeNode {
        name: SmallVec::new(),
        entries: Vec::new(),
        children: Vec::new(),
        num_entries: 0,
        sparse_tree_oid: None,
    }];

    for entry in &state.entries {
        if entry.flags.contains(entry::Flags::REMOVE) {
            continue;
        }
        // Skip intent-to-add entries, matching C Git's cache-tree.c behavior.
        // These entries have placeholder OIDs and should not appear in trees.
        if entry.flags.contains(entry::Flags::INTENT_TO_ADD) {
            continue;
        }

        let path = entry.path(state);

        if entry.id.is_null() {
            return Err(Error::NullObjectId {
                path: bstr::BString::from(path.as_bytes()),
            });
        }

        // Handle sparse directory entries: these have Mode::DIR and their OID
        // is already a tree OID. We treat them as pre-computed subtree nodes,
        // matching C Git's cache-tree.c behavior for S_ISSPARSEDIR entries.
        if entry.mode.is_sparse() {
            // The path of a sparse directory entry is the directory path itself
            // (e.g., "some/dir"). Split it into parent components and the dir name.
            let (dir_components, dir_name) = split_path(path);

            let common_depth = common_prefix_depth(&stack, &dir_components);

            // Pop directories that are no longer in the path.
            while stack.len() > common_depth + 1 {
                let child = stack.pop().expect("stack not empty");
                let parent = stack.last_mut().expect("root always present");
                parent.num_entries += child.num_entries;
                parent.children.push(child);
            }

            // Push new directories that we need to enter.
            for component in &dir_components[common_depth..] {
                stack.push(TreeNode {
                    name: SmallVec::from_slice(component),
                    entries: Vec::new(),
                    children: Vec::new(),
                    num_entries: 0,
                    sparse_tree_oid: None,
                });
            }

            // Add the sparse directory as a child node with its tree OID pre-set.
            let current = stack.last_mut().expect("root always present");
            current.children.push(TreeNode {
                name: SmallVec::from_slice(dir_name),
                entries: Vec::new(),
                children: Vec::new(),
                num_entries: 1,
                sparse_tree_oid: Some(entry.id),
            });
            current.num_entries += 1;
            continue;
        }

        let mode = entry.mode.to_tree_entry_mode().ok_or_else(|| Error::InvalidEntryMode {
            path: bstr::BString::from(path.as_bytes()),
            mode: entry.mode.bits(),
        })?;

        // Split path into directory components and filename.
        let (dir_components, filename) = split_path(path);

        // Navigate to the correct directory, creating nodes as needed.
        // First, figure out how deep the current stack matches `dir_components`.
        let common_depth = common_prefix_depth(&stack, &dir_components);

        // Pop directories that are no longer in the path.
        while stack.len() > common_depth + 1 {
            let child = stack.pop().expect("stack not empty");
            let parent = stack.last_mut().expect("root always present");
            parent.num_entries += child.num_entries;
            parent.children.push(child);
        }

        // Push new directories that we need to enter.
        for component in &dir_components[common_depth..] {
            stack.push(TreeNode {
                name: SmallVec::from_slice(component),
                entries: Vec::new(),
                children: Vec::new(),
                num_entries: 0,
                sparse_tree_oid: None,
            });
        }

        // Add the entry to the current (deepest) directory.
        let current = stack.last_mut().expect("root always present");
        current.entries.push((SmallVec::from_slice(filename), mode, entry.id));
        current.num_entries += 1;
    }

    // Pop remaining directories back to root.
    while stack.len() > 1 {
        let child = stack.pop().expect("stack not empty");
        let parent = stack.last_mut().expect("root always present");
        parent.num_entries += child.num_entries;
        parent.children.push(child);
    }

    Ok(stack.into_iter().next().expect("root always present"))
}

/// Split a path like `a/b/c.txt` into (["a", "b"], "c.txt").
fn split_path(path: &BStr) -> (SmallVec<[&[u8]; 4]>, &[u8]) {
    let bytes = path.as_bytes();
    if let Some(last_slash) = bytes.iter().rposition(|&b| b == b'/') {
        let dir_part = &bytes[..last_slash];
        let filename = &bytes[last_slash + 1..];
        let components: SmallVec<[&[u8]; 4]> = dir_part.split(|&b| b == b'/').collect();
        (components, filename)
    } else {
        (SmallVec::new(), bytes)
    }
}

/// Determine how many levels of the current stack match the given directory components.
fn common_prefix_depth(stack: &[TreeNode], dir_components: &[&[u8]]) -> usize {
    // stack[0] is root (name = ""), stack[1..] are nested dirs.
    // dir_components[0..] are the path components.
    let max_check = (stack.len() - 1).min(dir_components.len());
    for i in 0..max_check {
        if stack[i + 1].name.as_slice() != dir_components[i] {
            return i;
        }
    }
    max_check
}

/// Recursively write tree objects bottom-up and produce the extension::Tree cache.
fn write_trees_recursive<E>(
    node: &TreeNode,
    write: &mut impl FnMut(&gix_object::Tree) -> Result<ObjectId, E>,
) -> Result<(ObjectId, extension::Tree), Error>
where
    E: std::error::Error + Send + Sync + 'static,
{
    // If this node is a sparse directory with a pre-computed tree OID,
    // use it directly without building or writing a tree object.
    if let Some(tree_oid) = node.sparse_tree_oid {
        let ext_tree = extension::Tree {
            name: node.name.clone(),
            id: tree_oid,
            num_entries: Some(node.num_entries),
            children: Vec::new(),
        };
        return Ok((tree_oid, ext_tree));
    }

    let mut child_results: Vec<(ObjectId, extension::Tree)> = Vec::with_capacity(node.children.len());

    for child in &node.children {
        let result = write_trees_recursive(child, write)?;
        child_results.push(result);
    }

    // Build the gix_object::Tree for this node.
    let mut tree = gix_object::Tree {
        entries: Vec::with_capacity(node.entries.len() + child_results.len()),
    };

    // Add blob/symlink/submodule entries.
    for (name, mode, oid) in &node.entries {
        tree.entries.push(gix_object::tree::Entry {
            filename: bstr::BString::from(name.as_slice()),
            mode: *mode,
            oid: *oid,
        });
    }

    // Add sub-tree entries.
    for (child_oid, child_ext) in &child_results {
        tree.entries.push(gix_object::tree::Entry {
            filename: bstr::BString::from(child_ext.name.as_slice()),
            mode: EntryKind::Tree.into(),
            oid: *child_oid,
        });
    }

    // Sort entries as git requires (trees sort as if they have a trailing '/').
    tree.entries.sort();

    // Write the tree object.
    let tree_id = write(&tree).map_err(|err| Error::WriteTree(Box::new(err)))?;

    // Build the extension::Tree node.
    let ext_tree = extension::Tree {
        name: node.name.clone(),
        id: tree_id,
        num_entries: Some(node.num_entries),
        children: child_results.into_iter().map(|(_, ext)| ext).collect(),
    };

    Ok((tree_id, ext_tree))
}
