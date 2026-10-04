//! # StateFS Runtime & Scenario Orchestrator
//!
//! Orchestrates access to `statefs-core` through `stitch-rs` U-Cycle pipelines
//! and scenario resolvers without dynamic allocation.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use statefs_adapter_opt_cache::{DEFAULT_CACHE_CAP, L1PathCache};
use statefs_adapter_opt_simd::SimdPathScanner;
use statefs_core::{MemStore, Node};
use stitch_rs::middleware::TerminalHandler;

/// Execution workload scenario under which a query operates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WorkloadScenario {
    /// Cold startup or mass batch write (bypasses caches to avoid pollution).
    BootLoading,

    /// High-frequency steady-state frame loop (99.9% read-heavy, hot path).
    #[default]
    SteadyStateLoop,

    /// Ad-hoc analytical or administrative query (unpredictable, wide paths).
    AdHocQuery,
}

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
pub struct QueryScenarioResolver<const CAP: usize = DEFAULT_CACHE_CAP> {
    pub store: MemStore,
    pub cache: L1PathCache<CAP>,
    pub scanner: SimdPathScanner,
    pub metrics: ResolverMetrics,
}

/// Telemetry metrics for resolver inspections.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResolverMetrics {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_bypasses: u64,
    pub total_queries: u64,
}

impl<const CAP: usize> QueryScenarioResolver<CAP> {
    pub fn new(store: MemStore) -> Self {
        Self {
            store,
            cache: L1PathCache::new(),
            scanner: SimdPathScanner::new(),
            metrics: ResolverMetrics::default(),
        }
    }

    #[inline(always)]
    pub fn resolve_query(&mut self, intent: QueryIntent) -> Option<&Node> {
        self.metrics.total_queries = self.metrics.total_queries.saturating_add(1);

        if intent.scenario == WorkloadScenario::SteadyStateLoop {
            if let Some(cached_node_id) = self.cache.get(intent.path) {
                self.metrics.cache_hits = self.metrics.cache_hits.saturating_add(1);
                return self.store.get_by_id(cached_node_id);
            }

            self.metrics.cache_misses = self.metrics.cache_misses.saturating_add(1);

            if let Some(node_id) = self.store.find_node_id(intent.path) {
                self.cache.put(intent.path, node_id);
                return self.store.get_by_id(node_id);
            }
            None
        } else {
            self.metrics.cache_bypasses = self.metrics.cache_bypasses.saturating_add(1);
            self.store.get_str(intent.path)
        }
    }
}

/// Execution context for stitch-rs U-cycle dispatches.
#[derive(Debug, Default)]
pub struct StateFsContext {
    pub dispatches: u64,
}

/// Terminal handler executing the query at the bottom of the stitch-rs U-cycle.
pub struct StateFsTerminal<const CAP: usize = DEFAULT_CACHE_CAP> {
    pub resolver: QueryScenarioResolver<CAP>,
}

impl<const CAP: usize> StateFsTerminal<CAP> {
    pub fn new(resolver: QueryScenarioResolver<CAP>) -> Self {
        Self { resolver }
    }
}

impl<'a, const CAP: usize>
    TerminalHandler<StateFsContext, QueryIntent<'a>, Option<Node>, &'static str>
    for StateFsTerminal<CAP>
{
    #[inline(always)]
    fn execute(
        &mut self,
        ctx: &mut StateFsContext,
        intent: QueryIntent<'a>,
    ) -> Result<Option<Node>, &'static str> {
        ctx.dispatches = ctx.dispatches.saturating_add(1);
        let node = self.resolver.resolve_query(intent).cloned();
        Ok(node)
    }
}

/// Extension trait extending [`MemStore`] with fluent runtime builders.
pub trait RuntimeExt: Sized {
    fn with_l1_cache<const CAP: usize>(self) -> QueryScenarioResolver<CAP>;
    fn with_default_l1_cache(self) -> QueryScenarioResolver<DEFAULT_CACHE_CAP> {
        self.with_l1_cache::<DEFAULT_CACHE_CAP>()
    }
}

impl RuntimeExt for MemStore {
    #[inline(always)]
    fn with_l1_cache<const CAP: usize>(self) -> QueryScenarioResolver<CAP> {
        QueryScenarioResolver::new(self)
    }
}
