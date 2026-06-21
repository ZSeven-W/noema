use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ids::{CandidateId, MemoryId, TenantId, UserId};
use crate::memory::Scope;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    CandidateQueued,
    CandidateAutoAccepted,
    CandidateRejectedSecret,
    CandidateAccepted,
    CandidateRejected,
    CandidateEdited,
    CandidateMerged,
    MemoryWritten,
    MemoryTombstoned,
    VacuumCompacted,
    PolicyChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: String,
    pub tenant_id: TenantId,
    pub actor_user_id: UserId,
    pub scope: Scope,
    pub action: AuditAction,
    #[serde(default)]
    pub memory_id: Option<MemoryId>,
    #[serde(default)]
    pub candidate_id: Option<CandidateId>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    #[serde(default)]
    pub reason: Option<String>,
}

impl AuditEvent {
    pub fn new(
        tenant_id: TenantId,
        actor_user_id: UserId,
        scope: Scope,
        action: AuditAction,
    ) -> Self {
        Self {
            id: format!("audit_{}", Uuid::new_v4()),
            tenant_id,
            actor_user_id,
            scope,
            action,
            memory_id: None,
            candidate_id: None,
            timestamp: OffsetDateTime::now_utc(),
            reason: None,
        }
    }
}

use std::path::Path;

use crate::error::Result;
use crate::jsonl::{append_jsonl_locked, read_jsonl};

pub fn append_audit(tenant_dir: &Path, event: &AuditEvent) -> Result<()> {
    let day = event.timestamp.date();
    let path = tenant_dir.join("audit").join(format!(
        "{:04}-{:02}-{:02}.jsonl",
        day.year(),
        u8::from(day.month()),
        day.day()
    ));
    append_jsonl_locked(&tenant_dir.join("audit/audit.lock"), &path, event)
}

pub fn load_audit(path: &Path) -> Result<Vec<AuditEvent>> {
    read_jsonl(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_event_contains_no_body_text() {
        let event = AuditEvent::new(
            TenantId::new("personal"),
            UserId::new("kay"),
            Scope::User,
            AuditAction::CandidateQueued,
        );
        let encoded = serde_json::to_string(&event).unwrap();
        assert!(!encoded.contains("Prefer Rust for Noema"));
        assert!(!encoded.contains("body"));
    }

    #[test]
    fn append_audit_writes_jsonl_without_body() {
        let dir = tempfile::tempdir().unwrap();
        let event = AuditEvent::new(
            TenantId::new("personal"),
            UserId::new("kay"),
            Scope::User,
            AuditAction::CandidateQueued,
        );

        append_audit(dir.path(), &event).unwrap();

        let entries: Vec<AuditEvent> = load_audit(&dir.path().join("audit").join(format!(
            "{:04}-{:02}-{:02}.jsonl",
            event.timestamp.date().year(),
            u8::from(event.timestamp.date().month()),
            event.timestamp.date().day()
        )))
        .unwrap();
        assert_eq!(entries.len(), 1);
    }
}
