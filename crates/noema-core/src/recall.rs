use std::collections::HashSet;

use crate::ids::ProjectId;
use crate::memory::{AccessLevel, MemoryRecord, MemoryStatus, RecallMode, Scope};
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
        if memory.status != MemoryStatus::Active {
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

        let entity_tokens = tokenize_values(&memory.entities);
        let body_tokens = tokenize(&memory.body);
        let overlap = query_tokens
            .iter()
            .filter(|token| body_tokens.contains(*token) && !entity_tokens.contains(*token))
            .count();
        let tag_tokens = tokenize_values(&memory.tags);
        let tag_overlap = query_tokens.intersection(&tag_tokens).count();
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
        let summary_boost = if entity_overlap > 0 && is_summary_memory(memory) {
            0.12
        } else {
            0.0
        };
        let importance = memory.importance.clamp(0.0, 1.0);
        let confidence = memory.confidence.clamp(0.0, 1.0);
        let score = bm25_norm * 0.58
            + importance * 0.15
            + confidence * 0.08
            + tag_boost
            + entity_boost
            + summary_boost
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
                format!("summary_boost={summary_boost:.3}"),
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
        Scope::Project => {
            project_id.is_some()
                && memory.project_id.as_ref() == project_id
                && (memory.owner_user_id == principal.user_id || acl_grants_read(memory, principal))
        }
        Scope::Team | Scope::Org => memory.owner_user_id == principal.user_id,
    }
}

/// Returns true when an ACL entry grants the principal (by user id or group)
/// at least read access (Read, Write, or Admin).
fn acl_grants_read(memory: &MemoryRecord, principal: &Principal) -> bool {
    memory.acl.iter().any(|entry| {
        let grants_access = matches!(
            entry.access,
            AccessLevel::Read | AccessLevel::Write | AccessLevel::Admin
        );
        let matches_principal = entry.principal == principal.user_id.as_str()
            || principal
                .groups
                .iter()
                .any(|group| entry.principal == group.as_str());
        grants_access && matches_principal
    })
}

fn is_summary_memory(memory: &MemoryRecord) -> bool {
    memory
        .tags
        .iter()
        .any(|tag| tag == "summary" || tag == "fact-layer" || tag == "episode")
}

fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .filter(|token| !is_stopword(token))
        .map(ToString::to_string)
        .collect()
}

fn tokenize_values(values: &[String]) -> HashSet<String> {
    values.iter().flat_map(|value| tokenize(value)).collect()
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "about"
            | "after"
            | "again"
            | "all"
            | "also"
            | "and"
            | "any"
            | "are"
            | "because"
            | "been"
            | "before"
            | "being"
            | "both"
            | "but"
            | "can"
            | "could"
            | "did"
            | "does"
            | "during"
            | "each"
            | "few"
            | "for"
            | "from"
            | "had"
            | "has"
            | "have"
            | "her"
            | "here"
            | "hers"
            | "him"
            | "his"
            | "how"
            | "into"
            | "its"
            | "just"
            | "more"
            | "most"
            | "nor"
            | "not"
            | "now"
            | "off"
            | "once"
            | "only"
            | "other"
            | "our"
            | "ours"
            | "out"
            | "over"
            | "own"
            | "same"
            | "she"
            | "should"
            | "some"
            | "such"
            | "than"
            | "that"
            | "the"
            | "their"
            | "theirs"
            | "them"
            | "then"
            | "there"
            | "they"
            | "this"
            | "through"
            | "too"
            | "under"
            | "until"
            | "very"
            | "was"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "whom"
            | "why"
            | "will"
            | "with"
            | "you"
            | "your"
            | "yours"
    )
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
    use crate::ids::{GroupId, MemoryId, ProjectId, TenantId, UserId};
    use crate::memory::{AccessLevel, AclEntry, MemoryRecord, MemoryStatus, RecallMode, Scope};
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
    fn recall_does_not_double_count_entity_tokens_as_body_overlap() {
        let principal = Principal::personal("kay", "zode");
        let mut memory = MemoryRecord::new_user_preference(
            MemoryId::new("mem_kay"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Kay prefers tea after dinner.",
        );
        memory.entities = vec!["Kay".to_string()];

        let explained = explain_memory("Kay Rust", &principal, None, &memory).unwrap();

        assert!(explained
            .explanation
            .iter()
            .any(|line| line == "token_overlap=0"));
        assert!(explained
            .explanation
            .iter()
            .any(|line| line == "entity_overlap=1"));
    }

    #[test]
    fn recall_prefers_summary_memory_for_entity_only_queries() {
        let principal = Principal::personal("kay", "zode");
        let mut ordinary = MemoryRecord::new_user_preference(
            MemoryId::new("mem_ordinary"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Kay mentioned a routine detail.",
        );
        ordinary.entities = vec!["Kay".to_string()];
        let mut summary = MemoryRecord::new_user_preference(
            MemoryId::new("mem_summary"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Kay has a compressed long-term profile.",
        );
        summary.entities = vec!["Kay".to_string()];
        summary.tags = vec!["summary".to_string()];

        let results = recall("Kay preference", &principal, None, &[ordinary, summary]);

        assert_eq!(results[0].id, "mem_summary");
        assert!(results[0]
            .explanation
            .iter()
            .any(|line| line == "summary_boost=0.120"));
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

    #[test]
    fn recall_excludes_tombstoned_memories() {
        let principal = Principal::personal("kay", "zode");
        let mut tombstoned = MemoryRecord::new_user_preference(
            MemoryId::new("mem_tombstoned"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Use Rust for the Noema memory system.",
        );
        tombstoned.status = MemoryStatus::Tombstoned;

        let results = recall("rust memory", &principal, None, &[tombstoned]);
        assert!(results.is_empty());
    }

    fn project_memory_owned_by_other(project_id: &ProjectId) -> MemoryRecord {
        let mut memory = MemoryRecord::new_user_preference(
            MemoryId::new("mem_project_other"),
            TenantId::new("personal"),
            UserId::new("other"),
            "Review candidates before persistence.",
        );
        memory.scope = Scope::Project;
        memory.project_id = Some(project_id.clone());
        memory.tags = vec!["review".into(), "rust".into()];
        memory.entities = vec!["Noema".into()];
        memory
    }

    #[test]
    fn recall_hides_project_memory_owned_by_other_without_acl() {
        let principal = Principal::personal("kay", "zode");
        let project_id = ProjectId::new("git_noema");
        let memory = project_memory_owned_by_other(&project_id);

        let results = recall(
            "noema rust review",
            &principal,
            Some(&project_id),
            &[memory],
        );
        assert!(results.is_empty());
    }

    #[test]
    fn recall_shows_project_memory_owned_by_other_with_acl_read() {
        let principal = Principal::personal("kay", "zode");
        let project_id = ProjectId::new("git_noema");
        let mut memory = project_memory_owned_by_other(&project_id);
        memory.acl = vec![AclEntry {
            principal: "kay".to_string(),
            access: AccessLevel::Read,
        }];

        let results = recall(
            "noema rust review",
            &principal,
            Some(&project_id),
            &[memory],
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "mem_project_other");
    }

    #[test]
    fn recall_shows_project_memory_via_group_acl() {
        let mut principal = Principal::personal("kay", "zode");
        principal.groups = vec![GroupId::new("team_core")];
        let project_id = ProjectId::new("git_noema");
        let mut memory = project_memory_owned_by_other(&project_id);
        memory.acl = vec![AclEntry {
            principal: "team_core".to_string(),
            access: AccessLevel::Write,
        }];

        let results = recall(
            "noema rust review",
            &principal,
            Some(&project_id),
            &[memory],
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "mem_project_other");
    }

    #[test]
    fn recall_hides_project_memory_with_no_project_id() {
        let principal = Principal::personal("kay", "zode");
        let mut memory = MemoryRecord::new_user_preference(
            MemoryId::new("mem_project_none"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Review candidates before persistence.",
        );
        memory.scope = Scope::Project;
        memory.project_id = None;
        memory.tags = vec!["review".into(), "rust".into()];
        memory.entities = vec!["Noema".into()];

        let results = recall("noema rust review", &principal, None, &[memory]);
        assert!(results.is_empty());
    }

    #[test]
    fn recall_ignores_stopword_only_overlap() {
        let principal = Principal::personal("kay", "zode");
        let noisy = MemoryRecord::new_user_preference(
            MemoryId::new("mem_noisy"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "What has happened there?",
        );
        let content = MemoryRecord::new_user_preference(
            MemoryId::new("mem_content"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Caroline attended a Pride march.",
        );

        let results = recall(
            "What has Caroline attended there?",
            &principal,
            None,
            &[noisy, content],
        );

        assert_eq!(results[0].id, "mem_content");
    }
}
