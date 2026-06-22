use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::ids::MemoryId;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LexicalIndex {
    documents: Vec<IndexDocument>,
    postings: HashMap<String, HashSet<MemoryId>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexDocument {
    pub id: MemoryId,
    pub text: String,
    pub tags: Vec<String>,
    pub entities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexHit {
    pub id: MemoryId,
    pub score: f32,
}

impl LexicalIndex {
    pub fn add(&mut self, document: IndexDocument) {
        for token in tokens(&document.text)
            .into_iter()
            .chain(document.tags.iter().flat_map(|tag| tokens(tag)))
            .chain(document.entities.iter().flat_map(|entity| tokens(entity)))
        {
            self.postings
                .entry(token)
                .or_default()
                .insert(document.id.clone());
        }
        self.documents.push(document);
    }

    pub fn search(&self, query: &str) -> Vec<IndexHit> {
        let query_tokens = tokens(query);
        let mut scores: HashMap<MemoryId, f32> = HashMap::new();
        for token in query_tokens {
            if let Some(ids) = self.postings.get(&token) {
                for id in ids {
                    *scores.entry(id.clone()).or_insert(0.0) += 1.0;
                }
            }
        }
        let mut hits: Vec<_> = scores
            .into_iter()
            .map(|(id, raw)| IndexHit {
                id,
                score: raw / (raw + 3.0),
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits
    }
}

fn tokens(text: &str) -> HashSet<String> {
    crate::text::tokenize(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_index_returns_matching_memory_ids() {
        let mut index = LexicalIndex::default();
        index.add(IndexDocument {
            id: MemoryId::new("mem_rust"),
            text: "Prefer Rust for Noema memory.".to_string(),
            tags: vec!["rust".to_string()],
            entities: vec!["Noema".to_string()],
        });

        let results = index.search("rust noema");
        assert_eq!(results[0].id, MemoryId::new("mem_rust"));
        assert!(results[0].score > 0.0);
    }
}
