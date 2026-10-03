//! Layer trait: The contract of a single fabric slice in the U-Cycle.
//!
//! Each Layer participates in two phases of the stitch:
//! 1. **Descent (`admit`)**: Inspects/enriches the intent. Can `Admit` or `Refuse`.
//! 2. **Ascent (`tighten`)**: Observes the outcome emerging from the bottom.
//!    Can trigger reactions, compute diffs, or log telemetry.

use crate::intent::{Admission, Refusal};

/// A composable layer through which the stitching needle passes.
///
/// Both `TCtx` (context/state) and `TIntent` / `TOutcome` are generic, allowing
/// the Sewing Machine Architecture to remain strictly agnostic of domain types.
pub trait Layer<TCtx, TIntent, TOutcome> {
    /// Phase 1: Descent (Спуск).
    ///
    /// Evaluates whether to admit the incoming intent down to deeper layers.
    /// Returns `Admission::Admit(intent)` to proceed or `Admission::Refuse(reason)` to halt.
    fn admit(&self, ctx: &mut TCtx, intent: TIntent) -> Admission<TIntent>;

    /// Phase 2: Ascent (Подъем).
    ///
    /// Observes the outcome emerging from the layer below and tightens the stitch
    /// (e.g. emits diffs, updates metrics, executes secondary reactive intents).
    fn tighten(&self, ctx: &mut TCtx, outcome: &mut Result<TOutcome, Refusal>);
}

/// The bottom-most terminal handler that touches the Material (Fabric).
///
/// This is the "Point of Puncture" (Точка прокола / Дно буквы U).
pub trait Terminal<TCtx, TIntent, TOutcome> {
    /// Executes the intent directly against the raw material and returns the atomic outcome.
    fn execute(&mut self, ctx: &mut TCtx, intent: TIntent) -> Result<TOutcome, Refusal>;
}
