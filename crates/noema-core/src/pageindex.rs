//! PageIndex: a reasoning-retrieval catalog over the memory store, in the
//! spirit of Karpathy's "LLM Wiki" `index.md` and PageIndex-style tree search —
//! *no vectors, no embeddings*. Memories are grouped into entity / topic
//! "pages"; a page carries a lexical rollup summary. Retrieval reads the
//! catalog first (cheap, deterministic), picks the best pages by query↔summary
//! overlap, and returns every memory on those pages — including ones that share
//! no lexical term with the query but live under the same entity. That is how a
//! single-hop catalog lookup collapses a multi-hop question ("what does X
//! like?" → the X page → all facts about X).
//!
//! The tree is intentionally shallow (root → page → memories) for now; deeper
//! topic nesting can be layered on without changing the retrieval contract.

use std::collections::{HashMap, HashSet};

use crate::ids::MemoryId;
use crate::memory::{MemoryRecord, MemoryStatus};
use crate::text;

/// Number of rollup keywords kept in a page summary.
const PAGE_SUMMARY_KEYWORDS: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct PageNode {
    /// Display title — an entity name, a tag, or "general".
    pub title: String,
    /// Lexical rollup: the page's most frequent content tokens. This is what
    /// retrieval matches the query against, so a page is reachable by any term
    /// that is common across its memories, not just by its title.
    pub summary: String,
    /// Memories filed on this page, in insertion order.
    pub memory_ids: Vec<MemoryId>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageIndex {
    pub pages: Vec<PageNode>,
}

/// Where a memory files when it has no entity: under its first tag, else here.
const GENERAL_PAGE: &str = "general";

impl PageIndex {
    /// Build the catalog from active memories. A memory with several entities is
    /// filed on each of their pages (an associative trail, à la Memex), so it is
    /// reachable from any of them.
    pub fn build(memories: &[MemoryRecord]) -> Self {
        // Preserve first-seen order of pages for stable output.
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, (String, Vec<MemoryId>)> = HashMap::new();

        for memory in memories {
            if memory.status != MemoryStatus::Active {
                continue;
            }
            for title in page_titles(memory) {
                let key = title.to_lowercase();
                let entry = groups.entry(key.clone()).or_insert_with(|| {
                    order.push(key);
                    (title.clone(), Vec::new())
                });
                if !entry.1.contains(&memory.id) {
                    entry.1.push(memory.id.clone());
                }
            }
        }

        let by_id: HashMap<&str, &MemoryRecord> =
            memories.iter().map(|m| (m.id.as_str(), m)).collect();

        let pages = order
            .into_iter()
            .filter_map(|key| groups.remove(&key))
            .map(|(title, memory_ids)| {
                let bodies: Vec<&str> = memory_ids
                    .iter()
                    .filter_map(|id| by_id.get(id.as_str()).map(|m| m.body.as_str()))
                    .collect();
                PageNode {
                    summary: rollup_summary(&title, &bodies),
                    title,
                    memory_ids,
                }
            })
            .collect();

        Self { pages }
    }

    /// Navigate the catalog: score each page by query↔(title+summary) overlap,
    /// then return the memories on the best pages until `limit` is reached.
    /// Pages with zero overlap are skipped. Order is by descending page score,
    /// memories within a page in catalog order, de-duplicated across pages.
    pub fn retrieve(&self, query: &str, limit: usize) -> Vec<MemoryId> {
        let query_tokens = text::tokenize(query);
        if query_tokens.is_empty() || limit == 0 {
            return Vec::new();
        }

        let mut ranked: Vec<(usize, &PageNode)> = self
            .pages
            .iter()
            .filter_map(|page| {
                let mut page_tokens = text::tokenize(&page.title);
                page_tokens.extend(text::tokenize(&page.summary));
                let overlap = query_tokens.intersection(&page_tokens).count();
                (overlap > 0).then_some((overlap, page))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (_, page) in ranked {
            for id in &page.memory_ids {
                if seen.insert(id.clone()) {
                    out.push(id.clone());
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// Render the catalog as an `index.md`-style markdown document — the
    /// human-/agent-readable "read this first" page of the LLM-Wiki pattern.
    pub fn to_markdown(&self) -> String {
        let mut out = String::from("# Memory Catalog\n");
        for page in &self.pages {
            out.push_str(&format!(
                "\n## {} ({} memories)\n{}\n",
                page.title,
                page.memory_ids.len(),
                page.summary
            ));
            for id in &page.memory_ids {
                out.push_str(&format!("- {id}\n"));
            }
        }
        out
    }
}

/// The page titles a memory files under: each of its entities, or its first tag,
/// or the catch-all `general` page.
fn page_titles(memory: &MemoryRecord) -> Vec<String> {
    if !memory.entities.is_empty() {
        return memory
            .entities
            .iter()
            .map(|entity| entity.trim().to_string())
            .filter(|entity| !entity.is_empty())
            .collect();
    }
    if let Some(tag) = memory.tags.iter().find(|tag| !tag.trim().is_empty()) {
        return vec![tag.trim().to_string()];
    }
    vec![GENERAL_PAGE.to_string()]
}

fn rollup_summary(title: &str, bodies: &[&str]) -> String {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for body in bodies {
        for (token, count) in text::term_counts(body) {
            *counts.entry(token).or_insert(0) += count;
        }
    }
    let mut ranked: Vec<(String, u32)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let keywords: Vec<String> = ranked
        .into_iter()
        .take(PAGE_SUMMARY_KEYWORDS)
        .map(|(token, _)| token)
        .collect();
    if keywords.is_empty() {
        title.to_string()
    } else {
        format!("{title}: {}", keywords.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{TenantId, UserId};

    fn memory(id: &str, body: &str, entities: &[&str]) -> MemoryRecord {
        let mut record = MemoryRecord::new_user_preference(
            MemoryId::new(id),
            TenantId::new("personal"),
            UserId::new("kay"),
            body,
        );
        record.entities = entities.iter().map(|e| e.to_string()).collect();
        record
    }

    #[test]
    fn catalog_groups_memories_under_entity_pages() {
        let memories = vec![
            memory("mem_book", "favorite book is Charlotte's Web", &["Melanie"]),
            memory("mem_hobby", "enjoys pottery on weekends", &["Melanie"]),
            memory("mem_other", "deploys with kubernetes", &["Caroline"]),
        ];
        let index = PageIndex::build(&memories);
        let melanie = index
            .pages
            .iter()
            .find(|page| page.title == "Melanie")
            .expect("Melanie page exists");
        assert_eq!(melanie.memory_ids.len(), 2);
        assert!(index.to_markdown().contains("## Melanie"));
    }

    #[test]
    fn retrieve_surfaces_all_memories_on_the_matched_entity_page() {
        let memories = vec![
            memory("mem_book", "favorite book is Charlotte's Web", &["Melanie"]),
            memory("mem_hobby", "enjoys pottery on weekends", &["Melanie"]),
            memory("mem_other", "deploys with kubernetes", &["Caroline"]),
        ];
        let index = PageIndex::build(&memories);

        // "pottery" shares no term with the book memory, but navigating the
        // Melanie page returns both of Melanie's memories — the catalog collapses
        // the hop from question to entity to facts.
        let hits = index.retrieve("What does Melanie like?", 8);
        assert!(hits.iter().any(|id| id.as_str() == "mem_book"), "{hits:?}");
        assert!(hits.iter().any(|id| id.as_str() == "mem_hobby"), "{hits:?}");
        assert!(
            !hits.iter().any(|id| id.as_str() == "mem_other"),
            "unrelated entity page must not be returned: {hits:?}"
        );
    }

    #[test]
    fn entityless_memories_file_under_first_tag_then_general() {
        let mut tagged = memory("mem_tagged", "ripgrep is fast", &[]);
        tagged.tags = vec!["tools".to_string()];
        let bare = memory("mem_bare", "a stray note", &[]);

        let index = PageIndex::build(&[tagged, bare]);
        assert!(index.pages.iter().any(|page| page.title == "tools"));
        assert!(index.pages.iter().any(|page| page.title == GENERAL_PAGE));
    }
}
