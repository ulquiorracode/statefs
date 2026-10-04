//! # L1 Direct-Mapped Path Cache Adapter
//!
//! A zero-allocation, fixed-capacity L1 cache designed for hot game-loop queries.
//!
//! - **Capacity via `const generic`**: Allocated inline in the structure without heap indirection.
//! - **Masked Hash Indexing**: For power-of-two capacities, lookup is a single bitwise `AND`.
//! - **Sub-10ns Lookup**: Bypasses tree traversal completely on cache hits.

#![cfg_attr(not(feature = "std"), no_std)]

/// Default capacity for the L1 path cache (64 entries).
pub const DEFAULT_CACHE_CAP: usize = 64;

/// Ultra-fast 64-bit FNV-1a hash with 8-byte chunking.
#[inline(always)]
pub fn fast_path_hash(path: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let bytes = path.as_bytes();
    let mut hash = FNV_OFFSET ^ (bytes.len() as u64);

    let (chunks, remainder) = bytes.as_chunks::<8>();
    for chunk in chunks {
        let word = u64::from_ne_bytes(*chunk);
        hash ^= word;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    for &rem in remainder {
        hash ^= rem as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    hash
}

/// A cache entry storing the path hash, corresponding node ID, and generation epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheSlot {
    pub hash: u64,
    pub node_id: u32,
    pub epoch: u64,
    pub valid: bool,
}

/// L1 Direct-Mapped Path Cache with compile-time inline capacity.
pub struct L1PathCache<const CAP: usize = DEFAULT_CACHE_CAP> {
    slots: [CacheSlot; CAP],
}

impl<const CAP: usize> Default for L1PathCache<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> L1PathCache<CAP> {
    /// Creates a new empty L1 cache with compile-time bounded capacity.
    pub const fn new() -> Self {
        assert!(
            CAP > 0 && (CAP & (CAP - 1)) == 0,
            "L1PathCache CAP must be a power of two"
        );
        Self {
            slots: [CacheSlot {
                hash: 0,
                node_id: 0,
                epoch: 0,
                valid: false,
            }; CAP],
        }
    }

    /// Looks up a cached `node_id` by raw path string without epoch check.
    #[inline(always)]
    pub fn get(&self, path: &str) -> Option<u32> {
        let hash = fast_path_hash(path);
        let idx = (hash as usize) & (CAP - 1);
        let slot = &self.slots[idx];

        if slot.valid && slot.hash == hash {
            Some(slot.node_id)
        } else {
            None
        }
    }

    /// Looks up a cached `node_id` verifying current store epoch / global revision.
    #[inline(always)]
    pub fn get_with_epoch(&self, path: &str, current_epoch: u64) -> Option<u32> {
        let hash = fast_path_hash(path);
        let idx = (hash as usize) & (CAP - 1);
        let slot = &self.slots[idx];

        if slot.valid && slot.hash == hash && slot.epoch == current_epoch {
            Some(slot.node_id)
        } else {
            None
        }
    }

    /// Stores a resolved `node_id` into the direct-mapped slot with current store epoch.
    #[inline(always)]
    pub fn put(&mut self, path: &str, node_id: u32) {
        self.put_with_epoch(path, node_id, 0);
    }

    /// Stores a resolved `node_id` along with store epoch.
    #[inline(always)]
    pub fn put_with_epoch(&mut self, path: &str, node_id: u32, epoch: u64) {
        let hash = fast_path_hash(path);
        let idx = (hash as usize) & (CAP - 1);
        self.slots[idx] = CacheSlot {
            hash,
            node_id,
            epoch,
            valid: true,
        };
    }

    /// Invalidates all entries in the cache (e.g. after a tree mutation).
    #[inline(always)]
    pub fn invalidate(&mut self) {
        for slot in &mut self.slots {
            slot.valid = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_put_get_invalidate() {
        let mut cache = L1PathCache::<32>::new();
        assert_eq!(cache.get("/a/b/c"), None);

        cache.put("/a/b/c", 42);
        assert_eq!(cache.get("/a/b/c"), Some(42));

        cache.invalidate();
        assert_eq!(cache.get("/a/b/c"), None);
    }

    #[test]
    fn test_cache_epoch_staleness() {
        let mut cache = L1PathCache::<32>::new();
        cache.put_with_epoch("/server/tickrate", 15, 1);

        // Matching epoch hits
        assert_eq!(cache.get_with_epoch("/server/tickrate", 1), Some(15));

        // Stale epoch (e.g. store mutated to revision 2) misses safely
        assert_eq!(cache.get_with_epoch("/server/tickrate", 2), None);
    }
}
