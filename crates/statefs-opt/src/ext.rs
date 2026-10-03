//! # Extension Traits for Ergonomic Composition
//!
//! Provides zero-cost builder methods (`with_l1_cache`, `into_resolver`)
//! directly on any compatible [`statefs_core::MemStore`].

use crate::cache::DEFAULT_CACHE_CAP;
use crate::resolver::QueryScenarioResolver;
use statefs_core::MemStore;

/// Extension trait extending [`MemStore`] with zero-cost optimization adapters.
pub trait StoreOptExt: Sized {
    /// Wraps this memory store into a [`QueryScenarioResolver`] with an L1 path cache.
    fn with_l1_cache<const CAP: usize>(self) -> QueryScenarioResolver<CAP>;

    /// Wraps this memory store with the default L1 path cache (64 slots).
    fn with_default_l1_cache(self) -> QueryScenarioResolver<DEFAULT_CACHE_CAP> {
        self.with_l1_cache::<DEFAULT_CACHE_CAP>()
    }
}

impl StoreOptExt for MemStore {
    #[inline(always)]
    fn with_l1_cache<const CAP: usize>(self) -> QueryScenarioResolver<CAP> {
        QueryScenarioResolver::new(self)
    }
}
