//! # Scenario Resolvers (The First-Level Policy Regulators)
//!
//! A [`ScenarioResolver`] acts as the border control / admission officer between
//! high-level execution scenarios (intents) and optimization adapters.
//!
//! Resolvers evaluate adapter passports against task requirements, routing execution
//! through the fastest permissible path without dynamic allocations (`Box<dyn>`).

use crate::cache::L1PathCache;
use crate::passport::{AdapterContract, WorkloadScenario};
use crate::simd::SimdScanner;
use statefs_core::{MemStore, Node};

/// Intent entering the query scenario resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryIntent<'a> {
    pub path: &'a str,
    pub scenario: WorkloadScenario,
}

impl<'a> QueryIntent<'a> {
    pub const fn new(path: &'a str, scenario: WorkloadScenario) -> Self {
        Self { path, scenario }
    }

    pub const fn hot_loop(path: &'a str) -> Self {
        Self {
            path,
            scenario: WorkloadScenario::SteadyStateLoop,
        }
    }

    pub const fn boot_load(path: &'a str) -> Self {
        Self {
            path,
            scenario: WorkloadScenario::BootLoading,
        }
    }
}

/// First-level Scenario Resolver coordinating MemStore, L1 Cache, and SIMD scanner.
pub struct QueryScenarioResolver<const CAP: usize = 64> {
    pub store: MemStore,
    pub cache: L1PathCache<CAP>,
    pub scanner: SimdScanner,
    pub metrics: ResolverMetrics,
}

/// Non-intrusive lightweight telemetry for the resolver.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResolverMetrics {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_bypasses: u64,
    pub total_queries: u64,
}

impl<const CAP: usize> QueryScenarioResolver<CAP> {
    /// Constructs a new resolver wrapping a storage instance.
    pub fn new(store: MemStore) -> Self {
        Self {
            store,
            cache: L1PathCache::new(),
            scanner: SimdScanner::new(),
            metrics: ResolverMetrics::default(),
        }
    }

    /// Primary entry point: queries the node by intent according to passport evaluation.
    #[inline(always)]
    pub fn resolve_query(&mut self, intent: QueryIntent) -> Option<&Node> {
        self.metrics.total_queries = self.metrics.total_queries.saturating_add(1);

        // 1. Passport & Visa evaluation
        let visa = self.cache.evaluate_visa(intent.scenario, intent.path);

        if visa.is_admitted() {
            // Fast lane: try L1 cache
            if let Some(cached_node_id) = self.cache.get(intent.path) {
                self.metrics.cache_hits = self.metrics.cache_hits.saturating_add(1);
                return self.store.get_by_id(cached_node_id);
            }

            self.metrics.cache_misses = self.metrics.cache_misses.saturating_add(1);

            // Resolve through core store and populate cache slot
            if let Some(node_id) = self.store.find_node_id(intent.path) {
                self.cache.put(intent.path, node_id);
                return self.store.get_by_id(node_id);
            }
            None
        } else {
            // Direct lane: bypass cache (e.g. boot loading or oversized path)
            self.metrics.cache_bypasses = self.metrics.cache_bypasses.saturating_add(1);
            self.store.get_str(intent.path)
        }
    }

    /// Read-only immutable query (does not mutate cache or metrics).
    #[inline(always)]
    pub fn resolve_query_ref(&self, intent: QueryIntent) -> Option<&Node> {
        let visa = self.cache.evaluate_visa(intent.scenario, intent.path);
        if visa.is_admitted()
            && let Some(cached_node_id) = self.cache.get(intent.path)
        {
            return self.store.get_by_id(cached_node_id);
        }
        self.store.get_str(intent.path)
    }
}
