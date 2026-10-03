//! Fundamental Store trait contract for StateFS nanokernel.

use crate::error::StoreError;
use crate::node::Node;
use crate::path::Path;
use crate::value::Value;
use alloc::vec::Vec;

/// Primitive storage contract for hierarchical state trees.
pub trait Store {
    /// Retrieves a reference to the node at the specified path, if present.
    fn get(&self, path: &Path) -> Option<&Node>;

    /// Inserts a new value at the specified path.
    ///
    /// Returns the previous node if one existed.
    fn insert(&mut self, path: &Path, value: Value) -> Result<Option<Node>, StoreError> {
        self.insert_node(path, Node::new(value))
    }

    /// Inserts a fully configured `Node` at the specified path.
    fn insert_node(&mut self, path: &Path, node: Node) -> Result<Option<Node>, StoreError>;

    /// Removes the node at the specified path.
    ///
    /// Returns the removed node if it existed.
    fn remove(&mut self, path: &Path) -> Result<Option<Node>, StoreError>;

    /// Returns `true` if a node exists at the specified path.
    fn contains(&self, path: &Path) -> bool {
        self.get(path).is_some()
    }

    /// Lists immediate child paths directly beneath `prefix`.
    fn list_children(&self, prefix: &Path) -> Vec<Path>;

    /// Recursively lists all node paths located under `prefix`.
    fn list_subpaths(&self, prefix: &Path) -> Vec<Path>;
}
