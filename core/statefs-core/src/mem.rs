//! High-Performance Arena-based Prefix Trie implementation of `Store`.
//!
//! Stores the entire state hierarchy inside a single flat `Vec<ArenaNode>` (Arena layout).
//! All segment strings are interned inside a shared deduplicated `StringPool`.
//!
//! ### Scrooge Invariants:
//! - **0 Heap Allocations per Lookup**: Traversal uses 32-bit array indices (`u32`).
//! - **Maximum Cache Locality**: Sequential cache-line prefetching friendly.
//! - **String Deduplication**: Common segments (`"plugins"`, `"moderation"`) are stored once.
//! - **Unchanged DX**: Implements [`Store`], providing identical `get`, `insert`, `remove` semantics.

use crate::error::StoreError;
use crate::node::Node;
use crate::path::Path;
use crate::store::Store;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// Sentinel index representing null / none in the 32-bit arena.
const NULL_IDX: u32 = u32::MAX;

/// An interned string identifier within the [`StringPool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Symbol(u32);

/// Deduplicating string pool to eliminate redundant segment allocations.
#[derive(Default, Clone, Debug)]
struct StringPool {
    buffer: String,
    spans: Vec<(u32, u32)>, // (offset, len)
    lookup: BTreeMap<String, Symbol>,
}

impl StringPool {
    fn new() -> Self {
        Self::default()
    }

    fn intern(&mut self, text: &str) -> Symbol {
        if let Some(&sym) = self.lookup.get(text) {
            return sym;
        }

        let offset = self.buffer.len() as u32;
        let len = text.len() as u32;
        self.buffer.push_str(text);

        let id = Symbol(self.spans.len() as u32);
        self.spans.push((offset, len));
        self.lookup.insert(text.into(), id);
        id
    }

    #[inline]
    fn resolve(&self, sym: Symbol) -> &str {
        let (offset, len) = self.spans[sym.0 as usize];
        let start = offset as usize;
        let end = start + len as usize;
        &self.buffer[start..end]
    }
}

/// A compact, cache-friendly node in the flat arena.
///
/// Uses the classic Left-Child / Right-Sibling binary representation of an N-ary tree:
/// - `first_child`: points to the first child node.
/// - `next_sibling`: points to the next sibling sharing the same parent.
#[derive(Clone, Debug)]
struct ArenaNode {
    symbol: Symbol,
    node: Option<Node>,
    first_child: u32,
    next_sibling: u32,
}

impl ArenaNode {
    fn new(symbol: Symbol) -> Self {
        Self {
            symbol,
            node: None,
            first_child: NULL_IDX,
            next_sibling: NULL_IDX,
        }
    }
}

/// A zero-dependency, cache-coherent hierarchical Arena Trie store.
#[derive(Clone, Debug)]
pub struct MemStore {
    arena: Vec<ArenaNode>,
    pool: StringPool,
    global_revision: u64,
}

impl Default for MemStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemStore {
    /// Constructs a new empty arena-backed store.
    pub fn new() -> Self {
        let mut pool = StringPool::new();
        let root_symbol = pool.intern("");
        let mut arena = Vec::with_capacity(32);
        arena.push(ArenaNode::new(root_symbol)); // index 0 is root

        Self {
            arena,
            pool,
            global_revision: 0,
        }
    }

    /// Returns the global revision counter.
    #[inline]
    pub fn global_revision(&self) -> u64 {
        self.global_revision
    }

    /// Clears all nodes from the store.
    pub fn clear(&mut self) {
        self.arena.clear();
        self.pool = StringPool::new();
        let root_symbol = self.pool.intern("");
        self.arena.push(ArenaNode::new(root_symbol));
        self.global_revision = self.global_revision.saturating_add(1);
    }

    /// Zero-allocation lookup by raw string path with `/` or `\` separators.
    pub fn get_str(&self, raw_path: &str) -> Option<&Node> {
        self.find_node_id(raw_path)
            .and_then(|id| self.get_by_id(id))
    }

    /// Resolves the raw path string to an internal 32-bit arena node ID.
    pub fn find_node_id(&self, raw_path: &str) -> Option<u32> {
        let trimmed = raw_path.trim();
        let mut cur = 0u32;

        #[cfg(feature = "simd")]
        {
            let bytes = trimmed.as_bytes();
            let mut start = 0;
            for pos in memchr::memchr2_iter(b'/', b'\\', bytes) {
                if pos > start {
                    // SAFETY: valid utf-8 slice of verified str
                    let seg = unsafe { core::str::from_utf8_unchecked(&bytes[start..pos]) }.trim();
                    if !seg.is_empty() {
                        cur = self.find_child(cur, seg)?;
                    }
                }
                start = pos + 1;
            }

            if start < bytes.len() {
                let seg = unsafe { core::str::from_utf8_unchecked(&bytes[start..]) }.trim();
                if !seg.is_empty() {
                    cur = self.find_child(cur, seg)?;
                }
            }
        }

        #[cfg(not(feature = "simd"))]
        {
            for seg in trimmed
                .split(['/', '\\'])
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                cur = self.find_child(cur, seg)?;
            }
        }

        if self.arena[cur as usize].node.is_some() {
            Some(cur)
        } else {
            None
        }
    }

    /// Direct O(1) lookup of a node by its 32-bit arena index.
    #[inline(always)]
    pub fn get_by_id(&self, node_id: u32) -> Option<&Node> {
        self.arena
            .get(node_id as usize)
            .and_then(|an| an.node.as_ref())
    }

    fn find_child(&self, parent_idx: u32, segment: &str) -> Option<u32> {
        let mut cur = self.arena[parent_idx as usize].first_child;
        while cur != NULL_IDX {
            let node = &self.arena[cur as usize];
            if self.pool.resolve(node.symbol) == segment {
                return Some(cur);
            }
            cur = node.next_sibling;
        }
        None
    }
}

impl Store for MemStore {
    fn get_by_segments<'a, I>(&self, segments: I) -> Option<&Node>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut cur = 0u32; // Root is always index 0
        for seg in segments {
            cur = self.find_child(cur, seg)?;
        }
        self.arena[cur as usize].node.as_ref()
    }

    fn insert_node(&mut self, path: &Path, mut node: Node) -> Result<Option<Node>, StoreError> {
        let mut cur = 0u32;

        for seg in path.segments() {
            if let Some(child_idx) = self.find_child(cur, seg) {
                cur = child_idx;
            } else {
                // Allocate symbol & node in arena
                let sym = self.pool.intern(seg);
                let new_idx = self.arena.len() as u32;
                let mut new_node = ArenaNode::new(sym);

                // Insert into sibling linked-list
                let old_first = self.arena[cur as usize].first_child;
                new_node.next_sibling = old_first;
                self.arena.push(new_node);
                self.arena[cur as usize].first_child = new_idx;

                cur = new_idx;
            }
        }

        let target = &mut self.arena[cur as usize];
        if let Some(existing) = &target.node {
            if existing.readonly {
                return Err(StoreError::ReadOnly(path.clone()));
            }
            node.revision = existing.revision.saturating_add(1);
        }

        self.global_revision = self.global_revision.saturating_add(1);
        let old = target.node.replace(node);
        Ok(old)
    }

    fn remove(&mut self, path: &Path) -> Result<Option<Node>, StoreError> {
        let mut cur = 0u32;
        for seg in path.segments() {
            let Some(next) = self.find_child(cur, seg) else {
                return Ok(None);
            };
            cur = next;
        }

        let target = &mut self.arena[cur as usize];
        if let Some(existing) = &target.node
            && existing.readonly
        {
            return Err(StoreError::ReadOnly(path.clone()));
        }

        let old = target.node.take();
        if old.is_some() {
            self.global_revision = self.global_revision.saturating_add(1);
        }
        Ok(old)
    }

    fn list_children(&self, prefix: &Path) -> Vec<Path> {
        let mut cur = 0u32;
        for seg in prefix.segments() {
            let Some(next) = self.find_child(cur, seg) else {
                return Vec::new();
            };
            cur = next;
        }

        let mut child_paths = Vec::new();
        let mut child_idx = self.arena[cur as usize].first_child;

        while child_idx != NULL_IDX {
            let child = &self.arena[child_idx as usize];
            let is_visible = child.node.as_ref().is_none_or(|n| !n.hidden);

            if is_visible {
                let name = self.pool.resolve(child.symbol);
                child_paths.push(prefix.join(name));
            }
            child_idx = child.next_sibling;
        }

        child_paths.sort();
        child_paths
    }

    fn list_subpaths(&self, prefix: &Path) -> Vec<Path> {
        let mut cur = 0u32;
        for seg in prefix.segments() {
            let Some(next) = self.find_child(cur, seg) else {
                return Vec::new();
            };
            cur = next;
        }

        fn collect_rec(
            arena: &[ArenaNode],
            pool: &StringPool,
            cur_idx: u32,
            cur_path: &Path,
            results: &mut Vec<Path>,
        ) {
            let node = &arena[cur_idx as usize];
            if let Some(val) = &node.node
                && !val.hidden
                && !cur_path.is_root()
            {
                results.push(cur_path.clone());
            }

            let mut child_idx = node.first_child;
            while child_idx != NULL_IDX {
                let child = &arena[child_idx as usize];
                let name = pool.resolve(child.symbol);
                let next_path = cur_path.join(name);
                collect_rec(arena, pool, child_idx, &next_path, results);
                child_idx = child.next_sibling;
            }
        }

        let mut results = Vec::new();
        collect_rec(&self.arena, &self.pool, cur, prefix, &mut results);
        results.sort();
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

        store
            .insert_node(&path, Node::read_only(Value::from("1.0.0")))
            .unwrap();

        let err = store.insert(&path, Value::from("2.0.0")).unwrap_err();
        assert_eq!(err, StoreError::ReadOnly(path.clone()));

        let err_rm = store.remove(&path).unwrap_err();
        assert_eq!(err_rm, StoreError::ReadOnly(path.clone()));
    }

    #[test]
    fn test_memstore_hierarchy_listing() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/a/b/c"), Value::from(1))
            .unwrap();
        store
            .insert(&Path::parse("/a/b/d"), Value::from(2))
            .unwrap();
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
