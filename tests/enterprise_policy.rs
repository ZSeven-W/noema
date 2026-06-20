use noema_core::crypto::KmsPolicy;
use noema_core::policy::{AclDecision, EnterprisePolicy};
use noema_core::sensitivity::SensitivityLevel;

#[test]
fn confidential_s3_requires_kms_key() {
    let policy = KmsPolicy {
        tenant_id: "acme".to_string(),
        kms_key_id: Some("arn:aws:kms:us-east-1:123:key/acme".to_string()),
    };
    assert!(policy.allows_s3_write(SensitivityLevel::Confidential));
}

#[test]
fn org_memory_requires_reviewer_role() {
    let policy = EnterprisePolicy::default();
    let decision = policy.can_write_org_memory(&["developer".to_string()]);
    assert_eq!(decision, AclDecision::Deny);
}
