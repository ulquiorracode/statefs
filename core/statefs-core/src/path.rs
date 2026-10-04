//! Canonical hierarchical segmented path for StateFS.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::{self, Display, Formatter};
use core::str::FromStr;

/// Configuration options for parsing hierarchical paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathOptions<'a> {
    /// Separator characters dividing segments (e.g., `&['/', '\\']`).
    pub separators: &'a [char],
    /// Whether to trim leading/trailing whitespace around each segment.
    pub trim_segments: bool,
    /// Whether to ignore empty segments resulting from consecutive separators (e.g., `//` -> `/`).
    pub ignore_empty: bool,
}

impl PathOptions<'static> {
    /// Standard URI/Unix file path separators: `/` and `\`.
    pub const DEFAULT: Self = Self {
        separators: &['/', '\\'],
        trim_segments: true,
        ignore_empty: true,
    };

    /// Dot-separated property notation: `.`.
    pub const DOT_NOTATION: Self = Self {
        separators: &['.'],
        trim_segments: true,
        ignore_empty: true,
    };

    /// Comprehensive separators including dots: `/`, `\`, `.`.
    pub const PERMISSIVE: Self = Self {
        separators: &['/', '\\', '.'],
        trim_segments: true,
        ignore_empty: true,
    };
}

impl Default for PathOptions<'static> {
    #[inline]
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A canonical, OS-independent segmented path in the StateFS hierarchy.
///
/// Paths are always composed of clean UTF-8 string segments and are free
/// of platform-specific separators (`\` or `/`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Path {
    segments: Vec<String>,
}

impl Path {
    /// Constructs a new empty (root) path.
    #[inline]
    pub const fn root() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Constructs a path from an iterator or sequence of segments.
    pub fn from_segments<I, S>(iter: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let segments = iter
            .into_iter()
            .map(Into::into)
            .filter(|s| !s.is_empty())
            .collect();
        Self { segments }
    }

    /// Parses a raw string path using standard path separators (`/` and `\`).
    pub fn parse(raw: &str) -> Self {
        Self::parse_with_options(raw, &PathOptions::DEFAULT)
    }

    /// Parses a raw string path using explicit parsing options and separators.
    pub fn parse_with_options(raw: &str, options: &PathOptions) -> Self {
        let mut segments = Vec::new();
        let normalized = if options.trim_segments {
            raw.trim()
        } else {
            raw
        };

        for part in normalized.split(options.separators) {
            let seg = if options.trim_segments {
                part.trim()
            } else {
                part
            };
            if !options.ignore_empty || !seg.is_empty() {
                segments.push(seg.to_owned());
            }
        }

        Self { segments }
    }

    /// Returns `true` if this path points to the root of the hierarchy.
    #[inline]
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// Returns the number of segments in this path.
    #[inline]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Returns `true` if the path has no segments (root path).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Returns the segments slice.
    #[inline]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Appends a sub-path or segment, returning a new `Path`.
    pub fn join(&self, sub: impl AsRef<str>) -> Self {
        let mut new_path = self.clone();
        for part in sub.as_ref().split(['/', '\\', '.']) {
            let seg = part.trim();
            if !seg.is_empty() {
                new_path.segments.push(seg.to_owned());
            }
        }
        new_path
    }

    /// Returns the parent path, or `None` if this path is already root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            None
        } else {
            let mut parent_segs = self.segments.clone();
            parent_segs.pop();
            Some(Self {
                segments: parent_segs,
            })
        }
    }

    /// Returns the leaf segment name of this path, or `None` if root.
    pub fn leaf(&self) -> Option<&str> {
        self.segments.last().map(|s| s.as_str())
    }

    /// Returns `true` if this path starts with the given `prefix`.
    pub fn starts_with(&self, prefix: &Path) -> bool {
        if prefix.segments.len() > self.segments.len() {
            return false;
        }
        self.segments[..prefix.segments.len()] == prefix.segments[..]
    }

    /// Strips the given `prefix` from this path, returning the remaining relative subpath.
    pub fn strip_prefix(&self, prefix: &Path) -> Option<Self> {
        if !self.starts_with(prefix) {
            return None;
        }
        let remaining = self.segments[prefix.segments.len()..].to_vec();
        Some(Self {
            segments: remaining,
        })
    }
}

impl Display for Path {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.is_root() {
            return write!(f, "/");
        }
        for seg in &self.segments {
            write!(f, "/{seg}")?;
        }
        Ok(())
    }
}

impl FromStr for Path {
    type Err = core::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::parse(s))
    }
}

impl From<&str> for Path {
    fn from(s: &str) -> Self {
        Self::parse(s)
    }
}

impl From<String> for Path {
    fn from(s: String) -> Self {
        Self::parse(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_parse_and_display() {
        let p1 = Path::parse("/plugins/moderation/cvars/ban_time");
        assert_eq!(
            p1.segments(),
            &["plugins", "moderation", "cvars", "ban_time"]
        );
        assert_eq!(p1.to_string(), "/plugins/moderation/cvars/ban_time");

        let p2 = Path::parse_with_options(
            "plugins.moderation.errors.not_found",
            &PathOptions::DOT_NOTATION,
        );
        assert_eq!(
            p2.segments(),
            &["plugins", "moderation", "errors", "not_found"]
        );
        assert_eq!(p2.to_string(), "/plugins/moderation/errors/not_found");

        let root = Path::parse("/");
        assert!(root.is_root());
        assert_eq!(root.to_string(), "/");
    }

    #[test]
    fn test_path_parent_and_join() {
        let p = Path::parse("/a/b/c");
        let parent = p.parent().unwrap();
        assert_eq!(parent.to_string(), "/a/b");
        assert_eq!(parent.parent().unwrap().to_string(), "/a");
        assert_eq!(parent.parent().unwrap().parent().unwrap().to_string(), "/");
        assert_eq!(parent.parent().unwrap().parent().unwrap().parent(), None);

        let joined = parent.join("c/d");
        assert_eq!(joined.to_string(), "/a/b/c/d");
        assert_eq!(joined.leaf(), Some("d"));
    }

    #[test]
    fn test_path_prefix() {
        let p = Path::parse("/plugins/moderation/cvars/timeout");
        let prefix = Path::parse("/plugins/moderation");
        assert!(p.starts_with(&prefix));

        let rel = p.strip_prefix(&prefix).unwrap();
        assert_eq!(rel.to_string(), "/cvars/timeout");
    }
}
