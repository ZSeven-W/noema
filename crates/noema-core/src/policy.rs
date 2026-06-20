use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AclDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterprisePolicy {
    pub org_memory_requires_review_role: bool,
    pub reviewer_roles: Vec<String>,
}

impl Default for EnterprisePolicy {
    fn default() -> Self {
        Self {
            org_memory_requires_review_role: true,
            reviewer_roles: vec!["reviewer".to_string(), "admin".to_string()],
        }
    }
}

impl EnterprisePolicy {
    pub fn can_write_org_memory(&self, roles: &[String]) -> AclDecision {
        if !self.org_memory_requires_review_role {
            return AclDecision::Allow;
        }
        if roles
            .iter()
            .any(|role| self.reviewer_roles.iter().any(|allowed| allowed == role))
        {
            AclDecision::Allow
        } else {
            AclDecision::Deny
        }
    }
}
