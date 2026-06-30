use std::collections::{HashMap, HashSet};

use crate::ids::ProjectId;
use crate::memory::{AccessLevel, MemoryRecord, MemoryStatus, RecallMode, Scope};
use crate::sensitivity::Principal;
use crate::text;

/// BM25 term-frequency saturation.
const BM25_K1: f32 = 1.2;
/// BM25 length-normalization strength.
const BM25_B: f32 = 0.75;
/// Maps the unbounded BM25 sum into `[0, 1)` so it composes with the additive
/// importance / tag / entity boosts below.
const BM25_SATURATION: f32 = 4.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredMemory {
    pub id: String,
    pub score: f32,
    pub explanation: Vec<String>,
}

/// Per-document tokenization plus the corpus statistics BM25 needs (document
/// frequency and average document length), computed once over the recallable
/// set so scoring does not re-tokenize every body per query term.
struct Bm25Corpus {
    n: f32,
    avgdl: f32,
    df: HashMap<String, u32>,
    docs: Vec<Bm25Doc>,
}

struct Bm25Doc {
    counts: HashMap<String, u32>,
    len: f32,
}

impl Bm25Corpus {
    fn build(memories: &[&MemoryRecord]) -> Self {
        let mut df: HashMap<String, u32> = HashMap::new();
        let mut docs = Vec::with_capacity(memories.len());
        let mut total_len: u64 = 0;
        for memory in memories {
            let counts = text::term_counts(&memory.body);
            let len: u32 = counts.values().sum();
            total_len += len as u64;
            for token in counts.keys() {
                *df.entry(token.clone()).or_insert(0) += 1;
            }
            docs.push(Bm25Doc {
                counts,
                len: len as f32,
            });
        }
        let n = memories.len();
        let avgdl = if n == 0 {
            1.0
        } else {
            (total_len as f32 / n as f32).max(1.0)
        };
        Self {
            n: n as f32,
            avgdl,
            df,
            docs,
        }
    }

    fn idf(&self, token: &str) -> f32 {
        let df = *self.df.get(token).unwrap_or(&0) as f32;
        // BM25 idf with +1 inside the log so it stays non-negative even for a
        // term present in nearly every document.
        ((self.n - df + 0.5) / (df + 0.5) + 1.0).ln()
    }

    /// `b` is the length-normalization strength for this document. Compiled
    /// summary / fact-layer pages pass `b == 0`: their length is curated
    /// coverage (a rollup of many facts), so penalizing it would push the very
    /// pages that collapse multi-hop recall below the raw fragments they
    /// summarize — the opposite of what a wiki-style memory wants.
    fn term_score(&self, token: &str, tf: f32, doc_len: f32, b: f32) -> f32 {
        let denom = tf + BM25_K1 * (1.0 - b + b * doc_len / self.avgdl);
        self.idf(token) * (tf * (BM25_K1 + 1.0)) / denom
    }
}

pub fn recall(
    query: &str,
    principal: &Principal,
    project_id: Option<&ProjectId>,
    memories: &[MemoryRecord],
) -> Vec<ScoredMemory> {
    let query_tokens = query_tokens_with_aliases(query, principal, project_id, memories);

    let recallable: Vec<&MemoryRecord> = memories
        .iter()
        .filter(|memory| is_recallable_by_request(memory, principal, project_id))
        .collect();
    let corpus = Bm25Corpus::build(&recallable);

    let mut scored = Vec::new();
    for (memory, doc) in recallable.iter().zip(corpus.docs.iter()) {
        let entity_tokens = text::tokenize_values(&memory.entities);

        // BM25 over the body, but skip query terms that are this memory's own
        // entity tokens — those are credited via entity_overlap so a single
        // mention is not double-counted (preserves the original semantics).
        let length_norm = if is_summary_memory(memory) {
            0.0
        } else {
            BM25_B
        };
        let mut bm25_sum = 0.0f32;
        let mut token_overlap = 0usize;
        for token in &query_tokens {
            if entity_tokens.contains(token) {
                continue;
            }
            if let Some(&tf) = doc.counts.get(token) {
                token_overlap += 1;
                bm25_sum += corpus.term_score(token, tf as f32, doc.len, length_norm);
            }
        }

        let tag_tokens = text::tokenize_values(&memory.tags);
        let tag_overlap = query_tokens.intersection(&tag_tokens).count();
        let entity_overlap = query_tokens.intersection(&entity_tokens).count();
        let project_scope_matches = memory.scope == Scope::Project
            && project_id.is_some()
            && memory.project_id.as_ref() == project_id;
        let project_scope_boost = if project_scope_matches { 0.15 } else { 0.0 };
        if token_overlap == 0 && tag_overlap == 0 && entity_overlap == 0 && !project_scope_matches {
            continue;
        }
        let bm25_norm = bm25_sum / (bm25_sum + BM25_SATURATION);
        let tag_boost = (tag_overlap as f32 * 0.08).min(0.16);
        let entity_boost = (entity_overlap as f32 * 0.06).min(0.12);
        let summary_boost = if entity_overlap > 0 && is_summary_memory(memory) {
            0.12
        } else {
            0.0
        };
        let importance = memory.importance.clamp(0.0, 1.0);
        let confidence = memory.confidence.clamp(0.0, 1.0);
        let recency_boost = recency_boost(memory);
        let score = bm25_norm * 0.58
            + importance * 0.15
            + confidence * 0.08
            + tag_boost
            + entity_boost
            + summary_boost
            + project_scope_boost
            + recency_boost;
        scored.push(ScoredMemory {
            id: memory.id.to_string(),
            score,
            explanation: vec![
                format!("token_overlap={token_overlap}"),
                format!("tag_overlap={tag_overlap}"),
                format!("entity_overlap={entity_overlap}"),
                format!("bm25={bm25_sum:.3}"),
                format!("bm25_norm={bm25_norm:.3}"),
                format!("importance={importance:.3}"),
                format!("confidence={confidence:.3}"),
                format!("summary_boost={summary_boost:.3}"),
                format!("project_scope_boost={project_scope_boost:.3}"),
                format!("recency_boost={recency_boost:.3}"),
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

/// A small boost for "live" memories, decaying linearly over ~30 days. Keyed on
/// the most recent of `last_used_at` and `created_at`, so BOTH a recently-served
/// memory AND a just-learned (never-served) one surface among lexical ties —
/// without a never-served fresh preference being buried under an old one. Kept
/// well below the BM25 weight so it only breaks ties. The engine bumps
/// `last_used_at` when a memory is served (see `NoemaEngine::recall`).
fn recency_boost(memory: &MemoryRecord) -> f32 {
    let reference = match memory.last_used_at {
        Some(last_used) => last_used.max(memory.created_at),
        None => memory.created_at,
    };
    let age_days = (time::OffsetDateTime::now_utc() - reference)
        .whole_days()
        .max(0) as f32;
    (0.06 * (1.0 - age_days / 30.0)).max(0.0)
}

fn query_tokens_with_aliases(
    query: &str,
    principal: &Principal,
    project_id: Option<&ProjectId>,
    memories: &[MemoryRecord],
) -> HashSet<String> {
    let mut tokens = text::tokenize(query);
    for _ in 0..3 {
        let mut changed = false;
        for memory in memories {
            if !is_recallable_by_request(memory, principal, project_id) {
                continue;
            }
            for (left, right) in extract_alias_pairs(&memory.body) {
                if alias_side_matches_query(query, &tokens, right) {
                    changed |= extend_tokens(&mut tokens, left);
                }
                if alias_side_matches_query(query, &tokens, left) {
                    changed |= extend_tokens(&mut tokens, right);
                }
            }
        }
        if !changed {
            break;
        }
    }
    tokens
}

/// Whether a memory may be recalled by this request (tenant, status, visibility,
/// recall policy, clearance, host). Exposed so the multi-hop graph walk builds
/// its adjacency over exactly the same recallable set as lexical recall.
pub fn is_recallable_by_request(
    memory: &MemoryRecord,
    principal: &Principal,
    project_id: Option<&ProjectId>,
) -> bool {
    memory.tenant_id == principal.tenant_id
        && memory.status == MemoryStatus::Active
        && is_visible_to_request(memory, principal, project_id)
        && memory.recall_policy.mode != RecallMode::Never
        && principal.clearance.allows(memory.sensitivity)
        && (memory.recall_policy.allowed_hosts.is_empty()
            || memory
                .recall_policy
                .allowed_hosts
                .iter()
                .any(|host| host == principal.host.as_str()))
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

pub(crate) fn extract_alias_pairs(text: &str) -> Vec<(&str, &str)> {
    let mut pairs = Vec::new();
    for marker in [
        "也就是",
        "就是",
        "也叫",
        "又叫",
        "叫做",
        "叫",
        " aka ",
        " AKA ",
        "=",
    ] {
        if let Some((left, right)) = text.split_once(marker) {
            let left = cleanup_alias_side(left);
            let right = cleanup_alias_side(right);
            if is_valid_alias_side(left) && is_valid_alias_side(right) {
                pairs.push((left, right));
            }
        }
    }
    pairs
}

fn cleanup_alias_side(input: &str) -> &str {
    input
        .trim()
        .trim_matches(['"', '\'', '“', '”', '‘', '’'])
        .trim_matches([
            ' ', '\t', '\n', '\r', '。', '，', ',', '.', '；', ';', ':', '：',
        ])
        .trim()
}

fn is_valid_alias_side(input: &str) -> bool {
    let chars = input.chars().count();
    (2..=32).contains(&chars)
        && !input.contains('？')
        && !input.contains('?')
        && !["什么", "怎么", "如何", "为什么", "哪里", "哪位"]
            .iter()
            .any(|marker| input.contains(marker))
}

fn alias_side_matches_query(query: &str, query_tokens: &HashSet<String>, side: &str) -> bool {
    query.contains(side)
        || crate::text::tokenize(side)
            .iter()
            .any(|token| query_tokens.contains(token))
}

fn extend_tokens(tokens: &mut HashSet<String>, text: &str) -> bool {
    let mut changed = false;
    for token in crate::text::tokenize(text) {
        changed |= tokens.insert(token);
    }
    changed
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
    fn recall_matches_cjk_preference_questions() {
        let principal = Principal::personal("kay", "zode");
        let memory = MemoryRecord::new_user_preference(
            MemoryId::new("mem_cjk"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "王小明爱吃酸的",
        );

        let results = recall("王小明爱吃什么", &principal, None, &[memory]);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "mem_cjk");
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn recall_expands_cjk_aliases_for_related_memories() {
        let principal = Principal::personal("kay", "zode");
        let alias = MemoryRecord::new_user_preference(
            MemoryId::new("mem_alias"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "老李就是李小红",
        );
        let hobby = MemoryRecord::new_user_preference(
            MemoryId::new("mem_hobby"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "老李爱健身",
        );

        let results = recall("李小红的爱好", &principal, None, &[alias, hobby]);

        assert!(results.iter().any(|result| result.id == "mem_alias"));
        assert!(results.iter().any(|result| result.id == "mem_hobby"));
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
    fn recall_matches_across_word_inflections() {
        let principal = Principal::personal("kay", "zode");
        let memory = MemoryRecord::new_user_preference(
            MemoryId::new("mem_infl"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Prefer ripgrep when searching.",
        );
        // The query says "search"; the body says "searching" — with no stemming
        // these never overlap and the memory is filtered out entirely.
        let results = recall("search tool", &principal, None, &[memory]);
        assert_eq!(results.len(), 1, "{results:?}");
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn recall_prefers_recently_used_memory_among_ties() {
        use time::{Duration, OffsetDateTime};
        let principal = Principal::personal("kay", "zode");
        let make = |id: &str| {
            MemoryRecord::new_user_preference(
                MemoryId::new(id),
                TenantId::new("personal"),
                UserId::new("kay"),
                "Use Rust for the Noema memory system.",
            )
        };
        // Both were learned long ago (so creation freshness doesn't dominate);
        // only `warm` has been served recently.
        let mut cold = make("mem_cold");
        cold.created_at = OffsetDateTime::now_utc() - Duration::days(60);
        cold.updated_at = cold.created_at;
        let mut warm = make("mem_warm");
        warm.created_at = OffsetDateTime::now_utc() - Duration::days(60);
        warm.updated_at = warm.created_at;
        warm.last_used_at = Some(OffsetDateTime::now_utc() - Duration::hours(1));

        let results = recall("rust memory", &principal, None, &[cold, warm]);
        // Otherwise-identical memories tie on BM25; the recently-used one must
        // win so the "live" memory surfaces first.
        assert_eq!(results[0].id, "mem_warm", "{results:?}");
    }

    #[test]
    fn recall_prefers_freshly_created_memory_even_if_never_used() {
        use time::{Duration, OffsetDateTime};
        let principal = Principal::personal("kay", "zode");
        let make = |id: &str| {
            MemoryRecord::new_user_preference(
                MemoryId::new(id),
                TenantId::new("personal"),
                UserId::new("kay"),
                "Use Rust for the Noema memory system.",
            )
        };
        // Neither has ever been served (last_used_at = None). The stale one was
        // created 60 days ago; the fresh one just now. The just-learned memory
        // must surface first — a brand-new preference shouldn't be buried under
        // an old never-served one that ties on BM25.
        let mut stale = make("mem_stale");
        stale.created_at = OffsetDateTime::now_utc() - Duration::days(60);
        stale.updated_at = stale.created_at;
        let fresh = make("mem_fresh"); // created_at defaults to now

        let results = recall("rust memory", &principal, None, &[stale, fresh]);
        assert_eq!(results[0].id, "mem_fresh", "{results:?}");
    }

    #[test]
    fn recall_ranks_rare_term_match_above_common_term_match() {
        let principal = Principal::personal("kay", "zode");
        let mut memories = Vec::new();
        // Make "rust" a common term across the corpus (high document frequency).
        for i in 0..8 {
            memories.push(MemoryRecord::new_user_preference(
                MemoryId::new(format!("mem_filler_{i}")),
                TenantId::new("personal"),
                UserId::new("kay"),
                "rust",
            ));
        }
        // One memory matches only the common term, one only the rare term.
        memories.push(MemoryRecord::new_user_preference(
            MemoryId::new("mem_common"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "rust",
        ));
        memories.push(MemoryRecord::new_user_preference(
            MemoryId::new("mem_rare"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "kubernetes",
        ));

        let results = recall("rust kubernetes", &principal, None, &memories);
        let score_of = |id: &str| results.iter().find(|r| r.id == id).map(|r| r.score);
        let rare = score_of("mem_rare").expect("rare memory recalled");
        let common = score_of("mem_common").expect("common memory recalled");
        // IDF must make the single rare-term match outrank the single common-term
        // match; a plain overlap count ties them (both match one query term).
        assert!(rare > common, "rare={rare} common={common}");
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
