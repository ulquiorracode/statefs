//! # Storage Backing Port
//!
//! Formal decoupled storage ports allowing StateFS to run over arbitrary physical memory:
//! heap vectors, static ROM buffers, or memory-mapped files (`memmap2` + `zerocopy`).

use crate::error::StoreError;

/// C-ABI stable representation of a tree node in physical memory.
///
/// Enables zero-copy memory mapping directly from disk or network without parsing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawNode {
    pub symbol_offset: u32,
    pub symbol_len: u32,
    pub first_child: u32,
    pub next_sibling: u32,
    pub revision: u64,
    pub flags: u32,
}

/// Port abstraction for physical backing providers.
pub trait StorageBacking {
    /// Returns the contiguous slice of raw nodes in memory.
    fn nodes(&self) -> &[RawNode];

    /// Returns the contiguous byte buffer containing interned symbol strings.
    fn string_bytes(&self) -> &[u8];

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
