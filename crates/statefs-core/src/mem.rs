//! Reference In-Memory Prefix Trie implementation of `Store`.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use crate::error::StoreError;
use crate::node::Node;
use crate::path::Path;
use crate::store::Store;

#[derive(Default, Clone, Debug)]
struct TrieNode {
    node: Option<Node>,
    children: BTreeMap<String, TrieNode>,
}

impl TrieNode {
    fn is_empty(&self) -> bool {
        self.node.is_none() && self.children.is_empty()
    }
}

/// A zero-dependency, in-memory hierarchical Trie store.
#[derive(Default, Clone, Debug)]
pub struct MemStore {
    root: TrieNode,
    global_revision: u64,
}

impl MemStore {
    /// Constructs a new empty in-memory store.
    pub fn new() -> Self {
        Self {
            root: TrieNode::default(),
            global_revision: 0,
        }
    }

    /// Returns the global revision counter.
    pub fn global_revision(&self) -> u64 {
        self.global_revision
    }

    /// Clears all nodes from the store.
    pub fn clear(&mut self) {
        self.root = TrieNode::default();
        self.global_revision = self.global_revision.saturating_add(1);
    }
}

impl Store for MemStore {
    fn get(&self, path: &Path) -> Option<&Node> {
        let mut cur = &self.root;
        for seg in path.segments() {
            cur = cur.children.get(seg)?;
        }
        cur.node.as_ref()
    }

    fn insert_node(&mut self, path: &Path, mut node: Node) -> Result<Option<Node>, StoreError> {
        let mut cur = &mut self.root;
        for seg in path.segments() {
            cur = cur.children.entry(seg.clone()).or_default();
        }

        if let Some(existing) = &cur.node {
            if existing.readonly {
                return Err(StoreError::ReadOnly(path.clone()));
            }
            node.revision = existing.revision.saturating_add(1);
        }

        self.global_revision = self.global_revision.saturating_add(1);
        let old = cur.node.replace(node);
        Ok(old)
    }

    fn remove(&mut self, path: &Path) -> Result<Option<Node>, StoreError> {
        fn remove_rec(cur: &mut TrieNode, segments: &[String], idx: usize) -> Result<Option<Node>, StoreError> {
            if idx == segments.len() {
                if let Some(existing) = &cur.node {
                    if existing.readonly {
                        return Err(StoreError::ReadOnly(Path::from_segments(segments.to_vec())));
                    }
                }
                return Ok(cur.node.take());
            }

            let seg = &segments[idx];
            let Some(child) = cur.children.get_mut(seg) else {
                return Ok(None);
            };

            let res = remove_rec(child, segments, idx + 1)?;

            if child.is_empty() {
                cur.children.remove(seg);
            }

            Ok(res)
        }

        let old = remove_rec(&mut self.root, path.segments(), 0)?;
        if old.is_some() {
            self.global_revision = self.global_revision.saturating_add(1);
        }
        Ok(old)
    }

    fn list_children(&self, prefix: &Path) -> Vec<Path> {
        let mut cur = &self.root;
        for seg in prefix.segments() {
            let Some(next) = cur.children.get(seg) else {
                return Vec::new();
            };
            cur = next;
        }

        cur.children
            .iter()
            .filter(|(_, child)| child.node.as_ref().map_or(true, |n| !n.hidden))
            .map(|(k, _)| prefix.join(k))
            .collect()
    }

    fn list_subpaths(&self, prefix: &Path) -> Vec<Path> {
        let mut results = Vec::new();
        let mut cur = &self.root;

        for seg in prefix.segments() {
            let Some(next) = cur.children.get(seg) else {
                return results;
            };
            cur = next;
        }

        fn collect_rec(cur: &TrieNode, cur_path: &Path, results: &mut Vec<Path>) {
            if let Some(node) = &cur.node {
                if !node.hidden {
                    results.push(cur_path.clone());
                }
            }
            for (seg, child) in &cur.children {
                collect_rec(child, &cur_path.join(seg), results);
            }
        }

        collect_rec(cur, prefix, &mut results);
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    #[test]
    fn test_memstore_insert_get_remove() {
        let mut store = MemStore::new();
        let path = Path::parse("/plugins/moderation/cvars/ban_time");

        assert!(!store.contains(&path));
        assert!(store.get(&path).is_none());

        let prev = store.insert(&path, Value::Int(60)).unwrap();
        assert!(prev.is_none());

        let node = store.get(&path).unwrap();
        assert_eq!(node.value, Value::Int(60));
        assert_eq!(node.revision, 1);

        // Update
        let prev = store.insert(&path, Value::Int(120)).unwrap();
        assert_eq!(prev.unwrap().value, Value::Int(60));
        assert_eq!(store.get(&path).unwrap().revision, 2);

        // Remove
        let removed = store.remove(&path).unwrap().unwrap();
        assert_eq!(removed.value, Value::Int(120));
        assert!(!store.contains(&path));
    }

    #[test]
    fn test_memstore_read_only_protection() {
        let mut store = MemStore::new();
        let path = Path::parse("/system/kernel/version");

        store.insert_node(&path, Node::read_only(Value::from("1.0.0"))).unwrap();

        let err = store.insert(&path, Value::from("2.0.0")).unwrap_err();
        assert_eq!(err, StoreError::ReadOnly(path.clone()));

        let err_rm = store.remove(&path).unwrap_err();
        assert_eq!(err_rm, StoreError::ReadOnly(path.clone()));
    }

    #[test]
    fn test_memstore_hierarchy_listing() {
        let mut store = MemStore::new();
        store.insert(&Path::parse("/a/b/c"), Value::from(1)).unwrap();
        store.insert(&Path::parse("/a/b/d"), Value::from(2)).unwrap();
        store.insert(&Path::parse("/a/e"), Value::from(3)).unwrap();

        let children_a = store.list_children(&Path::parse("/a"));
        assert_eq!(children_a.len(), 2);
        assert_eq!(children_a[0].to_string(), "/a/b");
        assert_eq!(children_a[1].to_string(), "/a/e");

        let all_under_a = store.list_subpaths(&Path::parse("/a"));
        assert_eq!(all_under_a.len(), 3);
        assert_eq!(all_under_a[0].to_string(), "/a/b/c");
        assert_eq!(all_under_a[1].to_string(), "/a/b/d");
        assert_eq!(all_under_a[2].to_string(), "/a/e");
    }
}
