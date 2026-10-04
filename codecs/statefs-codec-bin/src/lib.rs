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

/// Writes an in-memory state tree snapshot to a binary file directly from [`MemStore`].
pub fn export_snapshot<P: AsRef<Path>>(store: &MemStore, path: P) -> io::Result<()> {
    let (nodes, strings) = store.export_raw_nodes();
    export_snapshot_raw(&nodes, strings, path)
}

/// Low-level exporter for raw node slices and strings.
pub fn export_snapshot_raw<P: AsRef<Path>>(
    nodes: &[RawNode],
    strings: &[u8],
    path: P,
) -> io::Result<()> {
    let mut file = File::create(path)?;

    let header = SnapshotHeader {
        magic: SNAPSHOT_MAGIC,
        version: statefs_adapter_opt_mmap::SNAPSHOT_VERSION,
        node_count: nodes.len() as u32,
        string_bytes_len: strings.len() as u32,
        reserved: 0,
        _pad: [0u8; 8],
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

#[cfg(test)]
mod tests {
    use super::*;
    use statefs_core::backing::StorageBacking;
    use statefs_core::path::Path as StatePath;
    use statefs_core::store::Store;
    use statefs_core::value::Value;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_export_and_mmap_roundtrip() {
        let mut store = MemStore::new();
        store
            .insert(&StatePath::parse("/server/tickrate"), Value::Int(128))
            .expect("insert tickrate");
        store
            .insert(
                &StatePath::parse("/server/hostname"),
                Value::String("StateFS Arena".into()),
            )
            .expect("insert hostname");
        store
            .insert(
                &StatePath::parse("/game/mp_friendlyfire"),
                Value::Bool(true),
            )
            .expect("insert cvar");

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp_path = std::env::temp_dir().join(format!("statefs_test_{}.bin", unique));

        // Export real snapshot
        export_snapshot(&store, &tmp_path).expect("export snapshot");

        // Verify mmap open and reading
        let backing = statefs_adapter_opt_mmap::MmapStorageBacking::open(&tmp_path)
            .expect("mmap open snapshot");
        assert!(backing.nodes().len() >= 3);

        let _ = std::fs::remove_file(&tmp_path);
    }

    #[test]
    fn test_truncated_snapshot_fails_gracefully() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp_path = std::env::temp_dir().join(format!("statefs_corrupt_{}.bin", unique));

        // Write an incomplete header (less than 32 bytes)
        std::fs::write(&tmp_path, b"STATEFS\0short").expect("write corrupt");
        let res = statefs_adapter_opt_mmap::MmapStorageBacking::open(&tmp_path);
        assert!(res.is_err());

        let _ = std::fs::remove_file(&tmp_path);
    }
}
