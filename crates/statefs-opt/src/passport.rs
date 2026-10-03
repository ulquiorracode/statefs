//! # Adapter Passport & Contract Module
//!
//! Every optimization adapter in StateFS provides a [`Passport`] (its identity,
//! workload eligibility requirements, and execution constraints).
//!
//! Scenario Resolvers inspect this passport to decide whether an adapter receives
//! admission ("visa") for a given task scenario.

use core::fmt::Debug;

/// Execution workload scenario under which a task operates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkloadScenario {
    /// Cold startup or mass batch write (cache pollution hazard, favors bulk ingestion).
    BootLoading,

    /// High-frequency steady-state frame loop (99.9% read-heavy, latency critical).
    SteadyStateLoop,

    /// Ad-hoc analytical or administrative query (unpredictable, wide paths).
    AdHocQuery,

    /// Transactional mutation burst (selective invalidate / dirty propagation).
    MutationBurst,
}

/// Passport declaration: capabilities, limits, and cost profile of an adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Passport {
    /// Human-readable identifier of the adapter.
    pub name: &'static str,

    /// Maximum key/path byte length this adapter can handle without degradation.
    pub max_key_len: usize,

    /// Whether this adapter requires an immutable/stable state tree.
    pub requires_immutable_tree: bool,

    /// Minimum frequency required to amortize adapter overhead (0 = always beneficial).
    pub min_frequency_threshold: u32,
}

impl Passport {
    /// Creates a new adapter passport.
    pub const fn new(
        name: &'static str,
        max_key_len: usize,
        requires_immutable_tree: bool,
        min_frequency_threshold: u32,
    ) -> Self {
        Self {
            name,
            max_key_len,
            requires_immutable_tree,
            min_frequency_threshold,
        }
    }
}

/// Visa decision issued by a scenario resolver to an adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visa {
    /// Admitted: the adapter is optimal for this task.
    Admitted,

    /// Rejected: the adapter is unsuitable (e.g. exceeds key length or cache hazard).
    Rejected(&'static str),
}

impl Visa {
    #[inline(always)]
    pub fn is_admitted(&self) -> bool {
        matches!(self, Visa::Admitted)
    }
}

/// Universal trait contract implemented by all StateFS optimization adapters.
pub trait AdapterContract {
    /// Target input key type (e.g. `str` or `Path`).
    type Key: ?Sized;

    /// Output resolved data (e.g. `u32` for node index or `&Node`).
    type Output;

    /// Returns the static passport descriptor of this adapter.
    fn passport(&self) -> &Passport;

    /// Evaluates visa admission for the given scenario and input key.
    #[inline(always)]
    fn evaluate_visa(&self, scenario: WorkloadScenario, _key: &Self::Key) -> Visa {
        match scenario {
            WorkloadScenario::BootLoading => {
                Visa::Rejected("BootLoading scenario bypasses speculative caches")
            }
            WorkloadScenario::SteadyStateLoop
            | WorkloadScenario::AdHocQuery
            | WorkloadScenario::MutationBurst => Visa::Admitted,
        }
    }
}
