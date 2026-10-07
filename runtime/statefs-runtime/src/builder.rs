//! Ergonomic configuration builder and factory facade for StateFS.

use alloc::string::String;
use statefs_core::{MemStore, Path, StateSource, Store, StoreError, Value};

#[cfg(feature = "std")]
use std::path::Path as StdPath;

use crate::{DEFAULT_CACHE_CAP, QueryScenarioResolver};

/// Errors encountered while assembling a StateFS store or resolver.
#[derive(Debug)]
pub enum StateFsError {
    Store(StoreError),
    #[cfg(feature = "toml")]
    Toml(statefs_codec_toml::TomlCodecError),
    #[cfg(feature = "json")]
    Json(statefs_codec_json::JsonCodecError),
    #[cfg(feature = "snapshot")]
    Snapshot(std::io::Error),
    #[cfg(feature = "serde")]
    Serde(statefs_adapter_bridge_serde::DeError),
    #[cfg(feature = "std")]
    Io(std::io::Error),
    Custom(String),
}

impl core::fmt::Display for StateFsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Store(e) => write!(f, "Store error: {e}"),
            #[cfg(feature = "toml")]
            Self::Toml(e) => write!(f, "TOML error: {e}"),
            #[cfg(feature = "json")]
            Self::Json(e) => write!(f, "JSON error: {e}"),
            #[cfg(feature = "snapshot")]
            Self::Snapshot(e) => write!(f, "Snapshot error: {e}"),
            #[cfg(feature = "serde")]
            Self::Serde(e) => write!(f, "Serde extraction error: {e}"),
            #[cfg(feature = "std")]
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for StateFsError {}

impl From<StoreError> for StateFsError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

#[cfg(feature = "toml")]
impl From<statefs_codec_toml::TomlCodecError> for StateFsError {
    fn from(e: statefs_codec_toml::TomlCodecError) -> Self {
        Self::Toml(e)
    }
}

#[cfg(feature = "json")]
impl From<statefs_codec_json::JsonCodecError> for StateFsError {
    fn from(e: statefs_codec_json::JsonCodecError) -> Self {
        Self::Json(e)
    }
}

/// Primary entry point facade for constructing StateFS stores and resolvers.
pub struct StateFs;

impl StateFs {
    /// Creates a new fluent [`StateFsBuilder`].
    pub fn builder() -> StateFsBuilder {
        StateFsBuilder::new()
    }

    /// Creates a fresh empty [`MemStore`].
    pub fn new_store() -> MemStore {
        MemStore::new()
    }

    /// Creates a default [`QueryScenarioResolver`] with fresh store, L1 cache, and SIMD scanner.
    pub fn default_resolver() -> QueryScenarioResolver<DEFAULT_CACHE_CAP> {
        QueryScenarioResolver::new(MemStore::new())
    }
}

/// Fluent builder for composing StateFS hierarchical stores from multiple pluggable sources.
#[derive(Debug)]
pub struct StateFsBuilder {
    store: MemStore,
}

impl Default for StateFsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl StateFsBuilder {
    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self {
            store: MemStore::new(),
        }
    }

    /// Applies an arbitrary external [`StateSource`] (SPI port) into the store.
    pub fn with_source<S: StateSource>(mut self, source: S) -> Result<Self, StateFsError> {
        source.apply_to_store(&mut self.store)?;
        Ok(self)
    }

    /// Manually inserts a typed scalar or complex value at the specified path.
    pub fn with_value(mut self, path: &str, value: impl Into<Value>) -> Result<Self, StateFsError> {
        let p = Path::parse(path);
        self.store.insert(&p, value.into())?;
        Ok(self)
    }

    /// Manually inserts a read-only value at the specified path to enforce immutable overrides.
    pub fn with_readonly_value(
        mut self,
        path: &str,
        value: impl Into<Value>,
    ) -> Result<Self, StateFsError> {
        let p = Path::parse(path);
        let node = statefs_core::Node::read_only(value.into());
        self.store.insert_node(&p, node)?;
        Ok(self)
    }

    /// Ingests a raw TOML string into the store.
    #[cfg(feature = "toml")]
    pub fn with_toml_str(mut self, raw_toml: &str) -> Result<Self, StateFsError> {
        statefs_codec_toml::ingest_toml(&mut self.store, raw_toml)?;
        Ok(self)
    }

    /// Ingests a TOML configuration file from the filesystem.
    #[cfg(all(feature = "toml", feature = "std"))]
    pub fn with_toml_file<P: AsRef<StdPath>>(self, path: P) -> Result<Self, StateFsError> {
        let content = std::fs::read_to_string(path).map_err(StateFsError::Io)?;
        self.with_toml_str(&content)
    }

    /// Ingests a raw JSON string into the store.
    #[cfg(feature = "json")]
    pub fn with_json_str(mut self, raw_json: &str) -> Result<Self, StateFsError> {
        statefs_codec_json::ingest_json(&mut self.store, raw_json)?;
        Ok(self)
    }

    /// Ingests a JSON configuration file from the filesystem.
    #[cfg(all(feature = "json", feature = "std"))]
    pub fn with_json_file<P: AsRef<StdPath>>(self, path: P) -> Result<Self, StateFsError> {
        let content = std::fs::read_to_string(path).map_err(StateFsError::Io)?;
        self.with_json_str(&content)
    }

    /// Ingests system environment variables matching `prefix` (e.g. `"APP"` or `"SERVER"`).
    #[cfg(feature = "env")]
    pub fn with_env(mut self, prefix: &str) -> Result<Self, StateFsError> {
        let source = statefs_adapter_bridge_env::EnvSource::new().with_prefix(prefix);
        source.apply_to_store(&mut self.store)?;
        Ok(self)
    }

    /// Ingests environment variables using a configured [`EnvSource`].
    #[cfg(feature = "env")]
    pub fn with_env_source(
        mut self,
        source: statefs_adapter_bridge_env::EnvSource,
    ) -> Result<Self, StateFsError> {
        source.apply_to_store(&mut self.store)?;
        Ok(self)
    }

    /// Restores or merges store state from an in-memory binary snapshot slice.
    #[cfg(feature = "snapshot")]
    pub fn with_snapshot_bytes(mut self, bytes: &[u8]) -> Result<Self, StateFsError> {
        let snapshot_store =
            statefs_codec_bin::restore_snapshot_bytes(bytes).map_err(StateFsError::Snapshot)?;
        self.merge_snapshot(snapshot_store)?;
        Ok(self)
    }

    /// Restores or merges store state from a binary snapshot file.
    #[cfg(all(feature = "snapshot", feature = "std"))]
    pub fn with_snapshot_file<P: AsRef<StdPath>>(mut self, path: P) -> Result<Self, StateFsError> {
        let snapshot_store =
            statefs_codec_bin::restore_snapshot(path).map_err(StateFsError::Snapshot)?;
        self.merge_snapshot(snapshot_store)?;
        Ok(self)
    }

    #[cfg(feature = "snapshot")]
    fn merge_snapshot(&mut self, snapshot_store: MemStore) -> Result<(), StateFsError> {
        let is_empty = self.store.arena_len() <= 1 && self.store.get(&Path::root()).is_none();
        if is_empty {
            self.store = snapshot_store;
        } else {
            if let Some(root_node) = snapshot_store.get(&Path::root()) {
                self.store.insert_node(&Path::root(), root_node.clone())?;
            }
            for (path, node) in snapshot_store.find_glob("/**") {
                self.store.insert_node(&path, node.clone())?;
            }
        }
        Ok(())
    }

    /// Consumes the builder and returns the populated [`MemStore`].
    pub fn build(self) -> Result<MemStore, StateFsError> {
        Ok(self.store)
    }

    /// Consumes the builder and constructs an optimized [`QueryScenarioResolver`].
    pub fn build_resolver<const CAP: usize>(
        self,
    ) -> Result<QueryScenarioResolver<CAP>, StateFsError> {
        Ok(QueryScenarioResolver::new(self.store))
    }

    /// Extracts a strongly-typed struct directly from the configured store at `path`.
    #[cfg(feature = "serde")]
    pub fn extract<T: for<'de> serde::Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<T, StateFsError> {
        statefs_adapter_bridge_serde::extract(&self.store, path).map_err(StateFsError::Serde)
    }
}
