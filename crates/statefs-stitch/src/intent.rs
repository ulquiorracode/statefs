//! Intent & Outcome model for The Sewing Machine Architecture (SMA).
//!
//! An `Intent` is the "Thread" (Нить) fed into the machine. It carries desire,
//! but has no authority to execute or mutate on its own.
//!
//! An `Outcome` is the "Tightened Knot" (Затянутый стежок) resulting from
//! the needle's round-trip U-cycle through the fabric.

use alloc::vec::Vec;
use statefs_core::{Node, Path, StoreError, Value};

/// Standard state manipulation desires that can enter the pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum StateIntent {
    /// Desires to read a node at a given path.
    Get { path: Path },

    /// Desires to set a value at a given path.
    Set { path: Path, value: Value },

    /// Desires to set a full node with metadata/flags at a given path.
    SetNode { path: Path, node: Node },

    /// Desires to remove a node at a given path.
    Remove { path: Path },

    /// Desires to list direct children beneath a prefix path.
    ListChildren { prefix: Path },

    /// Desires to list all recursive subpaths beneath a prefix path.
    ListSubpaths { prefix: Path },
}

/// The result returned from the bottom of the stitch (execution against fabric).
#[derive(Debug, Clone, PartialEq)]
pub enum StateOutcome {
    /// Node was found or read.
    Read(Option<Node>),

    /// Node was written. Contains previous node if one existed.
    Written {
        path: Path,
        previous: Option<Node>,
        current: Node,
    },

    /// Node was removed. Contains the removed node if one existed.
    Removed { path: Path, removed: Option<Node> },

    /// List of paths returned.
    PathList(Vec<Path>),
}

/// Decision made by a Layer during the descent phase: accept or refuse.
#[derive(Debug, Clone, PartialEq)]
pub enum Admission<TIntent> {
    /// The intent is admitted and passed down to the next layer (potentially modified/enriched).
    Admit(TIntent),

    /// The intent is refused by policy or validation. The descent halts immediately.
    Refuse(Refusal),
}

/// Explanation for why an intent was refused by a layer.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// Insufficient permissions or rejected by security guard.
    AccessDenied(&'static str),

    /// Validation failed (schema violation, invalid bounds).
    InvalidData(&'static str),

    /// Node is marked read-only or immutable.
    Immutable(&'static str),

    /// Generic domain refusal with explanation.
    Custom(&'static str),

    /// Underlying storage or kernel failure.
    Store(StoreError),
}

impl From<StoreError> for Refusal {
    fn from(err: StoreError) -> Self {
        Refusal::Store(err)
    }
}
