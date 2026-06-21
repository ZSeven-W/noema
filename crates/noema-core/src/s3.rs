use serde::{Deserialize, Serialize};

use crate::error::NoemaError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Config {
    pub bucket: String,
    pub prefix: String,
    pub endpoint: Option<String>,
    pub region: String,
    pub access_key_env: String,
    pub secret_key_env: String,
    pub encryption: S3Encryption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum S3Encryption {
    Off,
    SseS3,
    SseKms,
}

pub fn tenant_key(prefix: &str, tenant: &str, suffix: &str) -> crate::error::Result<String> {
    if tenant.is_empty() || tenant.contains('/') || tenant.contains('\\') || tenant.contains("..") {
        return Err(NoemaError::PolicyDenied(format!(
            "invalid tenant: {tenant:?}"
        )));
    }
    if std::path::Path::new(suffix)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(NoemaError::PolicyDenied(format!(
            "suffix escapes tenant: {suffix:?}"
        )));
    }
    Ok(format!(
        "{}/tenants/{}/{}",
        prefix.trim_end_matches('/'),
        tenant,
        suffix.trim_start_matches('/')
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_keys_do_not_cross_tenants() {
        assert_ne!(
            tenant_key("noema", "a", "deep/user/mem.md.zst.enc").unwrap(),
            tenant_key("noema", "b", "deep/user/mem.md.zst.enc").unwrap()
        );
    }

    #[test]
    fn parent_dir_tenant_is_rejected() {
        assert!(matches!(
            tenant_key("noema", "..", "deep/user/mem.md.zst.enc"),
            Err(NoemaError::PolicyDenied(_))
        ));
    }
}
