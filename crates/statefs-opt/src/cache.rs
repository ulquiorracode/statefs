//! # L1 Direct-Mapped Path Cache Adapter
//!
//! A zero-allocation, fixed-capacity L1 cache designed for hot game-loop queries.
//!
//! - **Capacity via `const generic`**: Allocated inline in the structure without heap indirection.
//! - **Masked Hash Indexing**: For power-of-two capacities, lookup is a single bitwise `AND`.
//! - **Sub-10ns Lookup**: Bypasses tree traversal completely on cache hits.

use crate::passport::{AdapterContract, Passport, Visa, WorkloadScenario};

/// Default capacity for the L1 path cache (64 entries).
pub const DEFAULT_CACHE_CAP: usize = 64;

/// Ultra-fast 64-bit FNV-1a non-cryptographic hash for short path strings.
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

/// A cache entry storing the path hash and corresponding node ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheSlot {
    pub hash: u64,
    pub node_id: u32,
    pub valid: bool,
}

/// L1 Direct-Mapped Path Cache with compile-time inline capacity.
pub struct L1PathCache<const CAP: usize = DEFAULT_CACHE_CAP> {
    passport: Passport,
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
        Self {
            passport: Passport::new(
                "L1PathCache",
                128,  // Supports paths up to 128 bytes
                true, // Relies on immutable tree during hot frame
                1,    // Single repetition amortizes check
            ),
            slots: [CacheSlot {
                hash: 0,
                node_id: 0,
                valid: false,
            }; CAP],
        }
    }

    /// Looks up a cached `node_id` by raw path string.
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

    /// Stores a resolved `node_id` into the direct-mapped slot.
    #[inline(always)]
    pub fn put(&mut self, path: &str, node_id: u32) {
        let hash = fast_path_hash(path);
        let idx = (hash as usize) & (CAP - 1);
        self.slots[idx] = CacheSlot {
            hash,
            node_id,
            valid: true,
        };
    }

    /// Invalidates all entries in the cache (e.g. after a mutation).
    #[inline(always)]
    pub fn invalidate(&mut self) {
        for slot in &mut self.slots {
            slot.valid = false;
        }
    }
}

impl<const CAP: usize> AdapterContract for L1PathCache<CAP> {
    type Key = str;
    type Output = u32;

    #[inline(always)]
    fn passport(&self) -> &Passport {
        &self.passport
    }

    #[inline(always)]
    fn evaluate_visa(&self, scenario: WorkloadScenario, key: &Self::Key) -> Visa {
        if scenario == WorkloadScenario::BootLoading {
            return Visa::Rejected("Bypassed during boot loading to prevent cache pollution");
        }
        if key.len() > self.passport.max_key_len {
            return Visa::Rejected("Key exceeds maximum cacheable path length (128 bytes)");
        }
        Visa::Admitted
    }
}
