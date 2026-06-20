use std::collections::HashSet;

use crate::ids::ProjectId;
use crate::memory::{MemoryRecord, RecallMode, Scope};
use crate::sensitivity::Principal;

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredMemory {
    pub id: String,
    pub score: f32,
    pub explanation: Vec<String>,
}

pub fn recall(
    query: &str,
    principal: &Principal,
    project_id: Option<&ProjectId>,
    memories: &[MemoryRecord],
) -> Vec<ScoredMemory> {
    let query_tokens = tokenize(query);
    let mut scored = Vec::new();

    for memory in memories {
        if memory.tenant_id != principal.tenant_id {
            continue;
        }
        if !is_visible_to_request(memory, principal, project_id) {
            continue;
        }
        if memory.recall_policy.mode == RecallMode::Never {
            continue;
        }
        if !principal.clearance.allows(memory.sensitivity) {
            continue;
        }
        if !memory.recall_policy.allowed_hosts.is_empty()
            && !memory
                .recall_policy
                .allowed_hosts
                .iter()
                .any(|host| host == principal.host.as_str())
        {
            continue;
        }

        let body_tokens = tokenize(&memory.body);
        let overlap = query_tokens.intersection(&body_tokens).count();
        let tag_tokens = tokenize_values(&memory.tags);
        let tag_overlap = query_tokens.intersection(&tag_tokens).count();
        let entity_tokens = tokenize_values(&memory.entities);
        let entity_overlap = query_tokens.intersection(&entity_tokens).count();
        let project_scope_matches = memory.scope == Scope::Project
            && project_id.is_some()
            && memory.project_id.as_ref() == project_id;
        let project_scope_boost = if project_scope_matches { 0.15 } else { 0.0 };
        if overlap == 0 && tag_overlap == 0 && entity_overlap == 0 && !project_scope_matches {
            continue;
        }
        let bm25_norm = overlap as f32 / (overlap as f32 + 3.0);
        let tag_boost = (tag_overlap as f32 * 0.08).min(0.16);
        let entity_boost = (entity_overlap as f32 * 0.06).min(0.12);
        let importance = memory.importance.clamp(0.0, 1.0);
        let confidence = memory.confidence.clamp(0.0, 1.0);
        let score = bm25_norm * 0.58
            + importance * 0.15
            + confidence * 0.08
            + tag_boost
            + entity_boost
            + project_scope_boost;
        scored.push(ScoredMemory {
            id: memory.id.to_string(),
            score,
            explanation: vec![
                format!("token_overlap={overlap}"),
                format!("tag_overlap={tag_overlap}"),
                format!("entity_overlap={entity_overlap}"),
                format!("bm25_norm={bm25_norm:.3}"),
                format!("importance={importance:.3}"),
                format!("confidence={confidence:.3}"),
                format!("project_scope_boost={project_scope_boost:.3}"),
            ],
        });
    }

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored
}

fn is_visible_to_request(
    memory: &MemoryRecord,
    principal: &Principal,
    project_id: Option<&ProjectId>,
) -> bool {
    match memory.scope {
        Scope::User => memory.owner_user_id == principal.user_id,
        Scope::Project => memory.project_id.as_ref() == project_id,
        Scope::Team | Scope::Org => memory.owner_user_id == principal.user_id,
    }
}

fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(ToString::to_string)
        .collect()
}

fn tokenize_values(values: &[String]) -> HashSet<String> {
    values.iter().flat_map(|value| tokenize(value)).collect()
}

pub fn explain_memory(
    query: &str,
    principal: &Principal,
    project_id: Option<&ProjectId>,
    memory: &MemoryRecord,
) -> Option<ScoredMemory> {
    let mut scored = recall(query, principal, project_id, std::slice::from_ref(memory));
    scored.pop().or_else(|| {
        if memory.tenant_id == principal.tenant_id
            && is_visible_to_request(memory, principal, project_id)
            && memory.recall_policy.mode != RecallMode::Never
            && principal.clearance.allows(memory.sensitivity)
        {
            Some(ScoredMemory {
                id: memory.id.to_string(),
                score: 0.0,
                explanation: vec![
                    "token_overlap=0".to_string(),
                    "tag_overlap=0".to_string(),
                    "entity_overlap=0".to_string(),
                    "bm25_norm=0.000".to_string(),
                    "project_scope_boost=0.000".to_string(),
                ],
            })
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemoryId, ProjectId, TenantId, UserId};
    use crate::memory::{MemoryRecord, RecallMode, Scope};
    use crate::sensitivity::{Principal, SensitivityLevel};

    #[test]
    fn recall_filters_never_and_scores_overlap() {
        let principal = Principal::personal("kay", "zode");
        let keep = MemoryRecord::new_user_preference(
            MemoryId::new("mem_keep"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Use Rust for the Noema memory system.",
        );
        let mut hidden = MemoryRecord::new_user_preference(
            MemoryId::new("mem_hidden"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "This sensitive incident is hidden.",
        );
        hidden.recall_policy.mode = RecallMode::Never;

        let results = recall("rust memory", &principal, None, &[hidden, keep]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "mem_keep");
        assert!(results[0].score > 0.0);
        assert!(results[0]
            .explanation
            .iter()
            .any(|line| line.contains("token")));
    }

    #[test]
    fn recall_scores_tags_entities_and_project_context() {
        let principal = Principal::personal("kay", "zode");
        let project_id = ProjectId::new("git_noema");
        let mut project = MemoryRecord::new_user_preference(
            MemoryId::new("mem_project"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Review candidates before persistence.",
        );
        project.scope = Scope::Project;
        project.project_id = Some(project_id.clone());
        project.tags = vec!["review".into(), "rust".into()];
        project.entities = vec!["Noema".into()];

        let results = recall(
            "noema rust review",
            &principal,
            Some(&project_id),
            &[project],
        );
        assert_eq!(results[0].id, "mem_project");
        assert!(results[0]
            .explanation
            .iter()
            .any(|line| line.contains("tag_overlap")));
        assert!(results[0]
            .explanation
            .iter()
            .any(|line| line.contains("project_scope_boost")));
    }

    #[test]
    fn recall_filters_above_principal_clearance() {
        let principal = Principal::personal("kay", "zode");
        let mut confidential = MemoryRecord::new_user_preference(
            MemoryId::new("mem_confidential"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Confidential launch plan uses Rust.",
        );
        confidential.sensitivity = SensitivityLevel::Confidential;

        let results = recall("rust launch", &principal, None, &[confidential]);
        assert!(results.is_empty());
    }

    #[test]
    fn explain_memory_reports_zero_overlap_for_requested_memory() {
        let principal = Principal::personal("kay", "zode");
        let memory = MemoryRecord::new_user_preference(
            MemoryId::new("mem_keep"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Use Rust for Noema.",
        );

        let explained = explain_memory("python packaging", &principal, None, &memory).unwrap();
        assert_eq!(explained.id, "mem_keep");
        assert!(explained.score.abs() < f32::EPSILON);
    }
}
