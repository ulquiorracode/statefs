//! StateFS node container encapsulating value and metadata.

use crate::value::Value;

/// Node stored at an exact `Path` in the StateFS hierarchy.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Node {
    /// Payload value.
    pub value: Value,
    /// Monotonically increasing revision counter for caching and change detection.
    pub revision: u64,
    /// If true, prevents mutations without explicit force/unlock.
    pub readonly: bool,
    /// If true, omitted from standard directory listings or external queries.
    pub hidden: bool,
}

impl Node {
    /// Constructs a standard node with revision 1.
    pub fn new(value: Value) -> Self {
        Self {
            value,
            revision: 1,
            readonly: false,
            hidden: false,
        }
    }

    /// Constructs a read-only node.
    pub fn read_only(value: Value) -> Self {
        Self {
            value,
            revision: 1,
            readonly: true,
            hidden: false,
        }
    }

    /// Constructs a hidden internal node.
    pub fn hidden(value: Value) -> Self {
        Self {
            value,
            revision: 1,
            readonly: false,
            hidden: true,
        }
    }

    /// Increments revision and updates payload value.
    pub fn update(&mut self, new_value: Value) {
        self.value = new_value;
        self.revision = self.revision.saturating_add(1);
    }
}
