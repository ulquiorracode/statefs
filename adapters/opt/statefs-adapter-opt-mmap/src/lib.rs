//! # StateFS Zero-Copy Memory-Mapped Storage Backing
//!
//! Maps flat state snapshots directly into process virtual memory using `memmap2`,
//! transmuting bytes into `[RawNode]` slices safely via `zerocopy` without runtime parsing.

use memmap2::Mmap;
use statefs_core::backing::{RawNode, SNAPSHOT_MAGIC, SNAPSHOT_VERSION, StorageBacking};
use std::fs::File;
use std::io;
use std::path::Path;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// Memory-mapped snapshot header for zerocopy prefix reading.
#[repr(C)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct MmapSnapshotHeader {
    pub magic: [u8; 8], // b"STATEFS\0"
    pub version: u32,   // 2
    pub node_count: u32,
    pub string_bytes_len: u32,
    pub value_bytes_len: u32,
    pub _pad: [u8; 8], // 8 + 4 + 4 + 4 + 4 + 8 = 32 bytes (8-byte aligned offset)
}

/// Memory-mapped file backing provider implementing [`StorageBacking`].
pub struct MmapStorageBacking {
    mmap: Mmap,
    node_offset: usize,
    node_count: usize,
    string_offset: usize,
    string_len: usize,
    value_offset: usize,
    value_len: usize,
}

impl MmapStorageBacking {
    /// Opens and memory-maps a binary StateFS snapshot file.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        if mmap.len() < size_of::<MmapSnapshotHeader>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small for StateFS snapshot header",
            ));
        }

        // Parse header via zerocopy
        let (header_ref, _) = MmapSnapshotHeader::ref_from_prefix(&mmap[..])
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

        let header_size = size_of::<MmapSnapshotHeader>();
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
        let value_offset = string_offset
            .checked_add(string_len)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "String offset overflow"))?;

        let value_len = header_ref.value_bytes_len as usize;
        let total_required = value_offset
            .checked_add(value_len)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Total size overflow"))?;

        if mmap.len() < total_required {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Snapshot file truncated",
            ));
        }

        let backing = Self {
            mmap,
            node_offset,
            node_count,
            string_offset,
            string_len,
            value_offset,
            value_len,
        };

        // Audit check: validate all nodes and bounds to prevent OOB or segfaults on corrupt files
        backing.validate()?;

        Ok(backing)
    }

    /// Validates the structural integrity and bounds of nodes, symbols, and values.
    pub fn validate(&self) -> io::Result<()> {
        let nodes = self.nodes();
        let string_len = self.string_len;
        let value_len = self.value_len;

        for (i, node) in nodes.iter().enumerate() {
            // Validate symbol boundaries
            let s_start = node.symbol_offset as usize;
            let s_len = node.symbol_len as usize;
            let s_end = s_start.checked_add(s_len).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Symbol offset overflow")
            })?;
            if s_end > string_len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Node {i} symbol offset out of bounds ({s_end} > {string_len})"),
                ));
            }

            // Validate value payload boundaries if present
            if (node.flags & 4) != 0 && node.value_len > 0 {
                let v_start = node.value_offset as usize;
                let v_l = node.value_len as usize;
                let v_end = v_start.checked_add(v_l).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Value offset overflow")
                })?;
                if v_end > value_len {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Node {i} value offset out of bounds ({v_end} > {value_len})"),
                    ));
                }
            }

            // Validate tree links
            if node.first_child != u32::MAX && node.first_child as usize >= nodes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Node {i} first_child index out of bounds: {}",
                        node.first_child
                    ),
                ));
            }
            if node.next_sibling != u32::MAX && node.next_sibling as usize >= nodes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Node {i} next_sibling index out of bounds: {}",
                        node.next_sibling
                    ),
                ));
            }
        }

        Ok(())
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

    fn value_bytes(&self) -> &[u8] {
        &self.mmap[self.value_offset..self.value_offset + self.value_len]
    }
}
