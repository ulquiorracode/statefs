//! Monomorphic U-Cycle Stitch Engine.
//!
//! Chains layers recursively using zero-sized static dispatch types.
//! When compiled with `--release`, the entire chain of `admit` -> `terminal` -> `tighten`
//! is fully inlined by LLVM into a single flat block of machine code.

use crate::intent::{Admission, Refusal};
use crate::layer::{Layer, Terminal};
use core::marker::PhantomData;

/// The Sewing Machine that drives the needle through all attached layers.
pub struct Machine<TCtx, TIntent, TOutcome, TChain> {
    chain: TChain,
    _phantom: PhantomData<(TCtx, TIntent, TOutcome)>,
}

impl<TCtx, TIntent, TOutcome, TTerm> Machine<TCtx, TIntent, TOutcome, TerminalNode<TTerm>>
where
    TTerm: Terminal<TCtx, TIntent, TOutcome>,
{
    /// Starts constructing a new stitching machine ending with the specified terminal handler.
    pub const fn on_terminal(terminal: TTerm) -> Self {
        Self {
            chain: TerminalNode(terminal),
            _phantom: PhantomData,
        }
    }
}

impl<TCtx, TIntent, TOutcome, TChain> Machine<TCtx, TIntent, TOutcome, TChain> {
    /// Wraps the current chain with an additional outer layer.
    ///
    /// The new layer executes *before* the inner chain on descent, and *after* on ascent.
    pub fn wrap<L>(self, layer: L) -> Machine<TCtx, TIntent, TOutcome, StackNode<L, TChain>>
    where
        L: Layer<TCtx, TIntent, TOutcome>,
    {
        Machine {
            chain: StackNode {
                layer,
                inner: self.chain,
            },
            _phantom: PhantomData,
        }
    }
}

/// A leaf terminal node wrapping the terminal implementation.
pub struct TerminalNode<T>(pub T);

/// A node in the compile-time stack of layers.
pub struct StackNode<L, Inner> {
    pub layer: L,
    pub inner: Inner,
}

/// Trait implemented by the entire monomorphic stack (both layers and terminal).
pub trait StitchChain<TCtx, TIntent, TOutcome> {
    /// Performs the complete U-Cycle: descent through remaining layers,
    /// puncture at terminal, and ascent back up.
    fn cycle(&mut self, ctx: &mut TCtx, intent: TIntent) -> Result<TOutcome, Refusal>;
}

// 1. Base case: TerminalNode (Дно буквы U)
impl<TCtx, TIntent, TOutcome, TTerm> StitchChain<TCtx, TIntent, TOutcome> for TerminalNode<TTerm>
where
    TTerm: Terminal<TCtx, TIntent, TOutcome>,
{
    #[inline(always)]
    fn cycle(&mut self, ctx: &mut TCtx, intent: TIntent) -> Result<TOutcome, Refusal> {
        self.0.execute(ctx, intent)
    }
}

// 2. Recursive case: Layer + Inner Stack
impl<TCtx, TIntent, TOutcome, L, Inner> StitchChain<TCtx, TIntent, TOutcome> for StackNode<L, Inner>
where
    L: Layer<TCtx, TIntent, TOutcome>,
    Inner: StitchChain<TCtx, TIntent, TOutcome>,
{
    #[inline(always)]
    fn cycle(&mut self, ctx: &mut TCtx, intent: TIntent) -> Result<TOutcome, Refusal> {
        // Phase 1: Descent (Спуск / Прокол слоя)
        let intent = match self.layer.admit(ctx, intent) {
            Admission::Admit(admitted) => admitted,
            Admission::Refuse(refusal) => return Err(refusal),
        };

        // Inner recursion (спуск дальше к терминалу)
        let mut outcome = self.inner.cycle(ctx, intent);

        // Phase 2: Ascent (Подъем / Затягивание узла)
        self.layer.tighten(ctx, &mut outcome);

        outcome
    }
}

impl<TCtx, TIntent, TOutcome, TChain> Machine<TCtx, TIntent, TOutcome, TChain>
where
    TChain: StitchChain<TCtx, TIntent, TOutcome>,
{
    /// Drives the needle through the entire machine: **Work = Machine::stitch(Material, Intent)**.
    #[inline(always)]
    pub fn stitch(&mut self, ctx: &mut TCtx, intent: TIntent) -> Result<TOutcome, Refusal> {
        self.chain.cycle(ctx, intent)
    }
}
