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
/// Aligned to 32 bytes to ensure subsequent `RawNode` slices remain 8-byte aligned.
#[repr(C)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct SnapshotHeader {
    pub magic: [u8; 8], // b"STATEFS\0"
    pub version: u32,   // 1
    pub node_count: u32,
    pub string_bytes_len: u32,
    pub reserved: u32,
    pub _pad: [u8; 8], // 8 + 4 + 4 + 4 + 4 + 8 = 32 bytes (8-byte aligned offset)
}

pub const SNAPSHOT_MAGIC: [u8; 8] = *b"STATEFS\0";
pub const SNAPSHOT_VERSION: u32 = 1;

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

        if header_ref.version != SNAPSHOT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Unsupported snapshot version: expected {}, got {}",
                    SNAPSHOT_VERSION, header_ref.version
                ),
            ));
        }

        let header_size = size_of::<SnapshotHeader>();
        let node_count = header_ref.node_count as usize;
        let node_size = size_of::<RawNode>();

        let nodes_bytes_total = node_count.checked_mul(node_size).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Node slice size overflow")
        })?;

        let node_offset = header_size;
        let string_offset = node_offset
            .checked_add(nodes_bytes_total)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Offset overflow"))?;

        let string_len = header_ref.string_bytes_len as usize;
        let total_required = string_offset
            .checked_add(string_len)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Total size overflow"))?;

        if mmap.len() < total_required {
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
