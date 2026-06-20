use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{CandidateId, ProjectId, TeamId, TenantId, UserId};
use crate::memory::{MemoryKind, MemorySource, Scope, SCHEMA_VERSION};
use crate::sensitivity::{DataClass, SensitivityLevel};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub schema_version: u32,
    pub id: CandidateId,
    pub tenant_id: TenantId,
    pub owner_user_id: UserId,
    pub scope: Scope,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub team_id: Option<TeamId>,
    pub kind: MemoryKind,
    pub body: String,
    pub confidence: f32,
    pub importance: f32,
    pub novelty: f32,
    pub sensitivity: SensitivityLevel,
    #[serde(default)]
    pub data_classes: Vec<DataClass>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub source: MemorySource,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl Candidate {
    pub fn new(id: CandidateId, body: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id,
            tenant_id: TenantId::new("personal"),
            owner_user_id: UserId::new(
                std::env::var("USER").unwrap_or_else(|_| "user".to_string()),
            ),
            scope: Scope::User,
            project_id: None,
            team_id: None,
            kind: MemoryKind::Preference,
            body: body.into(),
            confidence: 1.0,
            importance: 0.5,
            novelty: 1.0,
            sensitivity: SensitivityLevel::Internal,
            data_classes: Vec::new(),
            tags: Vec::new(),
            entities: Vec::new(),
            source: MemorySource {
                kind: "manual".to_string(),
                agent: "noema-cli".to_string(),
                uri: None,
            },
            created_at: OffsetDateTime::now_utc(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Accept {
        candidate_id: CandidateId,
    },
    Reject {
        candidate_id: CandidateId,
        reason: String,
    },
    Edit {
        candidate_id: CandidateId,
        body: String,
        reason: String,
    },
    Merge {
        candidate_id: CandidateId,
        target_memory_id: crate::ids::MemoryId,
        reason: String,
    },
}

// Accept, reject, and merge decisions are candidate tombstones in P0. Rejected
// and merged candidates stay in the append-only inbox history but are removed
// from the live pending view. Edit decisions are non-terminal and rewrite the
// pending view for the next reviewer action.
pub fn pending_candidates(
    candidates: &[Candidate],
    decisions: &[ReviewDecision],
) -> Vec<Candidate> {
    let terminal: std::collections::HashSet<CandidateId> = decisions
        .iter()
        .filter_map(|decision| match decision {
            ReviewDecision::Accept { candidate_id }
            | ReviewDecision::Reject { candidate_id, .. }
            | ReviewDecision::Merge { candidate_id, .. } => Some(candidate_id.clone()),
            ReviewDecision::Edit { .. } => None,
        })
        .collect();
    let edits: std::collections::HashMap<CandidateId, String> = decisions
        .iter()
        .filter_map(|decision| match decision {
            ReviewDecision::Edit {
                candidate_id, body, ..
            } => Some((candidate_id.clone(), body.clone())),
            _ => None,
        })
        .collect();

    candidates
        .iter()
        .filter(|candidate| !terminal.contains(&candidate.id))
        .map(|candidate| {
            let mut candidate = candidate.clone();
            if let Some(body) = edits.get(&candidate.id) {
                candidate.body = body.clone();
            }
            candidate
        })
        .collect()
}

use std::path::Path;

use crate::error::Result;
use crate::jsonl::{append_jsonl_locked, read_jsonl};

pub fn append_candidate(path: &Path, candidate: &Candidate) -> Result<()> {
    append_jsonl_locked(&path.with_file_name("inbox.lock"), path, candidate)
}

pub fn append_decision(path: &Path, decision: &ReviewDecision) -> Result<()> {
    append_jsonl_locked(&path.with_file_name("decisions.lock"), path, decision)
}

pub fn load_candidates(path: &Path) -> Result<Vec<Candidate>> {
    read_jsonl(path)
}

pub fn load_decisions(path: &Path) -> Result<Vec<ReviewDecision>> {
    read_jsonl(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_view_excludes_terminal_decisions() {
        let a = Candidate::new(CandidateId::new("cand_a"), "A");
        let b = Candidate::new(CandidateId::new("cand_b"), "B");
        let pending = pending_candidates(
            &[a.clone(), b.clone()],
            &[ReviewDecision::Reject {
                candidate_id: a.id.clone(),
                reason: "wrong".into(),
            }],
        );
        assert_eq!(pending, vec![b]);
    }

    #[test]
    fn pending_view_applies_edit_decisions() {
        let a = Candidate::new(CandidateId::new("cand_a"), "A");
        let pending = pending_candidates(
            &[a],
            &[ReviewDecision::Edit {
                candidate_id: CandidateId::new("cand_a"),
                body: "Edited".into(),
                reason: "tighten wording".into(),
            }],
        );
        assert_eq!(pending[0].body, "Edited");
    }

    #[test]
    fn pending_view_excludes_merged_candidates() {
        let a = Candidate::new(CandidateId::new("cand_a"), "A");
        let pending = pending_candidates(
            &[a],
            &[ReviewDecision::Merge {
                candidate_id: CandidateId::new("cand_a"),
                target_memory_id: crate::ids::MemoryId::new("mem_existing"),
                reason: "duplicate".into(),
            }],
        );
        assert!(pending.is_empty());
    }
}
