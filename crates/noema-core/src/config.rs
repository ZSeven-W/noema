use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::sensitivity::SensitivityLevel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoemaConfig {
    pub tenant: TenantConfig,
    pub policy: PolicyConfig,
    pub sensitive: SensitiveConfig,
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantConfig {
    pub id: String,
    pub mode: TenantMode,
    pub default_user_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TenantMode {
    Personal,
    Enterprise,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    pub write: WritePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WritePolicy {
    Manual,
    Review,
    AutoSafe,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensitiveConfig {
    pub default_sensitivity: SensitivityLevel,
    pub auto_accept_max_sensitivity: SensitivityLevel,
    pub external_llm_max_sensitivity: SensitivityLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub local_root: PathBuf,
}

impl Default for NoemaConfig {
    fn default() -> Self {
        Self {
            tenant: TenantConfig {
                id: "personal".to_string(),
                mode: TenantMode::Personal,
                default_user_id: std::env::var("USER").unwrap_or_else(|_| "user".to_string()),
            },
            policy: PolicyConfig {
                write: WritePolicy::Review,
            },
            sensitive: SensitiveConfig {
                default_sensitivity: SensitivityLevel::Internal,
                auto_accept_max_sensitivity: SensitivityLevel::Internal,
                external_llm_max_sensitivity: SensitivityLevel::Internal,
            },
            storage: StorageConfig {
                local_root: default_root(),
            },
        }
    }
}

impl NoemaConfig {
    pub fn from_toml(input: &str) -> Result<Self> {
        Ok(toml::from_str(input)?)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let text = std::fs::read_to_string(root.as_ref().join("config.toml"))?;
        Self::from_toml(&text)
    }

    pub fn load_or_default() -> Result<Self> {
        let default = Self::default();
        let path = default.storage.local_root.join("config.toml");
        if path.exists() {
            Self::load(&default.storage.local_root)
        } else {
            Ok(default)
        }
    }
}

pub fn default_root() -> PathBuf {
    if let Ok(root) = std::env::var("NOEMA_ROOT") {
        return PathBuf::from(root);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".agent-memory")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_toml_roundtrips() {
        let cfg = NoemaConfig::default();
        let encoded = cfg.to_toml().unwrap();
        let decoded = NoemaConfig::from_toml(&encoded).unwrap();
        assert_eq!(decoded.tenant.id, cfg.tenant.id);
        assert_eq!(decoded.policy.write, cfg.policy.write);
    }

    #[test]
    fn load_reads_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = NoemaConfig::default();
        cfg.tenant.default_user_id = "configured-user".to_string();
        std::fs::write(dir.path().join("config.toml"), cfg.to_toml().unwrap()).unwrap();

        let loaded = NoemaConfig::load(dir.path()).unwrap();
        assert_eq!(loaded.tenant.default_user_id, "configured-user");
    }
}
