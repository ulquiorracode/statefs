//! # StateFS Configuration Manager & Bridge
//!
//! Provides a Figment-like, high-performance configuration management engine
//! powered by the zero-allocation hierarchical StateFS kernel.
//!
//! ## Key Capabilities
//! - **Source Stacking & Precedence**: Multi-layer composition (Defaults -> Files -> CVars -> Env -> Ad-hoc overrides).
//! - **Profile-Aware**: Transparent profile selection (`[default]`, `[staging]`, `[production]`, `[competitive]`).
//! - **Rich Diagnostics**: Contextual `ConfigError` pinpointing keys, origins, and type mismatches.
//! - **Subtree Mount**: Mount isolated subtrees into custom mount points (e.g. `/plugins/vip`).
//! - **Direct Serde Extraction**: Extract typed configurations directly into domain structs.
//! - **config-rs Drop-in Compatibility**: Preserves [`ConfigBridge`] for legacy interoperability.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path as StdPath, PathBuf};

use statefs_core::{MemStore, Node, Path, StateSource, Store, StoreError, Value};

/// High-level profile selector for layered configurations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Profile(pub String);

impl Profile {
    /// The canonical default fallback profile.
    pub const DEFAULT: &'static str = "default";

    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self(Self::DEFAULT.to_string())
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<S: Into<String>> From<S> for Profile {
    fn from(s: S) -> Self {
        Self::new(s)
    }
}

/// Rich contextual errors produced during configuration building and extraction.
#[derive(Debug)]
pub enum ConfigError {
    /// Key was not found in the layered store.
    NotFound { path: String, profile: String },
    /// Value at path did not match expected scalar or structure type.
    TypeMismatch {
        path: String,
        expected: &'static str,
        actual: String,
    },
    /// An I/O error occurred while reading a file source.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Codec parse error (TOML, JSON, etc.).
    Parse { origin: String, message: String },
    /// Serde extraction error.
    Serde(String),
    /// Underlying StateFS store failure.
    Store(StoreError),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path, profile } => {
                write!(
                    f,
                    "configuration key '{path}' not found (active profile: '{profile}')"
                )
            }
            Self::TypeMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "type mismatch at '{path}': expected {expected}, found {actual}"
                )
            }
            Self::Io { path, source } => {
                write!(
                    f,
                    "failed to read configuration file at '{}': {source}",
                    path.display()
                )
            }
            Self::Parse { origin, message } => {
                write!(f, "failed to parse configuration from {origin}: {message}")
            }
            Self::Serde(msg) => write!(f, "deserialization error: {msg}"),
            Self::Store(e) => write!(f, "storage engine error: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<StoreError> for ConfigError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

#[cfg(feature = "serde")]
impl From<statefs_adapter_bridge_serde::DeError> for ConfigError {
    fn from(e: statefs_adapter_bridge_serde::DeError) -> Self {
        Self::Serde(e.to_string())
    }
}

/// An abstract, pluggable configuration provider source that can be stacked.
pub trait ConfigProvider: Send + Sync {
    /// Human-readable origin label for error diagnostics.
    fn origin(&self) -> String;

    /// Ingests configuration nodes into the target store, optionally scoped to a profile and mount prefix.
    fn apply(
        &self,
        store: &mut MemStore,
        profile: &Profile,
        mount_prefix: &str,
    ) -> Result<(), ConfigError>;
}

/// In-memory default values provider.
#[derive(Debug, Clone, Default)]
pub struct Defaults {
    entries: BTreeMap<String, Value>,
}

impl Defaults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(mut self, path: impl Into<String>, value: impl Into<Value>) -> Self {
        self.entries.insert(path.into(), value.into());
        self
    }
}

impl ConfigProvider for Defaults {
    fn origin(&self) -> String {
        "Defaults (in-memory)".to_string()
    }

    fn apply(
        &self,
        store: &mut MemStore,
        _profile: &Profile,
        mount_prefix: &str,
    ) -> Result<(), ConfigError> {
        for (rel_path, val) in &self.entries {
            let full_path = if mount_prefix.is_empty() || mount_prefix == "/" {
                format!("/{}", rel_path.trim_start_matches('/'))
            } else {
                format!(
                    "{}/{}",
                    mount_prefix.trim_end_matches('/'),
                    rel_path.trim_start_matches('/')
                )
            };
            let p = Path::parse(&full_path);
            store.insert(&p, val.clone())?;
        }
        Ok(())
    }
}

/// Raw TOML string or file provider with profile section awareness.
#[cfg(feature = "toml")]
#[derive(Debug, Clone)]
pub struct TomlSource {
    origin_name: String,
    content: String,
}

#[cfg(feature = "toml")]
impl TomlSource {
    pub fn string(content: impl Into<String>) -> Self {
        Self {
            origin_name: "<string.toml>".to_string(),
            content: content.into(),
        }
    }

    pub fn file(path: impl AsRef<StdPath>) -> Result<Self, ConfigError> {
        let path_ref = path.as_ref();
        let content = std::fs::read_to_string(path_ref).map_err(|e| ConfigError::Io {
            path: path_ref.to_path_buf(),
            source: e,
        })?;
        Ok(Self {
            origin_name: path_ref.display().to_string(),
            content,
        })
    }
}

#[cfg(feature = "toml")]
impl ConfigProvider for TomlSource {
    fn origin(&self) -> String {
        self.origin_name.clone()
    }

    fn apply(
        &self,
        store: &mut MemStore,
        profile: &Profile,
        mount_prefix: &str,
    ) -> Result<(), ConfigError> {
        let mut temp_store = MemStore::new();
        statefs_codec_toml::ingest_toml(&mut temp_store, &self.content).map_err(|e| {
            ConfigError::Parse {
                origin: self.origin_name.clone(),
                message: e.to_string(),
            }
        })?;

        // 1. Ingest base / default keys
        let default_profile_prefix = format!("/{}", Profile::DEFAULT);
        let target_profile_prefix = format!("/{}", profile.as_str());

        // Check if TOML has top-level profiles table (even if it's an intermediate directory)
        let has_profiles = temp_store
            .find_arena_index(&default_profile_prefix)
            .map(|idx| idx != 0)
            .unwrap_or(false)
            || temp_store
                .find_arena_index(&target_profile_prefix)
                .map(|idx| idx != 0)
                .unwrap_or(false);

        if has_profiles {
            // First merge default profile
            copy_subtree(&temp_store, &default_profile_prefix, store, mount_prefix)?;
            // Next override with target profile if distinct
            if profile.as_str() != Profile::DEFAULT {
                copy_subtree(&temp_store, &target_profile_prefix, store, mount_prefix)?;
            }
        } else {
            // Flat TOML without profiles: copy everything under root
            copy_subtree(&temp_store, "/", store, mount_prefix)?;
        }

        Ok(())
    }
}

/// Environment variable provider with prefix and separator stripping.
#[cfg(feature = "env")]
#[derive(Debug, Clone)]
pub struct EnvSource {
    inner: statefs_adapter_bridge_env::EnvSource,
    prefix_label: String,
}

#[cfg(feature = "env")]
impl EnvSource {
    pub fn prefixed(prefix: impl Into<String>) -> Self {
        let p = prefix.into();
        Self {
            inner: statefs_adapter_bridge_env::EnvSource::new().with_prefix(&p),
            prefix_label: p,
        }
    }

    pub fn with_separator(mut self, sep: impl Into<String>) -> Self {
        self.inner = self.inner.with_separator(sep);
        self
    }
}

#[cfg(feature = "env")]
impl ConfigProvider for EnvSource {
    fn origin(&self) -> String {
        format!("Env(prefix = \"{}\")", self.prefix_label)
    }

    fn apply(
        &self,
        store: &mut MemStore,
        _profile: &Profile,
        mount_prefix: &str,
    ) -> Result<(), ConfigError> {
        let mut temp_store = MemStore::new();
        self.inner.apply_to_store(&mut temp_store)?;
        copy_subtree(&temp_store, "/", store, mount_prefix)?;
        Ok(())
    }
}

/// Helper function to copy/mount a subtree from a source store into a target store.
fn copy_subtree(
    src: &MemStore,
    src_root: &str,
    dst: &mut MemStore,
    mount_prefix: &str,
) -> Result<(), ConfigError> {
    let src_clean = if src_root == "/" {
        ""
    } else {
        src_root.trim_end_matches('/')
    };
    let glob_query = if src_clean.is_empty() {
        "/**".to_string()
    } else {
        format!("{src_clean}/**")
    };

    for (p, node) in src.find_glob(&glob_query) {
        let path_str = p.to_string();
        let rel_suffix = if src_clean.is_empty() {
            path_str.as_str()
        } else {
            &path_str[src_clean.len()..]
        };

        let target_path_str = if mount_prefix.is_empty() || mount_prefix == "/" {
            format!("/{}", rel_suffix.trim_start_matches('/'))
        } else {
            format!(
                "{}/{}",
                mount_prefix.trim_end_matches('/'),
                rel_suffix.trim_start_matches('/')
            )
        };

        let target_path = Path::parse(&target_path_str);
        dst.insert_node(&target_path, node.clone())?;
    }

    Ok(())
}

/// Modern layered configuration engine combining DX ergonomics with StateFS performance.
pub struct StateFsConfig {
    profile: Profile,
    mount_prefix: String,
    providers: Vec<Box<dyn ConfigProvider>>,
}

impl Default for StateFsConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl StateFsConfig {
    /// Creates a new configuration builder with the `default` profile.
    pub fn new() -> Self {
        Self {
            profile: Profile::default(),
            mount_prefix: "/".to_string(),
            providers: Vec::new(),
        }
    }

    /// Sets the active profile (e.g. `"default"`, `"debug"`, `"competitive"`).
    pub fn profile(mut self, profile: impl Into<Profile>) -> Self {
        self.profile = profile.into();
        self
    }

    /// Sets the mount prefix where configurations will be rooted (e.g. `"/plugins/vip"`).
    pub fn mount(mut self, prefix: impl Into<String>) -> Self {
        self.mount_prefix = prefix.into();
        self
    }

    /// Appends a configuration provider to the stack.
    pub fn merge<P: ConfigProvider + 'static>(mut self, provider: P) -> Self {
        self.providers.push(Box::new(provider));
        self
    }

    /// Appends default values.
    pub fn merge_defaults(self, defaults: Defaults) -> Self {
        self.merge(defaults)
    }

    /// Appends a raw TOML string provider.
    #[cfg(feature = "toml")]
    pub fn merge_toml_str(self, toml_str: impl Into<String>) -> Self {
        self.merge(TomlSource::string(toml_str))
    }

    /// Appends a TOML file provider.
    #[cfg(feature = "toml")]
    pub fn merge_toml_file(self, path: impl AsRef<StdPath>) -> Result<Self, ConfigError> {
        let src = TomlSource::file(path)?;
        Ok(self.merge(src))
    }

    /// Appends an environment variable provider.
    #[cfg(feature = "env")]
    pub fn merge_env(self, prefix: impl Into<String>) -> Self {
        self.merge(EnvSource::prefixed(prefix))
    }

    /// Compiles all stacked providers into an optimized, in-memory [`MemStore`].
    pub fn compile(&self) -> Result<MemStore, ConfigError> {
        let mut store = MemStore::new();
        for provider in &self.providers {
            provider.apply(&mut store, &self.profile, &self.mount_prefix)?;
        }
        Ok(store)
    }

    /// Compiles all providers into a [`ConfigInstance`] offering fast typed queries and Serde extraction.
    pub fn build(&self) -> Result<ConfigInstance, ConfigError> {
        let store = self.compile()?;
        Ok(ConfigInstance {
            store,
            profile: self.profile.clone(),
            mount_prefix: self.mount_prefix.clone(),
        })
    }
}

/// An immutable, compiled configuration instance backed by StateFS.
pub struct ConfigInstance {
    store: MemStore,
    profile: Profile,
    mount_prefix: String,
}

impl ConfigInstance {
    /// Returns the active profile name.
    #[inline]
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Returns a reference to the underlying StateFS [`MemStore`].
    #[inline]
    pub fn store(&self) -> &MemStore {
        &self.store
    }

    /// Consumes this instance and returns the inner [`MemStore`].
    pub fn into_store(self) -> MemStore {
        self.store
    }

    /// Resolves full path taking mount prefix into account.
    fn resolve_path(&self, key: &str) -> String {
        if key.starts_with('/') {
            key.to_string()
        } else if self.mount_prefix == "/" || self.mount_prefix.is_empty() {
            format!("/{key}")
        } else {
            format!("{}/{}", self.mount_prefix.trim_end_matches('/'), key)
        }
    }

    /// Fetches a raw [`Node`] at `key`.
    pub fn get_node(&self, key: &str) -> Option<&Node> {
        let full_path = self.resolve_path(key);
        let p = Path::parse(&full_path);
        self.store.get(&p)
    }

    /// Reads an integer value at `key`.
    pub fn get_int(&self, key: &str) -> Result<i64, ConfigError> {
        let full = self.resolve_path(key);
        let node = self.get_node(key).ok_or_else(|| ConfigError::NotFound {
            path: full.clone(),
            profile: self.profile.to_string(),
        })?;
        node.value
            .as_int()
            .ok_or_else(|| ConfigError::TypeMismatch {
                path: full,
                expected: "integer",
                actual: format!("{:?}", node.value),
            })
    }

    /// Reads a floating-point value at `key`.
    pub fn get_float(&self, key: &str) -> Result<f64, ConfigError> {
        let full = self.resolve_path(key);
        let node = self.get_node(key).ok_or_else(|| ConfigError::NotFound {
            path: full.clone(),
            profile: self.profile.to_string(),
        })?;
        node.value
            .as_float()
            .ok_or_else(|| ConfigError::TypeMismatch {
                path: full,
                expected: "float",
                actual: format!("{:?}", node.value),
            })
    }

    /// Reads a boolean value at `key`.
    pub fn get_bool(&self, key: &str) -> Result<bool, ConfigError> {
        let full = self.resolve_path(key);
        let node = self.get_node(key).ok_or_else(|| ConfigError::NotFound {
            path: full.clone(),
            profile: self.profile.to_string(),
        })?;
        node.value
            .as_bool()
            .ok_or_else(|| ConfigError::TypeMismatch {
                path: full,
                expected: "bool",
                actual: format!("{:?}", node.value),
            })
    }

    /// Reads a string value at `key`.
    pub fn get_string(&self, key: &str) -> Result<String, ConfigError> {
        let full = self.resolve_path(key);
        let node = self.get_node(key).ok_or_else(|| ConfigError::NotFound {
            path: full.clone(),
            profile: self.profile.to_string(),
        })?;
        node.value
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| ConfigError::TypeMismatch {
                path: full,
                expected: "string",
                actual: format!("{:?}", node.value),
            })
    }

    /// Extracts a typed struct from the configuration tree or subtree at `path`.
    #[cfg(feature = "serde")]
    pub fn extract<T: for<'de> serde::Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<T, ConfigError> {
        let full = self.resolve_path(path);
        statefs_adapter_bridge_serde::extract(&self.store, &full).map_err(ConfigError::from)
    }
}

// -------------------------------------------------------------------------------------------------
// Legacy config-rs Compatibility Bridge
// -------------------------------------------------------------------------------------------------

/// High-performance StateFS wrapper matching the API ergonomics of `config::Config`.
pub struct ConfigBridge<const _CAP: usize = 64> {
    store: MemStore,
}

impl ConfigBridge<64> {
    pub fn new(store: MemStore) -> Self {
        Self { store }
    }
}

impl<const CAP: usize> ConfigBridge<CAP> {
    pub fn with_capacity(store: MemStore) -> Self {
        Self { store }
    }

    #[inline(always)]
    pub fn get_int(&mut self, key: &str) -> Result<i64, &'static str> {
        let p = Path::parse(key);
        let node = self.store.get(&p).ok_or("Key not found")?;
        node.value.as_int().ok_or("Value is not an integer")
    }

    #[inline(always)]
    pub fn get_string(&mut self, key: &str) -> Result<String, &'static str> {
        let p = Path::parse(key);
        let node = self.store.get(&p).ok_or("Key not found")?;
        node.value
            .as_str()
            .map(|s| s.to_string())
            .ok_or("Value is not a string")
    }

    #[inline(always)]
    pub fn get_bool(&mut self, key: &str) -> Result<bool, &'static str> {
        let p = Path::parse(key);
        let node = self.store.get(&p).ok_or("Key not found")?;
        node.value.as_bool().ok_or("Value is not a boolean")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stacked_defaults_and_toml_override() {
        let defaults = Defaults::new()
            .set("server/tickrate", 64)
            .set("server/hostname", "Default Server")
            .set("server/max_players", 32);

        let toml_data = r#"
[server]
tickrate = 128
hostname = "HLDS Production"
"#;

        let cfg = StateFsConfig::new()
            .merge_defaults(defaults)
            .merge_toml_str(toml_data)
            .build()
            .unwrap();

        assert_eq!(cfg.get_int("server/tickrate").unwrap(), 128);
        assert_eq!(
            cfg.get_string("server/hostname").unwrap(),
            "HLDS Production"
        );
        assert_eq!(cfg.get_int("server/max_players").unwrap(), 32);
    }

    #[test]
    fn test_profiles_stacking() {
        let toml_data = r#"
[default.server]
tickrate = 64
motd = "Welcome"

[competitive.server]
tickrate = 128
motd = "Match Live"
"#;

        // Test default profile
        let default_cfg = StateFsConfig::new()
            .profile("default")
            .merge_toml_str(toml_data)
            .build()
            .unwrap();
        assert_eq!(default_cfg.get_int("server/tickrate").unwrap(), 64);
        assert_eq!(default_cfg.get_string("server/motd").unwrap(), "Welcome");

        // Test competitive profile
        let comp_cfg = StateFsConfig::new()
            .profile("competitive")
            .merge_toml_str(toml_data)
            .build()
            .unwrap();
        assert_eq!(comp_cfg.get_int("server/tickrate").unwrap(), 128);
        assert_eq!(comp_cfg.get_string("server/motd").unwrap(), "Match Live");
    }

    #[test]
    fn test_subtree_mounting() {
        let toml_data = r#"
bonus_hp = 50
tag = "VIP"
"#;

        let cfg = StateFsConfig::new()
            .mount("/plugins/vip")
            .merge_toml_str(toml_data)
            .build()
            .unwrap();

        assert_eq!(cfg.get_int("bonus_hp").unwrap(), 50);
        assert_eq!(cfg.get_string("tag").unwrap(), "VIP");

        // Verify root node query matches
        assert_eq!(cfg.get_int("/plugins/vip/bonus_hp").unwrap(), 50);
    }

    #[cfg(feature = "serde")]
    #[derive(serde::Deserialize, PartialEq, Debug)]
    struct ServerModel {
        tickrate: i64,
        hostname: String,
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serde_typed_extraction() {
        let toml_data = r#"
[server]
tickrate = 128
hostname = "Cyberzone"
"#;

        let cfg = StateFsConfig::new()
            .merge_toml_str(toml_data)
            .build()
            .unwrap();

        let model: ServerModel = cfg.extract("server").unwrap();
        assert_eq!(
            model,
            ServerModel {
                tickrate: 128,
                hostname: "Cyberzone".to_string()
            }
        );
    }

    #[cfg(feature = "env")]
    #[test]
    fn test_env_source_override() {
        let toml_data = r#"
[server]
port = 27015
"#;
        unsafe {
            std::env::set_var("HLDS_SERVER__PORT", "27020");
        }

        let cfg = StateFsConfig::new()
            .merge_toml_str(toml_data)
            .merge_env("HLDS")
            .build()
            .unwrap();

        assert_eq!(cfg.get_int("server/port").unwrap(), 27020);
    }

    #[test]
    fn test_diagnostics_and_errors() {
        let cfg = StateFsConfig::new()
            .merge_toml_str("flag = true")
            .build()
            .unwrap();

        // 1. NotFound error
        match cfg.get_int("unknown_key") {
            Err(ConfigError::NotFound { path, .. }) => {
                assert!(path.contains("unknown_key"));
            }
            other => panic!("expected NotFound, got {other:?}"),
        }

        // 2. TypeMismatch error
        match cfg.get_int("flag") {
            Err(ConfigError::TypeMismatch { path, expected, .. }) => {
                assert_eq!(path, "/flag");
                assert_eq!(expected, "integer");
            }
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn test_config_bridge_compatibility() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/server/tickrate"), Value::from(128))
            .unwrap();
        store
            .insert(&Path::parse("/server/name"), Value::from("CS 1.6"))
            .unwrap();
        store
            .insert(&Path::parse("/server/active"), Value::from(true))
            .unwrap();

        let mut bridge = ConfigBridge::new(store);
        assert_eq!(bridge.get_int("/server/tickrate"), Ok(128));
        assert_eq!(bridge.get_string("/server/name"), Ok("CS 1.6".to_string()));
        assert_eq!(bridge.get_bool("/server/active"), Ok(true));
    }
}
