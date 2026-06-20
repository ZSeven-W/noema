use serde::{Deserialize, Serialize};

use crate::ids::{GroupId, HostId, TenantId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityLevel {
    Public,
    Internal,
    Confidential,
    Restricted,
    Secret,
}

impl SensitivityLevel {
    pub fn allows(self, required: SensitivityLevel) -> bool {
        self >= required
    }

    pub fn can_auto_accept(self) -> bool {
        matches!(self, Self::Public | Self::Internal)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    SourceCode,
    Architecture,
    CustomerData,
    Pii,
    Security,
    Credential,
    Legal,
    Finance,
    Health,
    Hr,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataClassClearance {
    pub data_class: DataClass,
    pub level: SensitivityLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default)]
    pub groups: Vec<GroupId>,
    pub host: HostId,
    #[serde(default)]
    pub roles: Vec<String>,
    pub clearance: SensitivityLevel,
    #[serde(default)]
    pub data_class_clearances: Vec<DataClassClearance>,
}

impl Principal {
    pub fn personal(user_id: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            tenant_id: TenantId::new("personal"),
            user_id: UserId::new(user_id),
            groups: Vec::new(),
            host: HostId::new(host),
            roles: vec!["owner".to_string()],
            clearance: SensitivityLevel::Internal,
            data_class_clearances: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearance_is_a_ceiling() {
        assert!(SensitivityLevel::Internal.allows(SensitivityLevel::Public));
        assert!(SensitivityLevel::Confidential.allows(SensitivityLevel::Internal));
        assert!(!SensitivityLevel::Internal.allows(SensitivityLevel::Confidential));
        assert!(!SensitivityLevel::Restricted.allows(SensitivityLevel::Secret));
    }

    #[test]
    fn secret_is_never_auto_accepted() {
        assert!(!SensitivityLevel::Secret.can_auto_accept());
        assert!(!SensitivityLevel::Restricted.can_auto_accept());
        assert!(!SensitivityLevel::Confidential.can_auto_accept());
        assert!(SensitivityLevel::Internal.can_auto_accept());
        assert!(SensitivityLevel::Public.can_auto_accept());
    }
}
