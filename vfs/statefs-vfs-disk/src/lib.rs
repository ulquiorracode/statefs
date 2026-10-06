//! # StateFS Physical Disk VFS Provider
//!
//! Exposes physical filesystem directories as [`VfsProvider`] mounts while
//! enforcing strict sandbox path validation to prevent directory traversal exploits.

use std::fs;
use std::path::{Component, Path, PathBuf};

use statefs_vfs_core::{VfsError, VfsMetadata, VfsProvider};

/// A physical directory mount provider.
#[derive(Debug, Clone)]
pub struct DiskMount {
    root_path: PathBuf,
}

impl DiskMount {
    /// Creates a new disk mount pointing to a physical root directory.
    pub fn new<P: AsRef<Path>>(root: P) -> Result<Self, VfsError> {
        let canonical = root.as_ref().canonicalize().map_err(|e| {
            VfsError::Io(format!(
                "Failed to canonicalize root path {}: {e}",
                root.as_ref().display()
            ))
        })?;

        if !canonical.is_dir() {
            return Err(VfsError::Io(format!(
                "Root path is not a directory: {}",
                canonical.display()
            )));
        }

        Ok(Self {
            root_path: canonical,
        })
    }

    /// Resolves and verifies that a relative path stays within the root sandbox.
    fn resolve_safe_path(&self, rel: &str) -> Result<PathBuf, VfsError> {
        let rel_path = Path::new(rel);

        // Security check: Reject parent dir components (`..`) and absolute components
        for comp in rel_path.components() {
            match comp {
                Component::ParentDir => {
                    return Err(VfsError::AccessDenied(format!(
                        "Directory traversal attempt detected: {rel}"
                    )));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(VfsError::AccessDenied(format!(
                        "Absolute paths forbidden in relative resolution: {rel}"
                    )));
                }
                Component::CurDir | Component::Normal(_) => {}
            }
        }

        let joined = self.root_path.join(rel_path);
        let canon = match joined.canonicalize() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VfsError::NotFound(String::from(rel)));
            }
            Err(e) => return Err(VfsError::Io(e.to_string())),
        };

        if !canon.starts_with(&self.root_path) {
            return Err(VfsError::AccessDenied(format!(
                "Path escapes root sandbox: {rel}"
            )));
        }

        Ok(canon)
    }
}

impl VfsProvider for DiskMount {
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, VfsError> {
        let safe_target = self.resolve_safe_path(relative_path)?;
        let meta = fs::metadata(&safe_target).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => VfsError::NotFound(String::from(relative_path)),
            _ => VfsError::Io(e.to_string()),
        })?;
        if meta.is_dir() {
            return Err(VfsError::NotFound(String::from(relative_path)));
        }

        fs::read(&safe_target).map_err(|e| VfsError::Io(e.to_string()))
    }

    fn metadata(&self, relative_path: &str) -> Result<VfsMetadata, VfsError> {
        let safe_target = self.resolve_safe_path(relative_path)?;
        let meta = fs::metadata(&safe_target).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => VfsError::NotFound(String::from(relative_path)),
            _ => VfsError::Io(e.to_string()),
        })?;
        Ok(VfsMetadata {
            is_dir: meta.is_dir(),
            size_bytes: meta.len(),
        })
    }

    fn list_dir(&self, relative_path: &str) -> Result<Vec<String>, VfsError> {
        let safe_target = if relative_path.is_empty() {
            self.root_path.clone()
        } else {
            self.resolve_safe_path(relative_path)?
        };
        let meta = fs::metadata(&safe_target).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => VfsError::NotFound(String::from(relative_path)),
            _ => VfsError::Io(e.to_string()),
        })?;
        if !meta.is_dir() {
            return Err(VfsError::NotFound(String::from(relative_path)));
        }

        let read_dir = fs::read_dir(&safe_target).map_err(|e| VfsError::Io(e.to_string()))?;
        let mut results = Vec::new();

        for entry in read_dir {
            let entry = entry.map_err(|e| VfsError::Io(e.to_string()))?;
            if let Ok(name) = entry.file_name().into_string() {
                results.push(name);
            }
        }

        results.sort();
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disk_mount_read_and_traversal_guard() {
        let temp_dir = std::env::temp_dir().join("statefs_test_disk_mount");
        let _ = fs::create_dir_all(temp_dir.join("configs"));
        let test_file = temp_dir.join("configs").join("game.cfg");
        fs::write(&test_file, b"hostname Dedicated").unwrap();

        let mount = DiskMount::new(&temp_dir).unwrap();

        // 1. Valid safe read
        let data = mount.read_file("configs/game.cfg").unwrap();
        assert_eq!(data, b"hostname Dedicated");

        // 2. Listing directory
        let files = mount.list_dir("configs").unwrap();
        assert_eq!(files, vec!["game.cfg"]);

        // 3. Security: traversal guard stops `..`
        assert!(matches!(
            mount.read_file("../some_secret.txt"),
            Err(VfsError::AccessDenied(_))
        ));

        // Clean up
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
