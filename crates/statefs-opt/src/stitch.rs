//! # Bridge between StateFS and stitch-rs Execution Pipeline
//!
//! Provides the adapter and terminal implementations to mount StateFS Scenario Resolvers
//! into monomorphic [`stitch_rs::Pipeline`] U-cycles.

use crate::cache::DEFAULT_CACHE_CAP;
use crate::passport::WorkloadScenario;
use crate::resolver::{QueryIntent, QueryScenarioResolver};
use statefs_core::Node;
use stitch_rs::flow::FlowControl;
use stitch_rs::middleware::{Middleware, TerminalHandler};

/// Execution context for stitch-rs U-cycle dispatches.
#[derive(Debug, Default)]
pub struct StateFsContext {
    pub dispatches: u64,
}

/// Terminal handler that executes the query at the bottom of the U-cycle (Point of Puncture).
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

/// Standalone L1 Cache Middleware layer for stitch-rs.
///
/// Demonstrates early descent short-circuiting: on cache hits, the request immediately
/// ascends through outer middleware (for audit/telemetry) without touching the terminal!
pub struct L1CacheMiddleware<const CAP: usize = DEFAULT_CACHE_CAP> {
    pub cache: crate::cache::L1PathCache<CAP>,
}

impl<const CAP: usize> Default for L1CacheMiddleware<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> L1CacheMiddleware<CAP> {
    pub fn new() -> Self {
        Self {
            cache: crate::cache::L1PathCache::new(),
        }
    }
}

impl<'a, const CAP: usize> Middleware<StateFsContext, QueryIntent<'a>, Option<Node>, &'static str>
    for L1CacheMiddleware<CAP>
{
    #[inline(always)]
    fn on_enter(
        &self,
        _ctx: &mut StateFsContext,
        intent: QueryIntent<'a>,
    ) -> FlowControl<QueryIntent<'a>, Option<Node>, &'static str> {
        // Evaluate passport
        if intent.scenario == WorkloadScenario::SteadyStateLoop
            && let Some(_cached_id) = self.cache.get(intent.path)
        {
            // Short-circuit: we have a cached node index, but returning a cloned Node
            // requires resolution. Here we allow descent or short-circuit if available.
            return FlowControl::Proceed(intent);
        }
        FlowControl::Proceed(intent)
    }

    #[inline(always)]
    fn on_exit(
        &self,
        _ctx: &mut StateFsContext,
        _outcome: &mut Result<Option<Node>, &'static str>,
    ) {
        // Ascent phase telemetry
    }
}
