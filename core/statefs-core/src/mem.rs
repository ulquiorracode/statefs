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
pub(crate) struct Symbol(pub(crate) u32);

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
pub(crate) struct ArenaNode {
    pub(crate) symbol: Symbol,
    pub(crate) node: Option<Node>,
    pub(crate) first_child: u32,
    pub(crate) next_sibling: u32,
    pub(crate) subtree_revision: u64,
}

impl ArenaNode {
    fn new(symbol: Symbol) -> Self {
        Self {
            symbol,
            node: None,
            first_child: NULL_IDX,
            next_sibling: NULL_IDX,
            subtree_revision: 0,
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

    /// Returns the monotonic revision of the subtree rooted at `path`.
    pub fn subtree_revision(&self, path: &Path) -> Option<u64> {
        let mut cur = 0u32;
        for seg in path.segments() {
            cur = self.find_child(cur, seg)?;
        }
        Some(self.arena[cur as usize].subtree_revision)
    }

    /// Returns the monotonic revision of the subtree rooted at `raw_path`.
    pub fn subtree_revision_str(&self, raw_path: &str) -> Option<u64> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() || trimmed == "/" {
            return Some(self.global_revision);
        }
        let node_id = self.find_arena_index(trimmed)?;
        Some(self.arena[node_id as usize].subtree_revision)
    }

    /// Clears all nodes from the store.
    pub fn clear(&mut self) {
        self.arena.clear();
        self.pool = StringPool::new();
        let root_symbol = self.pool.intern("");
        self.arena.push(ArenaNode::new(root_symbol));
        self.global_revision = self.global_revision.saturating_add(1);
        self.arena[0].subtree_revision = self.global_revision;
    }

    /// Zero-allocation lookup by raw string path with `/` or `\` separators.
    pub fn get_str(&self, raw_path: &str) -> Option<&Node> {
        self.find_node_id(raw_path)
            .and_then(|id| self.get_by_id(id))
    }

    /// Resolves a path string to a permanent O(1) [`PathHandle`].
    #[inline]
    pub fn resolve_handle(&self, raw_path: &str) -> Option<crate::path::PathHandle> {
        self.find_node_id(raw_path).map(crate::path::PathHandle)
    }

    /// Fetches a node directly by its [`PathHandle`] in true O(1) (2-3 ns) without path parsing.
    #[inline(always)]
    pub fn get_by_handle(&self, handle: crate::path::PathHandle) -> Option<&Node> {
        self.get_by_id(handle.0)
    }

    /// Resolves the raw path string to an internal 32-bit arena node ID, if it contains a value.
    pub fn find_node_id(&self, raw_path: &str) -> Option<u32> {
        let idx = self.find_arena_index(raw_path)?;
        if self.arena[idx as usize].node.is_some() {
            Some(idx)
        } else {
            None
        }
    }

    /// Resolves any valid path (intermediate directory or leaf) to its 32-bit arena index.
    pub fn find_arena_index(&self, raw_path: &str) -> Option<u32> {
        let trimmed = raw_path.trim_ascii();
        if trimmed.is_empty() || trimmed == "/" {
            return Some(0);
        }
        let mut cur = 0u32;

        #[cfg(feature = "simd")]
        {
            let bytes = trimmed.as_bytes();
            let mut start = 0;
            for pos in memchr::memchr2_iter(b'/', b'\\', bytes) {
                if pos > start {
                    // SAFETY: valid utf-8 slice of verified str
                    let seg =
                        unsafe { core::str::from_utf8_unchecked(&bytes[start..pos]) }.trim_ascii();
                    if !seg.is_empty() {
                        cur = self.find_child(cur, seg)?;
                    }
                }
                start = pos + 1;
            }

            if start < bytes.len() {
                let seg = unsafe { core::str::from_utf8_unchecked(&bytes[start..]) }.trim_ascii();
                if !seg.is_empty() {
                    cur = self.find_child(cur, seg)?;
                }
            }
        }

        #[cfg(not(feature = "simd"))]
        {
            for seg in trimmed
                .split(['/', '\\'])
                .map(|s| s.trim_ascii())
                .filter(|s| !s.is_empty())
            {
                cur = self.find_child(cur, seg)?;
            }
        }

        Some(cur)
    }

    /// Queries the tree with a pattern supporting `*` and `**` wildcards.
    pub fn find_glob(&self, pattern: &str) -> Vec<(Path, &Node)> {
        crate::glob::match_glob(self, pattern)
    }

    /// Direct O(1) lookup of a node by its 32-bit arena index.
    #[inline(always)]
    pub fn get_by_id(&self, node_id: u32) -> Option<&Node> {
        self.arena
            .get(node_id as usize)
            .and_then(|an| an.node.as_ref())
    }

    pub(crate) fn node_symbol_str(&self, idx: u32) -> &str {
        self.pool.resolve(self.arena[idx as usize].symbol)
    }

    pub(crate) fn node_first_child(&self, idx: u32) -> u32 {
        self.arena[idx as usize].first_child
    }

    pub(crate) fn node_next_sibling(&self, idx: u32) -> u32 {
        self.arena[idx as usize].next_sibling
    }

    pub(crate) fn node_value(&self, idx: u32) -> Option<&Node> {
        self.arena[idx as usize].node.as_ref()
    }

    /// Total number of arena nodes in memory.
    #[inline]
    pub fn arena_len(&self) -> usize {
        self.arena.len()
    }

    /// Access raw interned string buffer bytes.
    #[inline]
    pub fn string_pool_bytes(&self) -> &[u8] {
        self.pool.buffer.as_bytes()
    }

    /// Exports all arena nodes as stable [`RawNode`] structs along with interned string bytes.
    pub fn export_raw_nodes(&self) -> (Vec<crate::backing::RawNode>, &[u8]) {
        let mut raw_nodes = Vec::with_capacity(self.arena.len());
        for (i, arena_node) in self.arena.iter().enumerate() {
            let (sym_offset, sym_len) = self
                .pool
                .spans
                .get(arena_node.symbol.0 as usize)
                .copied()
                .unwrap_or((0, 0));
            let revision = arena_node
                .node
                .as_ref()
                .map(|n| n.revision)
                .unwrap_or(arena_node.subtree_revision);

            let mut flags = 0u32;
            if let Some(n) = &arena_node.node {
                if n.readonly {
                    flags |= 1;
                }
                if n.hidden {
                    flags |= 2;
                }
            }

            raw_nodes.push(crate::backing::RawNode {
                symbol_offset: sym_offset,
                symbol_len: sym_len,
                first_child: arena_node.first_child,
                next_sibling: arena_node.next_sibling,
                revision,
                flags,
            });
            let _ = i;
        }

        (raw_nodes, self.pool.buffer.as_bytes())
    }

    fn find_child(&self, parent_idx: u32, segment: &str) -> Option<u32> {
        // Fast-path: if the segment was never interned, it cannot exist in any child
        let sym_opt = self.pool.lookup.get(segment).copied();

        let mut cur = self.arena[parent_idx as usize].first_child;
        if let Some(target_sym) = sym_opt {
            // O(1) integer comparison for interned symbols
            while cur != NULL_IDX {
                let node = &self.arena[cur as usize];
                if node.symbol == target_sym {
                    return Some(cur);
                }
                cur = node.next_sibling;
            }
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
        let mut path_indices = Vec::with_capacity(8);
        path_indices.push(0);

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
            path_indices.push(cur);
        }

        if let Some(existing) = &self.arena[cur as usize].node {
            if existing.readonly {
                return Err(StoreError::ReadOnly(path.clone()));
            }
            node.revision = existing.revision.saturating_add(1);
        }

        self.global_revision = self.global_revision.saturating_add(1);
        for &idx in &path_indices {
            self.arena[idx as usize].subtree_revision = self.global_revision;
        }

        let old = self.arena[cur as usize].node.replace(node);
        Ok(old)
    }

    fn remove(&mut self, path: &Path) -> Result<Option<Node>, StoreError> {
        let mut cur = 0u32;
        let mut path_indices = Vec::with_capacity(8);
        path_indices.push(0);

        for seg in path.segments() {
            let Some(next) = self.find_child(cur, seg) else {
                return Ok(None);
            };
            cur = next;
            path_indices.push(cur);
        }

        if let Some(existing) = &self.arena[cur as usize].node
            && existing.readonly
        {
            return Err(StoreError::ReadOnly(path.clone()));
        }

        let old = self.arena[cur as usize].node.take();
        if old.is_some() {
            self.global_revision = self.global_revision.saturating_add(1);
            for &idx in &path_indices {
                self.arena[idx as usize].subtree_revision = self.global_revision;
            }
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
            // An entry is visible if it has a non-hidden node OR it is a non-empty directory branch
            let has_active_node = child.node.as_ref().is_some_and(|n| !n.hidden);
            let has_children = child.first_child != NULL_IDX;

            if has_active_node || has_children {
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
