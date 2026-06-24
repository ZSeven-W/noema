use std::collections::{HashMap, HashSet};

use crate::ids::ProjectId;
use crate::memory::MemoryRecord;
use crate::multihop::recall_multihop;
use crate::pageindex::PageIndex;
use crate::recall::{is_recallable_by_request, recall, ScoredMemory};
use crate::sensitivity::Principal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FusionOptions {
    pub limit: usize,
    pub lexical_limit: usize,
    pub page_limit: usize,
    pub graph_hops: usize,
    pub max_per_entity: usize,
}

impl Default for FusionOptions {
    fn default() -> Self {
        Self {
            limit: 24,
            lexical_limit: 24,
            page_limit: 24,
            graph_hops: 2,
            max_per_entity: 4,
        }
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    score: f32,
    explanation: Vec<String>,
}

pub fn fusion_recall(
    query: &str,
    principal: &Principal,
    project_id: Option<&ProjectId>,
    memories: &[MemoryRecord],
    options: FusionOptions,
) -> Vec<ScoredMemory> {
    if options.limit == 0 {
        return Vec::new();
    }

    let recallable_memories: Vec<MemoryRecord> = memories
        .iter()
        .filter(|memory| is_recallable_by_request(memory, principal, project_id))
        .cloned()
        .collect();
    let by_id: HashMap<&str, &MemoryRecord> = recallable_memories
        .iter()
        .map(|memory| (memory.id.as_str(), memory))
        .collect();
    let mut candidates: HashMap<String, Candidate> = HashMap::new();

    for scored in recall(query, principal, project_id, memories)
        .into_iter()
        .take(options.lexical_limit)
    {
        upsert_candidate(
            &mut candidates,
            scored.id,
            scored.score,
            "lexical",
            scored.explanation,
        );
    }

    let page_ids = PageIndex::build(&recallable_memories).retrieve(query, options.page_limit);
    for (rank, id) in page_ids.into_iter().enumerate() {
        let score = 0.72_f32 - (rank as f32 * 0.01);
        upsert_candidate(
            &mut candidates,
            id.to_string(),
            score.max(0.30),
            "pageindex",
            vec![format!("page_rank={rank}")],
        );
    }

    for scored in recall_multihop(query, principal, project_id, memories, options.graph_hops) {
        let score = scored.score * 0.90;
        upsert_candidate(
            &mut candidates,
            scored.id,
            score,
            "graph",
            scored.explanation,
        );
    }

    let mut ranked: Vec<Candidate> = candidates.into_values().collect();
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });

    apply_diversity(ranked, &by_id, options)
}

fn upsert_candidate(
    candidates: &mut HashMap<String, Candidate>,
    id: String,
    score: f32,
    source: &str,
    mut explanation: Vec<String>,
) {
    explanation.insert(0, format!("source={source}"));
    candidates
        .entry(id.clone())
        .and_modify(|candidate| {
            if score > candidate.score {
                candidate.score = score;
            }
            candidate.explanation.extend(explanation.clone());
        })
        .or_insert(Candidate {
            id,
            score,
            explanation,
        });
}

fn apply_diversity(
    ranked: Vec<Candidate>,
    by_id: &HashMap<&str, &MemoryRecord>,
    options: FusionOptions,
) -> Vec<ScoredMemory> {
    let mut out = Vec::new();
    let mut entity_counts: HashMap<String, usize> = HashMap::new();
    let mut seen = HashSet::new();

    for candidate in ranked {
        if out.len() >= options.limit {
            break;
        }
        if !seen.insert(candidate.id.clone()) {
            continue;
        }
        if options.max_per_entity > 0 {
            if let Some(memory) = by_id.get(candidate.id.as_str()) {
                if memory.entities.iter().any(|entity| {
                    entity_counts
                        .get(&entity.to_lowercase())
                        .copied()
                        .unwrap_or(0)
                        >= options.max_per_entity
                }) {
                    continue;
                }
                for entity in &memory.entities {
                    *entity_counts.entry(entity.to_lowercase()).or_insert(0) += 1;
                }
            }
        }
        out.push(ScoredMemory {
            id: candidate.id,
            score: candidate.score,
            explanation: candidate.explanation,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemoryId, TenantId, UserId};
    use crate::memory::MemoryRecord;
    use crate::sensitivity::Principal;

    fn mem(id: &str, body: &str, entities: &[&str], tags: &[&str]) -> MemoryRecord {
        let mut record = MemoryRecord::new_user_preference(
            MemoryId::new(id),
            TenantId::new("personal"),
            UserId::new("kay"),
            body,
        );
        record.entities = entities.iter().map(|value| value.to_string()).collect();
        record.tags = tags.iter().map(|value| value.to_string()).collect();
        record
    }

    #[test]
    fn fusion_includes_pageindex_associative_hits() {
        let principal = Principal::personal("kay", "zode");
        let memories = vec![
            mem(
                "mem_book",
                "favorite book is Charlotte's Web",
                &["Melanie"],
                &[],
            ),
            mem("mem_hobby", "enjoys pottery on weekends", &["Melanie"], &[]),
            mem("mem_other", "deploys with kubernetes", &["Caroline"], &[]),
        ];

        let results = fusion_recall(
            "What else is connected to Charlotte's Web?",
            &principal,
            None,
            &memories,
            FusionOptions::default(),
        );
        let ids: Vec<&str> = results.iter().map(|result| result.id.as_str()).collect();

        assert!(ids.contains(&"mem_book"), "{results:?}");
        assert!(ids.contains(&"mem_hobby"), "{results:?}");
        assert!(!ids.contains(&"mem_other"), "{results:?}");
        assert!(results.iter().any(|result| {
            result.id == "mem_hobby"
                && result
                    .explanation
                    .iter()
                    .any(|line| line.contains("source=pageindex"))
        }));
    }

    #[test]
    fn fusion_keeps_lexical_hits_ahead_of_distant_graph_only_hits() {
        let principal = Principal::personal("kay", "zode");
        let mut seed = mem(
            "mem_seed",
            "ripgrep is the preferred search tool",
            &["Tooling"],
            &[],
        );
        seed.importance = 0.5;
        let distant = mem(
            "mem_distant",
            "delta renders readable diffs",
            &["Tooling"],
            &[],
        );
        let results = fusion_recall(
            "ripgrep search",
            &principal,
            None,
            &[seed, distant],
            FusionOptions::default(),
        );

        assert_eq!(results[0].id, "mem_seed", "{results:?}");
        assert!(
            results.iter().any(|result| result.id == "mem_distant"),
            "{results:?}"
        );
    }

    #[test]
    fn fusion_limits_memories_per_entity_for_diversity() {
        let principal = Principal::personal("kay", "zode");
        let memories = vec![
            mem("mem_a", "Melanie likes tea", &["Melanie"], &[]),
            mem("mem_b", "Melanie likes pottery", &["Melanie"], &[]),
            mem("mem_c", "Melanie likes hiking", &["Melanie"], &[]),
            mem("mem_d", "Caroline likes Rust", &["Caroline"], &[]),
        ];
        let options = FusionOptions {
            limit: 8,
            lexical_limit: 8,
            page_limit: 8,
            graph_hops: 1,
            max_per_entity: 2,
        };

        let results = fusion_recall(
            "What does Melanie like and what does Caroline like?",
            &principal,
            None,
            &memories,
            options,
        );
        let melanie_count = results
            .iter()
            .filter(|result| result.id == "mem_a" || result.id == "mem_b" || result.id == "mem_c")
            .count();

        assert_eq!(melanie_count, 2, "{results:?}");
        assert!(
            results.iter().any(|result| result.id == "mem_d"),
            "{results:?}"
        );
    }
}
