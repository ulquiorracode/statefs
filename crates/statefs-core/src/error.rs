//! Core error definitions for StateFS.

use crate::path::Path;
use alloc::string::String;
use core::error::Error;
use core::fmt::{Display, Formatter, Result};

/// Core errors emitted by StateFS store operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Target path was not found in the store.
    NotFound(Path),
    /// Attempted to mutate a node marked as read-only.
    ReadOnly(Path),
    /// Path parsing or manipulation error.
    InvalidPath(String),
    /// Hierarchy conflict (e.g. attempting to insert under an existing scalar leaf).
    Conflict(String),
}

impl Display for StoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Self::NotFound(p) => write!(f, "Node not found at path: {p}"),
            Self::ReadOnly(p) => write!(f, "Cannot mutate read-only node at path: {p}"),
            Self::InvalidPath(msg) => write!(f, "Invalid path: {msg}"),
            Self::Conflict(msg) => write!(f, "Hierarchy conflict: {msg}"),
        }
    }
}

#[cfg(feature = "std")]
impl Error for StoreError {}
