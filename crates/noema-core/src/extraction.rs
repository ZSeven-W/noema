use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::Result;
use crate::ids::{TenantId, UserId};
use crate::jsonl::{append_jsonl_locked, read_jsonl};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionJob {
    pub id: String,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub session_id: String,
    pub range: TranscriptRange,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl ExtractionJob {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        session_id: impl Into<String>,
        range: TranscriptRange,
    ) -> Self {
        Self {
            id: format!("xjob_{}", Uuid::new_v4()),
            tenant_id,
            user_id,
            session_id: session_id.into(),
            range,
            created_at: OffsetDateTime::now_utc(),
        }
    }
}

pub fn append_job(root: &std::path::Path, job: &ExtractionJob) -> Result<()> {
    let path = root
        .join("tenants")
        .join(job.tenant_id.as_str())
        .join("extraction/jobs.jsonl");
    append_jsonl_locked(&path.with_extension("lock"), &path, job)
}

pub fn load_jobs(root: &std::path::Path, tenant: &TenantId) -> Result<Vec<ExtractionJob>> {
    let path = root
        .join("tenants")
        .join(tenant.as_str())
        .join("extraction/jobs.jsonl");
    read_jsonl(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extraction_jobs_roundtrip_without_transcript_body() {
        let dir = tempfile::tempdir().unwrap();
        let job = ExtractionJob::new(
            TenantId::new("personal"),
            UserId::new("kay"),
            "session_1",
            TranscriptRange { start: 10, end: 20 },
        );
        append_job(dir.path(), &job).unwrap();
        let jobs = load_jobs(dir.path(), &TenantId::new("personal")).unwrap();
        assert_eq!(jobs.len(), 1);
        let encoded = serde_json::to_string(&jobs[0]).unwrap();
        assert!(!encoded.contains("raw transcript"));
    }
}
