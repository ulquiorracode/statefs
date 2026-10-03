//! Layer trait: The contract of a single fabric slice in the U-Cycle.
//!
//! Each Layer participates in two phases of the stitch:
//! 1. **Descent (`admit`)**: Inspects/enriches the intent. Can `Admit` or `Refuse`.
//! 2. **Ascent (`tighten`)**: Observes the outcome emerging from the bottom.
//!    Can trigger reactions, compute diffs, or log telemetry.

use crate::intent::{Admission, Refusal};

/// A composable middleware layer through which the U-cycle dispatch passes.
///
/// Both `TCtx` (context/state) and `TIntent` / `TOutcome` are generic, allowing
/// the pipeline to remain strictly agnostic of domain types.
pub trait Layer<TCtx, TIntent, TOutcome> {
    /// Phase 1: Descent (Entering the pipeline).
    ///
    /// Evaluates whether to admit the incoming intent down to deeper layers.
    /// Returns `Admission::Admit(intent)` to proceed or `Admission::Refuse(reason)` to halt.
    fn admit(&self, ctx: &mut TCtx, intent: TIntent) -> Admission<TIntent>;

    /// Phase 2: Ascent (Exiting the pipeline).
    ///
    /// Observes the outcome emerging from the layer below and tightens the pipeline
    /// (e.g. emits diffs, updates metrics, executes secondary reactive intents).
    fn tighten(&self, ctx: &mut TCtx, outcome: &mut Result<TOutcome, Refusal>);

    /// Alias for Phase 1 Descent using standard middleware terminology.
    #[inline(always)]
    fn on_enter(&self, ctx: &mut TCtx, intent: TIntent) -> Admission<TIntent> {
        self.admit(ctx, intent)
    }

    /// Alias for Phase 2 Ascent using standard middleware terminology.
    #[inline(always)]
    fn on_exit(&self, ctx: &mut TCtx, outcome: &mut Result<TOutcome, Refusal>) {
        self.tighten(ctx, outcome)
    }
}

/// Standard industry alias for a composable pipeline interceptor.
pub trait Middleware<TCtx, TIntent, TOutcome>: Layer<TCtx, TIntent, TOutcome> {}
impl<T, TCtx, TIntent, TOutcome> Middleware<TCtx, TIntent, TOutcome> for T where
    T: Layer<TCtx, TIntent, TOutcome>
{
}

/// The bottom-most terminal handler that touches the Material (Fabric).
///
/// This is the "Point of Puncture".
pub trait Terminal<TCtx, TIntent, TOutcome> {
    /// Executes the intent directly against the raw material and returns the atomic outcome.
    fn execute(&mut self, ctx: &mut TCtx, intent: TIntent) -> Result<TOutcome, Refusal>;
}

/// Standard industry alias for terminal execution handler.
pub trait TerminalHandler<TCtx, TIntent, TOutcome>: Terminal<TCtx, TIntent, TOutcome> {}
impl<T, TCtx, TIntent, TOutcome> TerminalHandler<TCtx, TIntent, TOutcome> for T where
    T: Terminal<TCtx, TIntent, TOutcome>
{
}
