//! # StateFS Binary Snapshot Codec
//!
//! Encodes state trees into disk-persisted binary images compliant with
//! [`statefs_adapter_opt_mmap::SnapshotHeader`].

use statefs_core::MemStore;
use statefs_core::backing::{RawNode, SNAPSHOT_MAGIC, SNAPSHOT_VERSION, SnapshotHeader};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

/// Writes an in-memory state tree snapshot to a binary file directly from [`MemStore`].
/// Serializes full arena topology, interned string pool, and typed value payloads.
pub fn export_snapshot<P: AsRef<Path>>(store: &MemStore, path: P) -> io::Result<()> {
    let (nodes, strings, values) = store.export_snapshot_parts();
    export_snapshot_raw(&nodes, strings, &values, path)
}

/// Low-level exporter for raw node slices, interned strings, and value bytes.
pub fn export_snapshot_raw<P: AsRef<Path>>(
    nodes: &[RawNode],
    strings: &[u8],
    values: &[u8],
    path: P,
) -> io::Result<()> {
    let mut file = File::create(path)?;

    let header = SnapshotHeader {
        magic: SNAPSHOT_MAGIC,
        version: SNAPSHOT_VERSION,
        node_count: nodes.len() as u32,
        string_bytes_len: strings.len() as u32,
        value_bytes_len: values.len() as u32,
        _pad: [0u8; 8],
    };

    // SAFETY: SnapshotHeader is repr(C) with 32 bytes
    let header_bytes = unsafe {
        core::slice::from_raw_parts(
            &header as *const SnapshotHeader as *const u8,
            size_of::<SnapshotHeader>(),
        )
    };
    file.write_all(header_bytes)?;

    for node in nodes {
        // SAFETY: RawNode is repr(C) pod
        let node_bytes = unsafe {
            core::slice::from_raw_parts(node as *const RawNode as *const u8, size_of::<RawNode>())
        };
        file.write_all(node_bytes)?;
    }
    file.write_all(strings)?;
    file.write_all(values)?;
    file.flush()?;

    Ok(())
}

/// Restores a full in-memory [`MemStore`] with topology, strings, and values from a binary snapshot file.
pub fn restore_snapshot<P: AsRef<Path>>(path: P) -> io::Result<MemStore> {
    let mut file = File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;

    let header_size = size_of::<SnapshotHeader>();
    if data.len() < header_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Snapshot file smaller than header",
        ));
    }

    let header = unsafe { core::ptr::read_unaligned(data.as_ptr() as *const SnapshotHeader) };
    if header.magic != SNAPSHOT_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid snapshot magic header",
        ));
    }
    if header.version != SNAPSHOT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Unsupported snapshot version: {}", header.version),
        ));
    }

    let node_count = header.node_count as usize;
    let node_size = size_of::<RawNode>();
    let nodes_bytes_total = node_count
        .checked_mul(node_size)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Node slice size overflow"))?;

    let node_offset = header_size;
    let string_offset = node_offset
        .checked_add(nodes_bytes_total)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Offset overflow"))?;

    let string_len = header.string_bytes_len as usize;
    let value_offset = string_offset
        .checked_add(string_len)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "String offset overflow"))?;

    let value_len = header.value_bytes_len as usize;
    let total_required = value_offset
        .checked_add(value_len)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Total size overflow"))?;

    if data.len() < total_required {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Snapshot file truncated",
        ));
    }

    let raw_nodes_bytes = &data[node_offset..node_offset + nodes_bytes_total];
    let raw_nodes: Vec<RawNode> = raw_nodes_bytes
        .chunks_exact(node_size)
        .map(|chunk| unsafe { core::ptr::read_unaligned(chunk.as_ptr() as *const RawNode) })
        .collect();

    let string_bytes = &data[string_offset..string_offset + string_len];
    let value_bytes = &data[value_offset..value_offset + value_len];

    MemStore::from_raw_parts(&raw_nodes, string_bytes, value_bytes).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to restore store: {:?}", e),
        )
    })
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
        assert!(!backing.value_bytes().is_empty());

        // Verify full store restoration with values
        let restored = restore_snapshot(&tmp_path).expect("restore snapshot");
        assert_eq!(
            restored
                .get(&StatePath::parse("/server/tickrate"))
                .map(|n| &n.value),
            Some(&Value::Int(128))
        );
        assert_eq!(
            restored
                .get(&StatePath::parse("/server/hostname"))
                .map(|n| &n.value),
            Some(&Value::String("StateFS Arena".into()))
        );
        assert_eq!(
            restored
                .get(&StatePath::parse("/game/mp_friendlyfire"))
                .map(|n| &n.value),
            Some(&Value::Bool(true))
        );

        let _ = std::fs::remove_file(&tmp_path);
    }

    #[test]
    fn test_corrupt_node_link_fails_validation() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp_path = std::env::temp_dir().join(format!("statefs_corrupt_node_{}.bin", unique));

        let mut store = MemStore::new();
        store
            .insert(&StatePath::parse("/a/b"), Value::Int(1))
            .expect("insert");
        let (mut nodes, strings, values) = store.export_snapshot_parts();
        // Craft an out-of-bounds child pointer
        nodes[1].first_child = 99999;
        export_snapshot_raw(&nodes, strings, &values, &tmp_path).expect("export raw");

        let res = statefs_adapter_opt_mmap::MmapStorageBacking::open(&tmp_path);
        assert!(res.is_err(), "Must reject out-of-bounds first_child");

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
