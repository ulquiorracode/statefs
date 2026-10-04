//! # StateFS Binary Snapshot Codec
//!
//! Encodes state trees into disk-persisted binary images compliant with
//! [`statefs_adapter_opt_mmap::SnapshotHeader`].

use statefs_adapter_opt_mmap::{SNAPSHOT_MAGIC, SnapshotHeader};
use statefs_core::MemStore;
use statefs_core::backing::RawNode;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use zerocopy::IntoBytes;

/// Writes an in-memory state tree snapshot to a binary file.
pub fn export_snapshot<P: AsRef<Path>>(
    _store: &MemStore,
    nodes: &[RawNode],
    strings: &[u8],
    path: P,
) -> io::Result<()> {
    let mut file = File::create(path)?;

    let header = SnapshotHeader {
        magic: SNAPSHOT_MAGIC,
        version: 1,
        node_count: nodes.len() as u32,
        string_bytes_len: strings.len() as u32,
        reserved: 0,
    };

    file.write_all(header.as_bytes())?;
    for node in nodes {
        // SAFETY: RawNode is repr(C) pod
        let node_bytes = unsafe {
            core::slice::from_raw_parts(node as *const RawNode as *const u8, size_of::<RawNode>())
        };
        file.write_all(node_bytes)?;
    }
    file.write_all(strings)?;
    file.flush()?;

    Ok(())
}
