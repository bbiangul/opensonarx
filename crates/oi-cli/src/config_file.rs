use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Persistent CLI configuration stored at `~/.opensonarx/config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub auth: AuthSection,
    #[serde(default)]
    pub wallet: WalletSection,
    #[serde(default)]
    pub network: NetworkSection,
    #[serde(default)]
    pub search: SearchSection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthSection {
    pub api_key: Option<String>,
    pub gateway_url: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WalletSection {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSection {
    #[serde(default = "default_network")]
    pub default: String,
    pub rpc_url: Option<String>,
    #[serde(default)]
    pub bootstrap_peers: Vec<String>,
}

fn default_network() -> String {
    "mainnet".to_string()
}

impl Default for NetworkSection {
    fn default() -> Self {
        Self {
            default: default_network(),
            rpc_url: None,
            bootstrap_peers: Vec::new(),
        }
    }
}

/// Search engine configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchSection {
    /// Configured search engines with priorities.
    #[serde(default)]
    pub engines: Vec<oi_sdk::config::SearchEngineConfig>,
}

impl ConfigFile {
    /// Directory for OpenSonarX state.
    ///
    /// `OPENSONARX_DATA_DIR` provides an explicit deployment/test boundary;
    /// otherwise the canonical `~/.opensonarx/` location is used.
    pub fn config_dir() -> Option<PathBuf> {
        resolve_config_dir(std::env::var_os("OPENSONARX_DATA_DIR"), dirs::home_dir())
    }

    /// Full path to the config file: `~/.opensonarx/config.toml`
    pub fn config_path() -> Option<PathBuf> {
        Self::config_dir().map(|d| d.join("config.toml"))
    }

    /// Canonical local index directory: `~/.opensonarx/index`.
    ///
    /// Every component — `oi search`, `oi seed`, the sentinel daemon, the P2P
    /// node, and `oi index import/export` — reads and writes this one path so
    /// content ingested anywhere is immediately searchable everywhere. If the
    /// canonical dir has no index yet but a pre-unification legacy dir
    /// (`node_index`, `sentinel_index`) does, the legacy contents are migrated
    /// (renamed) into place on first access.
    pub fn index_dir() -> Option<PathBuf> {
        let canonical = Self::config_dir()?.join("index");
        let has_index = |p: &PathBuf| p.join("hnsw.bin").exists();
        if !has_index(&canonical) {
            for legacy_name in ["node_index", "sentinel_index"] {
                let legacy = Self::config_dir()?.join(legacy_name);
                if has_index(&legacy) {
                    // Canonical may exist as an empty dir; clear it so rename succeeds.
                    let _ = std::fs::remove_dir(&canonical);
                    if std::fs::rename(&legacy, &canonical).is_ok() {
                        eprintln!(
                            "Migrated legacy index {} -> {}",
                            legacy.display(),
                            canonical.display()
                        );
                        break;
                    }
                }
            }
        }
        let _ = std::fs::create_dir_all(&canonical);
        Some(canonical)
    }

    /// Load config from disk. Returns default if file doesn't exist.
    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save config to disk. Creates the directory if needed.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(dir) = Self::config_dir() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Cannot determine home directory",
            ));
        };
        std::fs::create_dir_all(&dir)?;
        let content = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join("config.toml"), content)
    }
}

fn resolve_config_dir(
    override_dir: Option<std::ffi::OsString>,
    home_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    override_dir
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir.map(|home| home.join(".opensonarx")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let config = ConfigFile {
            auth: AuthSection {
                api_key: Some("oik_test123".into()),
                gateway_url: Some("https://api.opensonarx.com".into()),
                email: Some("dev@example.com".into()),
            },
            wallet: WalletSection {
                path: Some("/path/to/key.json".into()),
            },
            network: NetworkSection {
                default: "devnet".into(),
                rpc_url: Some("https://api.devnet.solana.com".into()),
                bootstrap_peers: vec!["/ip4/1.2.3.4/tcp/4001".into()],
            },
            search: SearchSection::default(),
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let decoded: ConfigFile = toml::from_str(&toml_str).unwrap();

        assert_eq!(decoded.auth.api_key.as_deref(), Some("oik_test123"));
        assert_eq!(decoded.auth.email.as_deref(), Some("dev@example.com"));
        assert_eq!(decoded.wallet.path.as_deref(), Some("/path/to/key.json"));
        assert_eq!(decoded.network.default, "devnet");
        assert_eq!(decoded.network.bootstrap_peers.len(), 1);
    }

    #[test]
    fn test_default_deserialization() {
        let config: ConfigFile = toml::from_str("").unwrap();
        assert!(config.auth.api_key.is_none());
        assert!(config.wallet.path.is_none());
        assert_eq!(config.network.default, "mainnet");
    }

    #[test]
    fn test_partial_config() {
        let toml_str = r#"
[auth]
email = "test@test.com"
"#;
        let config: ConfigFile = toml::from_str(toml_str).unwrap();
        assert_eq!(config.auth.email.as_deref(), Some("test@test.com"));
        assert!(config.auth.api_key.is_none());
        assert!(config.wallet.path.is_none());
    }

    #[test]
    fn test_explicit_data_dir_isolated_from_home() {
        let resolved = resolve_config_dir(
            Some(std::ffi::OsString::from("/tmp/opensonarx-e2e")),
            Some(PathBuf::from("/users/example")),
        );
        assert_eq!(resolved, Some(PathBuf::from("/tmp/opensonarx-e2e")));
    }

    #[test]
    fn test_empty_data_dir_falls_back_to_home() {
        let resolved = resolve_config_dir(
            Some(std::ffi::OsString::new()),
            Some(PathBuf::from("/users/example")),
        );
        assert_eq!(resolved, Some(PathBuf::from("/users/example/.opensonarx")));
    }
}
