//! # StateFS Path Handle Resolution Adapter
//!
//! Provides O(1) path handle caching and lookup, transforming string paths
//! into instant 32-bit arena handles ([`PathHandle`]) to eliminate parsing in hot loops.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use statefs_core::{MemStore, Node, PathHandle};

/// A fixed-capacity direct-mapped cache from path strings to [`PathHandle`].
pub struct PathHandleCache<const CAP: usize = 64> {
    slots: [(u64, PathHandle); CAP],
}

impl<const CAP: usize> Default for PathHandleCache<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> PathHandleCache<CAP> {
    /// Creates a new uninitialized handle cache.
    pub const fn new() -> Self {
        Self {
            slots: [(0, PathHandle(u32::MAX)); CAP],
        }
    }

    #[inline(always)]
    fn hash(path: &str) -> u64 {
        // Fast DJB2 hash
        let mut h: u64 = 5381;
        for &b in path.as_bytes() {
            h = ((h << 5).wrapping_add(h)).wrapping_add(b as u64);
        }
        h
    }

    /// Gets a cached [`PathHandle`] if available.
    #[inline(always)]
    pub fn get(&self, path: &str) -> Option<PathHandle> {
        let h = Self::hash(path);
        let idx = (h as usize) % CAP;
        let (stored_hash, handle) = self.slots[idx];
        if stored_hash == h && handle.0 != u32::MAX {
            Some(handle)
        } else {
            None
        }
    }

    /// Inserts a newly resolved [`PathHandle`] into the cache.
    #[inline(always)]
    pub fn put(&mut self, path: &str, handle: PathHandle) {
        let h = Self::hash(path);
        let idx = (h as usize) % CAP;
        self.slots[idx] = (h, handle);
    }

    /// Resolves a path to a [`PathHandle`], populating the cache on miss.
    #[inline]
    pub fn resolve_or_lookup(&mut self, store: &MemStore, path: &str) -> Option<PathHandle> {
        if let Some(h) = self.get(path) {
            return Some(h);
        }
        if let Some(h) = store.resolve_handle(path) {
            self.put(path, h);
            Some(h)
        } else {
            None
        }
    }

    /// Fetches the node directly from store via cached handle.
    #[inline(always)]
    pub fn get_node<'a>(&mut self, store: &'a MemStore, path: &str) -> Option<&'a Node> {
        let handle = self.resolve_or_lookup(store, path)?;
        store.get_by_handle(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use statefs_core::{Path, Store, Value};

    #[test]
    fn test_handle_cache_resolution() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/server/tickrate"), Value::from(128))
            .unwrap();

        let mut cache = PathHandleCache::<32>::new();

        // 1. Initial resolution (miss -> lookup -> cache)
        let handle = cache.resolve_or_lookup(&store, "/server/tickrate").unwrap();
        assert_ne!(handle.0, u32::MAX);

        // 2. Second lookup (instant cache hit)
        let cached_handle = cache.get("/server/tickrate").unwrap();
        assert_eq!(cached_handle, handle);

        // 3. Direct O(1) node access
        let node = cache.get_node(&store, "/server/tickrate").unwrap();
        assert_eq!(node.value, Value::from(128));
    }
}
