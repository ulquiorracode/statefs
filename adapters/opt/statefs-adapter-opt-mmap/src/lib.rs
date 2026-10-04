//! # StateFS Zero-Copy Memory-Mapped Storage Backing
//!
//! Maps flat state snapshots directly into process virtual memory using `memmap2`,
//! transmuting bytes into `[RawNode]` slices safely via `zerocopy` without runtime parsing.

use memmap2::Mmap;
use statefs_core::backing::{RawNode, StorageBacking};
use std::fs::File;
use std::io;
use std::path::Path;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// Header placed at the beginning of a StateFS binary snapshot file.
#[repr(C)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct SnapshotHeader {
    pub magic: [u8; 8], // b"STATEFS\0"
    pub version: u32,   // 1
    pub node_count: u32,
    pub string_bytes_len: u32,
    pub reserved: u32,
}

pub const SNAPSHOT_MAGIC: [u8; 8] = *b"STATEFS\0";

/// Memory-mapped file backing provider implementing [`StorageBacking`].
pub struct MmapStorageBacking {
    mmap: Mmap,
    node_offset: usize,
    node_count: usize,
    string_offset: usize,
    string_len: usize,
}

impl MmapStorageBacking {
    /// Opens and memory-maps a binary StateFS snapshot file.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        if mmap.len() < size_of::<SnapshotHeader>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small for StateFS snapshot header",
            ));
        }

        // Parse header via zerocopy
        let (header_ref, _) = SnapshotHeader::ref_from_prefix(&mmap[..])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid header alignment"))?;

        if header_ref.magic != SNAPSHOT_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid snapshot magic header",
            ));
        }

        let header_size = size_of::<SnapshotHeader>();
        let node_count = header_ref.node_count as usize;
        let node_size = size_of::<RawNode>();
        let nodes_bytes_total = node_count * node_size;

        let node_offset = header_size;
        let string_offset = node_offset + nodes_bytes_total;
        let string_len = header_ref.string_bytes_len as usize;

        if mmap.len() < string_offset + string_len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Snapshot file truncated",
            ));
        }

        Ok(Self {
            mmap,
            node_offset,
            node_count,
            string_offset,
            string_len,
        })
    }
}

impl StorageBacking for MmapStorageBacking {
    fn nodes(&self) -> &[RawNode] {
        let node_slice_bytes = &self.mmap
            [self.node_offset..self.node_offset + (self.node_count * size_of::<RawNode>())];
        // SAFETY: RawNode is repr(C) pod data aligned and bounds checked
        unsafe {
            core::slice::from_raw_parts(
                node_slice_bytes.as_ptr() as *const RawNode,
                self.node_count,
            )
        }
    }

    fn string_bytes(&self) -> &[u8] {
        &self.mmap[self.string_offset..self.string_offset + self.string_len]
    }
}
