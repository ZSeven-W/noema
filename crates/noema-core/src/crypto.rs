use serde::{Deserialize, Serialize};

use crate::sensitivity::SensitivityLevel;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KmsPolicy {
    pub tenant_id: String,
    pub kms_key_id: Option<String>,
}

impl KmsPolicy {
    pub fn allows_s3_write(&self, sensitivity: SensitivityLevel) -> bool {
        match sensitivity {
            SensitivityLevel::Public | SensitivityLevel::Internal => true,
            SensitivityLevel::Confidential
            | SensitivityLevel::Restricted
            | SensitivityLevel::Secret => self.kms_key_id.is_some(),
        }
    }
}
