//! # Storage Backing Port
//!
//! Formal decoupled storage ports allowing StateFS to run over arbitrary physical memory:
//! heap vectors, static ROM buffers, or memory-mapped files (`memmap2` + `zerocopy`).

use crate::error::StoreError;

/// Magic signature for StateFS snapshot binary files.
pub const SNAPSHOT_MAGIC: [u8; 8] = *b"STATEFS\0";

/// Current supported snapshot container format version.
/// Version 2 includes the typed value payload table.
pub const SNAPSHOT_VERSION: u32 = 2;

/// Header placed at the beginning of a StateFS binary snapshot file.
/// Aligned to 32 bytes to ensure subsequent `RawNode` slices remain 8-byte aligned.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotHeader {
    pub magic: [u8; 8], // b"STATEFS\0"
    pub version: u32,   // 2
    pub node_count: u32,
    pub string_bytes_len: u32,
    pub value_bytes_len: u32,
    pub _pad: [u8; 8], // 8 + 4 + 4 + 4 + 4 + 8 = 32 bytes (8-byte aligned offset)
}

impl Default for SnapshotHeader {
    fn default() -> Self {
        Self {
            magic: SNAPSHOT_MAGIC,
            version: SNAPSHOT_VERSION,
            node_count: 0,
            string_bytes_len: 0,
            value_bytes_len: 0,
            _pad: [0u8; 8],
        }
    }
}

/// C-ABI stable representation of a tree node in physical memory.
///
/// Enables zero-copy memory mapping directly from disk or network without parsing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawNode {
    pub symbol_offset: u32,
    pub symbol_len: u32,
    pub value_offset: u32,
    pub value_len: u32,
    pub first_child: u32,
    pub next_sibling: u32,
    pub revision: u64,
    pub flags: u32,
    pub _pad: u32,
}

/// Port abstraction for physical backing providers.
pub trait StorageBacking {
    /// Returns the contiguous slice of raw nodes in memory.
    fn nodes(&self) -> &[RawNode];

    /// Returns the contiguous byte buffer containing interned symbol strings.
    fn string_bytes(&self) -> &[u8];

    /// Returns the contiguous byte buffer containing serialized value payloads.
    fn value_bytes(&self) -> &[u8] {
        &[]
    }

    /// Optional mutation capability.
    fn mutate_node(
        &mut self,
        index: u32,
        f: &mut dyn FnMut(&mut RawNode),
    ) -> Result<(), StoreError> {
        let _ = (index, f);
        Err(StoreError::Conflict(alloc::string::String::from(
            "Storage backing is immutable",
        )))
    }
}
