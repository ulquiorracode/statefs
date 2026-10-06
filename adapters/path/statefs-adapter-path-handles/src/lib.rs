//! # StateFS Path Handle Resolution Adapter
//!
//! Provides O(1) path handle caching and lookup, transforming string paths
//! into instant 32-bit arena handles ([`PathHandle`]) to eliminate parsing in hot loops.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use statefs_core::{MemStore, Node, PathHandle};

/// A cache slot storing the path hash, corresponding handle, exact path bytes, and generation epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandleSlot {
    pub hash: u64,
    pub handle: PathHandle,
    pub epoch: u64,
    pub path_len: u16,
    pub path_buf: [u8; 64],
}

impl Default for HandleSlot {
    fn default() -> Self {
        Self {
            hash: 0,
            handle: PathHandle(u32::MAX),
            epoch: 0,
            path_len: 0,
            path_buf: [0u8; 64],
        }
    }
}

/// A fixed-capacity direct-mapped cache from path strings to [`PathHandle`] with epoch validation.
pub struct PathHandleCache<const CAP: usize = 64> {
    slots: [HandleSlot; CAP],
}

impl<const CAP: usize> Default for PathHandleCache<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> PathHandleCache<CAP> {
    /// Creates a new uninitialized handle cache.
    pub const fn new() -> Self {
        assert!(
            CAP > 0 && (CAP & (CAP - 1)) == 0,
            "PathHandleCache CAP must be a power of two"
        );
        Self {
            slots: [HandleSlot {
                hash: 0,
                handle: PathHandle(u32::MAX),
                epoch: 0,
                path_len: 0,
                path_buf: [0u8; 64],
            }; CAP],
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

    /// Gets a cached [`PathHandle`] verifying exact path match.
    #[inline(always)]
    pub fn get(&self, path: &str) -> Option<PathHandle> {
        let h = Self::hash(path);
        let idx = (h as usize) & (CAP - 1);
        let slot = &self.slots[idx];
        if slot.handle.0 != u32::MAX
            && slot.hash == h
            && slot.path_len as usize == path.len()
            && &slot.path_buf[..slot.path_len as usize] == path.as_bytes()
        {
            Some(slot.handle)
        } else {
            None
        }
    }

    /// Gets a cached [`PathHandle`] ensuring it matches the current `epoch` / store revision and path.
    #[inline(always)]
    pub fn get_with_epoch(&self, path: &str, current_epoch: u64) -> Option<PathHandle> {
        let h = Self::hash(path);
        let idx = (h as usize) & (CAP - 1);
        let slot = &self.slots[idx];
        if slot.handle.0 != u32::MAX
            && slot.hash == h
            && slot.epoch == current_epoch
            && slot.path_len as usize == path.len()
            && &slot.path_buf[..slot.path_len as usize] == path.as_bytes()
        {
            Some(slot.handle)
        } else {
            None
        }
    }

    /// Inserts a newly resolved [`PathHandle`] into the cache with an epoch, verifying bounds.
    #[inline(always)]
    pub fn put_with_epoch(&mut self, path: &str, handle: PathHandle, epoch: u64) {
        let path_len = path.len();
        if path_len <= 64 {
            let h = Self::hash(path);
            let idx = (h as usize) & (CAP - 1);
            let mut path_buf = [0u8; 64];
            path_buf[..path_len].copy_from_slice(path.as_bytes());
            self.slots[idx] = HandleSlot {
                hash: h,
                handle,
                epoch,
                path_len: path_len as u16,
                path_buf,
            };
        }
    }

    /// Inserts a newly resolved [`PathHandle`] into the cache.
    #[inline(always)]
    pub fn put(&mut self, path: &str, handle: PathHandle) {
        self.put_with_epoch(path, handle, 0);
    }

    /// Resolves a path to a [`PathHandle`], verifying current store epoch and node presence.
    #[inline]
    pub fn resolve_or_lookup(&mut self, store: &MemStore, path: &str) -> Option<PathHandle> {
        let current_rev = store.global_revision();
        if let Some(h) = self.get_with_epoch(path, current_rev)
            && store.get_by_handle(h).is_some()
        {
            return Some(h);
        }
        if let Some(h) = store.resolve_handle(path) {
            self.put_with_epoch(path, h, current_rev);
            Some(h)
        } else {
            None
        }
    }

    /// Fetches the node directly from store via cached handle with epoch verification.
    #[inline(always)]
    pub fn get_node<'a>(&mut self, store: &'a MemStore, path: &str) -> Option<&'a Node> {
        let handle = self.resolve_or_lookup(store, path)?;
        store.get_by_handle(handle)
    }

    /// Invalidates all entries in the handle cache.
    #[inline(always)]
    pub fn invalidate(&mut self) {
        for slot in &mut self.slots {
            slot.handle = PathHandle(u32::MAX);
        }
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

    #[test]
    fn test_handle_cache_invalidation_on_mutation() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/server/tickrate"), Value::from(128))
            .unwrap();

        let mut cache = PathHandleCache::<32>::new();
        let _ = cache.resolve_or_lookup(&store, "/server/tickrate").unwrap();

        // Remove node from store -> global_revision increments
        store.remove(&Path::parse("/server/tickrate")).unwrap();

        // Lookup with epoch check must miss and return None safely, without stale read or panic
        let resolved = cache.resolve_or_lookup(&store, "/server/tickrate");
        assert_eq!(resolved, None);
        assert!(cache.get_node(&store, "/server/tickrate").is_none());
    }
}
