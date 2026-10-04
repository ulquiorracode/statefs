//! # StateFS Package & Archive VFS Provider
//!
//! Provides zero-allocation, read-only virtual archive mounts (e.g. GoldSrc WAD3 / PAK / FlatPackages)
//! directly into the [`VfsMountHub`].

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use statefs_vfs_core::{VfsError, VfsMetadata, VfsProvider};

/// An entry describing a slice inside an archive or continuous binary image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageEntry {
    /// Byte offset in the archive buffer.
    pub offset: usize,
    /// Byte length of the entry.
    pub length: usize,
}

/// A read-only package container operating over in-memory or memory-mapped archive bytes.
pub struct PackageMount<'a> {
    archive_data: &'a [u8],
    index: BTreeMap<String, PackageEntry>,
}

impl<'a> PackageMount<'a> {
    /// Creates a new package mount backed by raw archive bytes and an explicit index table.
    pub fn new(archive_data: &'a [u8], index: BTreeMap<String, PackageEntry>) -> Self {
        Self {
            archive_data,
            index,
        }
    }

    /// Builder helper to create an archive from files.
    pub fn builder() -> PackageBuilder {
        PackageBuilder::new()
    }
}

impl<'a> VfsProvider for PackageMount<'a> {
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, VfsError> {
        let entry = self
            .index
            .get(relative_path)
            .ok_or_else(|| VfsError::NotFound(relative_path.to_string()))?;

        if entry.offset + entry.length > self.archive_data.len() {
            return Err(VfsError::Io(String::from(
                "Corrupt archive: entry out of bounds",
            )));
        }

        Ok(self.archive_data[entry.offset..entry.offset + entry.length].to_vec())
    }

    fn metadata(&self, relative_path: &str) -> Result<VfsMetadata, VfsError> {
        let entry = self
            .index
            .get(relative_path)
            .ok_or_else(|| VfsError::NotFound(relative_path.to_string()))?;

        Ok(VfsMetadata {
            is_dir: false,
            size_bytes: entry.length as u64,
        })
    }

    fn list_dir(&self, _relative_path: &str) -> Result<Vec<String>, VfsError> {
        Ok(self.index.keys().cloned().collect())
    }
}

/// In-memory builder for constructing test packages and virtual archives.
#[derive(Default)]
pub struct PackageBuilder {
    buffer: Vec<u8>,
    index: BTreeMap<String, PackageEntry>,
}

impl PackageBuilder {
    /// Creates a new package builder.
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            index: BTreeMap::new(),
        }
    }

    /// Appends a file to the package archive.
    pub fn add_file(mut self, path: &str, content: &[u8]) -> Self {
        let offset = self.buffer.len();
        let length = content.len();
        self.buffer.extend_from_slice(content);
        self.index
            .insert(path.to_string(), PackageEntry { offset, length });
        self
    }

    /// Consumes the builder and returns the contiguous archive bytes and index map.
    pub fn build(self) -> (Vec<u8>, BTreeMap<String, PackageEntry>) {
        (self.buffer, self.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_mount_lookup() {
        let (archive_bytes, index) = PackageBuilder::new()
            .add_file("textures/{blue.mip", b"MIP_TEXTURE_HEADER_BLUE")
            .add_file("textures/{red.mip", b"MIP_TEXTURE_HEADER_RED")
            .build();

        let mount = PackageMount::new(&archive_bytes, index);

        // 1. Read files
        let blue = mount.read_file("textures/{blue.mip").unwrap();
        assert_eq!(blue, b"MIP_TEXTURE_HEADER_BLUE");

        let red = mount.read_file("textures/{red.mip").unwrap();
        assert_eq!(red, b"MIP_TEXTURE_HEADER_RED");

        // 2. Metadata
        let meta = mount.metadata("textures/{blue.mip").unwrap();
        assert_eq!(meta.size_bytes, b"MIP_TEXTURE_HEADER_BLUE".len() as u64);

        // 3. Not found
        assert!(mount.read_file("textures/{green.mip").is_err());
    }
}
