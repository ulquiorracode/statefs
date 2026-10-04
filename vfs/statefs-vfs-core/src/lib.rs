//! # StateFS VFS Core
//!
//! Provides the architectural ports, mount abstractions, and search path overlays
//! for Virtual File Systems operating over [`statefs_core`].

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Errors that can occur during VFS operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsError {
    /// The requested path was not found in the mount or provider.
    NotFound(String),
    /// Access was denied or security policy violated (e.g. traversal attempt).
    AccessDenied(String),
    /// An I/O error occurred in the backing provider.
    Io(String),
    /// Path format is invalid or malformed.
    InvalidPath(String),
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "VFS resource not found: {path}"),
            Self::AccessDenied(path) => write!(f, "VFS access denied / security violation: {path}"),
            Self::Io(msg) => write!(f, "VFS IO error: {msg}"),
            Self::InvalidPath(msg) => write!(f, "Invalid VFS path: {msg}"),
        }
    }
}

/// Metadata descriptor for a VFS entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsMetadata {
    /// Whether the entry is a directory/container.
    pub is_dir: bool,
    /// Size of the payload in bytes (0 for directories).
    pub size_bytes: u64,
}

/// Abstract VFS provider port.
///
/// Implementors can represent physical directories, in-memory caches,
/// or packaged game archives (.wad, .pak, .zip).
pub trait VfsProvider: Send + Sync {
    /// Reads the entire contents of a file at the given relative path.
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, VfsError>;

    /// Fetches metadata for an entry if it exists.
    fn metadata(&self, relative_path: &str) -> Result<VfsMetadata, VfsError>;

    /// Lists direct children names of a directory.
    fn list_dir(&self, relative_path: &str) -> Result<Vec<String>, VfsError>;
}

/// Mount entry inside [`VfsMountHub`].
struct MountEntry {
    prefix: String,
    provider: Box<dyn VfsProvider>,
}

/// Hub that dispatches absolute virtual paths to mounted [`VfsProvider`] instances.
pub struct VfsMountHub {
    mounts: Vec<MountEntry>,
}

impl Default for VfsMountHub {
    fn default() -> Self {
        Self::new()
    }
}

impl VfsMountHub {
    /// Creates an empty mount hub.
    pub fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    /// Mounts a provider at the specified virtual path prefix.
    ///
    /// Longest prefix matches have priority during dispatch.
    pub fn mount<P: VfsProvider + 'static>(&mut self, virtual_prefix: &str, provider: P) {
        let clean_prefix = normalize_prefix(virtual_prefix);
        self.mounts.push(MountEntry {
            prefix: clean_prefix,
            provider: Box::new(provider),
        });

        // Sort descending by prefix length so deepest prefixes match first
        self.mounts
            .sort_by_key(|entry| core::cmp::Reverse(entry.prefix.len()));
    }

    /// Finds the matching provider and strips the prefix.
    fn resolve_mount<'h, 'p>(&'h self, clean: &'p str) -> Option<(&'h dyn VfsProvider, &'p str)> {
        for entry in &self.mounts {
            if clean == entry.prefix {
                return Some((entry.provider.as_ref(), ""));
            }
            if clean.starts_with(&entry.prefix) {
                let remainder = &clean[entry.prefix.len()..];
                if let Some(stripped) = remainder.strip_prefix('/') {
                    return Some((entry.provider.as_ref(), stripped));
                }
            }
        }
        None
    }

    /// Reads a file from the appropriate mounted provider.
    pub fn read_file(&self, virtual_path: &str) -> Result<Vec<u8>, VfsError> {
        let clean = clean_path(virtual_path);
        let (provider, rel) = self
            .resolve_mount(&clean)
            .ok_or_else(|| VfsError::NotFound(String::from(virtual_path)))?;
        provider.read_file(rel)
    }

    /// Fetches metadata from the appropriate mounted provider.
    pub fn metadata(&self, virtual_path: &str) -> Result<VfsMetadata, VfsError> {
        let clean = clean_path(virtual_path);
        let (provider, rel) = self
            .resolve_mount(&clean)
            .ok_or_else(|| VfsError::NotFound(String::from(virtual_path)))?;
        provider.metadata(rel)
    }

    /// Lists directory contents from the appropriate mounted provider.
    pub fn list_dir(&self, virtual_path: &str) -> Result<Vec<String>, VfsError> {
        let clean = clean_path(virtual_path);
        let (provider, rel) = self
            .resolve_mount(&clean)
            .ok_or_else(|| VfsError::NotFound(String::from(virtual_path)))?;
        provider.list_dir(rel)
    }
}

/// Overlay search path resolver.
///
/// Evaluates providers in priority order (e.g., Mod Layer -> Base Layer -> Default Layer).
/// Writes or fallback lookups seamlessly traverse the stack.
pub struct VfsOverlay {
    layers: Vec<Box<dyn VfsProvider>>,
}

impl Default for VfsOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl VfsOverlay {
    /// Creates a new overlay stack.
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Adds a layer to the top of the search stack (highest priority).
    pub fn push_top<P: VfsProvider + 'static>(&mut self, provider: P) {
        self.layers.insert(0, Box::new(provider));
    }

    /// Adds a layer to the bottom of the search stack (lowest priority fallback).
    pub fn push_bottom<P: VfsProvider + 'static>(&mut self, provider: P) {
        self.layers.push(Box::new(provider));
    }
}

impl VfsProvider for VfsOverlay {
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, VfsError> {
        for layer in &self.layers {
            if let Ok(bytes) = layer.read_file(relative_path) {
                return Ok(bytes);
            }
        }
        Err(VfsError::NotFound(String::from(relative_path)))
    }

    fn metadata(&self, relative_path: &str) -> Result<VfsMetadata, VfsError> {
        for layer in &self.layers {
            if let Ok(meta) = layer.metadata(relative_path) {
                return Ok(meta);
            }
        }
        Err(VfsError::NotFound(String::from(relative_path)))
    }

    fn list_dir(&self, relative_path: &str) -> Result<Vec<String>, VfsError> {
        let mut combined = Vec::new();
        let mut any_found = false;

        for layer in &self.layers {
            if let Ok(entries) = layer.list_dir(relative_path) {
                any_found = true;
                for entry in entries {
                    if !combined.contains(&entry) {
                        combined.push(entry);
                    }
                }
            }
        }

        if any_found {
            Ok(combined)
        } else {
            Err(VfsError::NotFound(String::from(relative_path)))
        }
    }
}

/// Normalizes a mount prefix ensuring clean `/path` format without trailing slash.
fn normalize_prefix(prefix: &str) -> String {
    let clean = clean_path(prefix);
    if clean.is_empty() {
        String::from("/")
    } else {
        clean
    }
}

/// Cleans path separators and removes redundant slashes.
pub fn clean_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 1);
    let bytes = path.as_bytes();
    let mut i = 0;

    // Ensure leading slash
    if !path.starts_with('/') && !path.starts_with('\\') {
        out.push('/');
    }

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'/' || b == b'\\' {
            // Push single slash and skip consecutive slashes
            out.push('/');
            while i < bytes.len() && (bytes[i] == b'/' || bytes[i] == b'\\') {
                i += 1;
            }
        } else {
            out.push(b as char);
            i += 1;
        }
    }

    // Strip trailing slash if longer than 1 character
    if out.len() > 1 && out.ends_with('/') {
        out.pop();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;
    use alloc::vec;

    struct MemoryMockProvider {
        files: BTreeMap<String, Vec<u8>>,
    }

    impl VfsProvider for MemoryMockProvider {
        fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, VfsError> {
            self.files
                .get(relative_path)
                .cloned()
                .ok_or_else(|| VfsError::NotFound(String::from(relative_path)))
        }

        fn metadata(&self, relative_path: &str) -> Result<VfsMetadata, VfsError> {
            if let Some(data) = self.files.get(relative_path) {
                Ok(VfsMetadata {
                    is_dir: false,
                    size_bytes: data.len() as u64,
                })
            } else {
                Err(VfsError::NotFound(String::from(relative_path)))
            }
        }

        fn list_dir(&self, _relative_path: &str) -> Result<Vec<String>, VfsError> {
            Ok(self.files.keys().cloned().collect())
        }
    }

    #[test]
    fn test_vfs_mount_dispatch() {
        let mut hub = VfsMountHub::new();

        let mut provider1 = MemoryMockProvider {
            files: BTreeMap::new(),
        };
        provider1
            .files
            .insert(String::from("server.cfg"), b"hostname Test".to_vec());

        let mut provider2 = MemoryMockProvider {
            files: BTreeMap::new(),
        };
        provider2
            .files
            .insert(String::from("game.cfg"), b"mp_timelimit 30".to_vec());

        hub.mount("/cstrike/addons", provider1);
        hub.mount("/cstrike", provider2);

        // Longest prefix match: /cstrike/addons
        let content1 = hub.read_file("/cstrike/addons/server.cfg").unwrap();
        assert_eq!(content1, b"hostname Test");

        // General prefix match: /cstrike
        let content2 = hub.read_file("/cstrike/game.cfg").unwrap();
        assert_eq!(content2, b"mp_timelimit 30");

        // Not found
        assert!(hub.read_file("/valve/unknown.cfg").is_err());
    }

    #[test]
    fn test_vfs_overlay_search_priority() {
        let mut mod_layer = MemoryMockProvider {
            files: BTreeMap::new(),
        };
        mod_layer
            .files
            .insert(String::from("maps/de_dust2.bsp"), vec![1, 2, 3]);

        let mut base_layer = MemoryMockProvider {
            files: BTreeMap::new(),
        };
        base_layer
            .files
            .insert(String::from("maps/de_dust2.bsp"), vec![9, 9, 9]); // Outdated base map
        base_layer
            .files
            .insert(String::from("maps/crossfire.bsp"), vec![4, 5, 6]);

        let mut overlay = VfsOverlay::new();
        overlay.push_bottom(base_layer);
        overlay.push_top(mod_layer); // Mod layer overrides base layer

        // de_dust2 must come from mod layer
        let dust = overlay.read_file("maps/de_dust2.bsp").unwrap();
        assert_eq!(dust, vec![1, 2, 3]);

        // crossfire falls back to base layer
        let cross = overlay.read_file("maps/crossfire.bsp").unwrap();
        assert_eq!(cross, vec![4, 5, 6]);
    }
}
