use serde::{Deserialize, Serialize};

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

pub fn tenant_key(prefix: &str, tenant: &str, suffix: &str) -> String {
    format!(
        "{}/tenants/{}/{}",
        prefix.trim_end_matches('/'),
        tenant,
        suffix.trim_start_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_keys_do_not_cross_tenants() {
        assert_ne!(
            tenant_key("noema", "a", "deep/user/mem.md.zst.enc"),
            tenant_key("noema", "b", "deep/user/mem.md.zst.enc")
        );
    }
}
