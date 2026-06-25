//! Deterministic multi-hop retrieval: seed with lexical recall, then walk the
//! memory graph (curated `links` + shared-entity edges) outward up to `max_hops`,
//! so an answer that lives several hops from any term the query mentions still
//! surfaces. This is the no-LLM baseline that works for every host; an agentic
//! host (zode / Claude Code / Codex) can instead drive deeper, smarter hops by
//! calling `search` / `browse` / `neighbors` itself and reasoning between steps.
//!
//! Scores decay by `HOP_DECAY` per hop, so memories closer to a lexical match
//! rank above distant ones, and a node reached by several paths keeps its best
//! (closest) score. Breadth is capped (`MAX_NODES`) so a densely linked store
//! cannot blow up the walk.

use std::collections::{HashMap, HashSet};

use crate::ids::ProjectId;
use crate::memory::MemoryRecord;
use crate::recall::{is_recallable_by_request, recall, ScoredMemory};
use crate::sensitivity::Principal;

/// Fraction of the score carried across each additional hop.
const HOP_DECAY: f32 = 0.5;
/// Upper bound on total reached memories, to bound a dense graph.
const MAX_NODES: usize = 256;

pub fn recall_multihop(
    query: &str,
    principal: &Principal,
    project_id: Option<&ProjectId>,
    memories: &[MemoryRecord],
    max_hops: usize,
) -> Vec<ScoredMemory> {
    let seeds = recall(query, principal, project_id, memories);
    if max_hops == 0 || seeds.is_empty() {
        return seeds;
    }

    let recallable: Vec<&MemoryRecord> = memories
        .iter()
        .filter(|memory| is_recallable_by_request(memory, principal, project_id))
        .collect();
    let by_id: HashMap<&str, &MemoryRecord> =
        recallable.iter().map(|m| (m.id.as_str(), *m)).collect();

    // Best (closest) score reached per memory id, plus the seeds' explanations.
    let mut best: HashMap<String, f32> = HashMap::new();
    let mut explanation: HashMap<String, Vec<String>> = HashMap::new();
    for seed in &seeds {
        best.insert(seed.id.clone(), seed.score);
        explanation.insert(seed.id.clone(), seed.explanation.clone());
    }

    let mut frontier: Vec<(String, f32)> = seeds.iter().map(|s| (s.id.clone(), s.score)).collect();
    for hop in 1..=max_hops {
        if best.len() >= MAX_NODES {
            break;
        }
        let mut next: Vec<(String, f32)> = Vec::new();
        for (id, score) in &frontier {
            let inherited = score * HOP_DECAY;
            let Some(memory) = by_id.get(id.as_str()) else {
                continue;
            };
            for neighbor in neighbor_ids(memory, &recallable, &by_id) {
                let entry = best.entry(neighbor.clone()).or_insert(f32::MIN);
                if inherited > *entry {
                    *entry = inherited;
                    explanation
                        .entry(neighbor.clone())
                        .or_insert_with(|| vec![format!("graph_hop={hop}"), format!("from={id}")]);
                    next.push((neighbor, inherited));
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    let mut out: Vec<ScoredMemory> = best
        .into_iter()
        .map(|(id, score)| ScoredMemory {
            explanation: explanation.remove(&id).unwrap_or_default(),
            id,
            score,
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Public single-hop neighbor lookup: the recallable memories that `memory_id`
/// links to or shares an entity with. The primitive for host-driven multi-hop.
pub fn neighbors_of(
    memory_id: &str,
    principal: &Principal,
    project_id: Option<&ProjectId>,
    memories: &[MemoryRecord],
) -> Vec<MemoryRecord> {
    let recallable: Vec<&MemoryRecord> = memories
        .iter()
        .filter(|memory| is_recallable_by_request(memory, principal, project_id))
        .collect();
    let by_id: HashMap<&str, &MemoryRecord> =
        recallable.iter().map(|m| (m.id.as_str(), *m)).collect();
    let Some(memory) = by_id.get(memory_id) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    neighbor_ids(memory, &recallable, &by_id)
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .filter_map(|id| by_id.get(id.as_str()).map(|m| (*m).clone()))
        .collect()
}

/// One graph hop from `memory`: curated link targets that are recallable, plus
/// every recallable memory that shares at least one entity (the associative
/// trail that bridges facts about the same subject).
fn neighbor_ids(
    memory: &MemoryRecord,
    recallable: &[&MemoryRecord],
    by_id: &HashMap<&str, &MemoryRecord>,
) -> Vec<String> {
    let mut out = Vec::new();
    for link in &memory.links {
        if by_id.contains_key(link.target.as_str()) {
            out.push(link.target.clone());
        }
    }
    if !memory.entities.is_empty() {
        for other in recallable {
            if other.id == memory.id {
                continue;
            }
            let shares_entity = other.entities.iter().any(|entity| {
                memory
                    .entities
                    .iter()
                    .any(|own| own.eq_ignore_ascii_case(entity))
            });
            if shares_entity {
                out.push(other.id.to_string());
            }
        }
    }
    for other in recallable {
        if other.id == memory.id {
            continue;
        }
        if shares_alias(memory, other) {
            out.push(other.id.to_string());
        }
    }
    out
}

fn shares_alias(left: &MemoryRecord, right: &MemoryRecord) -> bool {
    alias_pair_connects(left, right) || alias_pair_connects(right, left)
}

fn alias_pair_connects(alias_memory: &MemoryRecord, other: &MemoryRecord) -> bool {
    crate::recall::extract_alias_pairs(&alias_memory.body)
        .into_iter()
        .any(|(alias_left, alias_right)| {
            mentions_alias_side(other, alias_left) || mentions_alias_side(other, alias_right)
        })
}

fn mentions_alias_side(memory: &MemoryRecord, side: &str) -> bool {
    memory.body.contains(side)
        || memory
            .entities
            .iter()
            .any(|entity| entity.eq_ignore_ascii_case(side))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemoryId, TenantId, UserId};
    use crate::memory::MemoryLink;

    fn mem(id: &str, body: &str) -> MemoryRecord {
        MemoryRecord::new_user_preference(
            MemoryId::new(id),
            TenantId::new("personal"),
            UserId::new("kay"),
            body,
        )
    }

    #[test]
    fn multihop_follows_links_and_shared_entities_three_hops() {
        let principal = Principal::personal("kay", "zode");
        // Chain: A --link--> B --shared entity--> C --link--> D.
        let mut a = mem("mem_a", "ripgrep is a fast search tool");
        a.links = vec![MemoryLink {
            rel: "related".into(),
            target: "mem_b".into(),
        }];
        let mut b = mem("mem_b", "fzf is a fuzzy finder");
        b.entities = vec!["Tooling".into()];
        let mut c = mem("mem_c", "bat shows files with color");
        c.entities = vec!["Tooling".into()];
        c.links = vec![MemoryLink {
            rel: "related".into(),
            target: "mem_d".into(),
        }];
        let d = mem("mem_d", "delta renders prettier diffs");

        // The query only matches A lexically.
        let results = recall_multihop("ripgrep", &principal, None, &[a, b, c, d], 3);
        for id in ["mem_a", "mem_b", "mem_c", "mem_d"] {
            assert!(
                results.iter().any(|r| r.id == id),
                "missing {id} in {results:?}"
            );
        }
        // The lexical seed must rank first; reached nodes decay with distance.
        assert_eq!(results[0].id, "mem_a");
    }

    #[test]
    fn multihop_with_zero_hops_is_plain_recall() {
        let principal = Principal::personal("kay", "zode");
        let mut a = mem("mem_a", "ripgrep is a fast search tool");
        a.links = vec![MemoryLink {
            rel: "related".into(),
            target: "mem_b".into(),
        }];
        let b = mem("mem_b", "fzf is a fuzzy finder");
        let results = recall_multihop("ripgrep", &principal, None, &[a, b], 0);
        assert!(results.iter().all(|r| r.id != "mem_b"));
    }

    #[test]
    fn multihop_follows_alias_edges_between_cjk_name_variants() {
        let principal = Principal::personal("kay", "zode");
        let alias = mem("mem_alias", "老杨就是杨晋飞");
        let hobby = mem("mem_hobby", "老杨爱打羽毛球");

        let results = recall_multihop("杨晋飞爱做什么", &principal, None, &[alias, hobby], 2);

        assert!(
            results.iter().any(|result| result.id == "mem_hobby"),
            "{results:?}"
        );
    }
}
