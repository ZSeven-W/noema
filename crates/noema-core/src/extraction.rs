use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::Result;
use crate::ids::{TenantId, UserId};
use crate::jsonl::{append_jsonl_locked, read_jsonl};

// ---------------------------------------------------------------------------
// Ingest: turning free text into enriched memory candidates.
//
// This is the LLM-Wiki "ingest" step. The host can supply an LLM-backed
// `Extractor` for high-quality extraction; the built-in `HeuristicExtractor`
// needs no model and runs anywhere, so entities/tags are populated even with
// zero external dependencies. Populating entities is what makes the entity
// recall boosts, candidate dedup, and the PageIndex catalog actually fire —
// previously callers almost always passed an empty entity list.
// ---------------------------------------------------------------------------

/// A memory candidate distilled from free text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedMemory {
    pub body: String,
    pub entities: Vec<String>,
    pub tags: Vec<String>,
}

/// Pluggable ingest. Implement this with an LLM in the host for richer
/// extraction; [`HeuristicExtractor`] is the dependency-free default.
pub trait Extractor {
    fn extract(&self, text: &str) -> Vec<ExtractedMemory>;
}

/// Dependency-free extractor: keeps the text as a single memory body and fills
/// entities heuristically (no model, no network).
#[derive(Debug, Clone, Copy, Default)]
pub struct HeuristicExtractor;

impl Extractor for HeuristicExtractor {
    fn extract(&self, text: &str) -> Vec<ExtractedMemory> {
        let body = text.trim().to_string();
        if body.is_empty() {
            return Vec::new();
        }
        vec![ExtractedMemory {
            entities: extract_entities(text),
            tags: Vec::new(),
            body,
        }]
    }
}

fn is_han(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0x20000..=0x2A6DF)
}

/// Conservative entity extraction. High-precision rules only, because a wrong
/// entity pollutes the catalog and dedup:
/// - English: possessive proper nouns (`Melanie's` → `Melanie`) and
///   capitalized words that appear mid-sentence (preceded by a lowercase word),
///   which skips sentence-initial verbs like "Use"/"Prefer".
/// - Chinese: a 2–4 character Han name immediately followed by a personal
///   predicate (爱/喜/讨/是/姓/叫 …), e.g. `王小明爱吃酸的` → `王小明`.
pub fn extract_entities(text: &str) -> Vec<String> {
    let mut entities: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push = |raw: &str| {
        let value = raw
            .trim()
            .trim_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ';'));
        if value.chars().count() >= 2 && seen.insert(value.to_lowercase()) {
            entities.push(value.to_string());
        }
    };

    let words: Vec<&str> = text.split_whitespace().collect();
    for (index, raw) in words.iter().enumerate() {
        let word = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}');
        let Some(first) = word.chars().next() else {
            continue;
        };
        if !first.is_ascii_uppercase() {
            continue;
        }
        if let Some(base) = word
            .strip_suffix("'s")
            .or_else(|| word.strip_suffix("\u{2019}s"))
        {
            push(base);
            continue;
        }
        // Mid-sentence capitalized word (previous token starts lowercase) — a
        // proper noun, not a sentence-initial verb.
        if index > 0 {
            let prev = words[index - 1].trim_matches(|c: char| !c.is_alphanumeric());
            if prev.chars().next().is_some_and(char::is_lowercase) {
                push(word);
            }
        }
    }

    push_cjk_names(text, &mut push);
    entities
}

fn push_cjk_names(text: &str, push: &mut impl FnMut(&str)) {
    const PREDICATES: &[char] = &[
        '爱', '喜', '讨', '是', '姓', '叫', '在', '有', '住', '用', '做',
    ];
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !is_han(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_han(chars[i]) {
            i += 1;
        }
        let run_len = i - start;
        // Prefer a 3-char name (most common full Chinese name), then 2, then 4.
        for name_len in [3usize, 2, 4] {
            if run_len > name_len && PREDICATES.contains(&chars[start + name_len]) {
                let name: String = chars[start..start + name_len].iter().collect();
                push(&name);
                break;
            }
        }
    }
}

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
    fn extract_entities_finds_english_proper_nouns() {
        let entities = extract_entities("Melanie's favorite book is set in Noema.");
        assert!(entities.iter().any(|e| e == "Melanie"), "{entities:?}");
        assert!(entities.iter().any(|e| e == "Noema"), "{entities:?}");
    }

    #[test]
    fn extract_entities_skips_sentence_initial_verbs() {
        // "Use" and "Prefer" start the sentence and are verbs, not entities.
        let entities = extract_entities("Use ripgrep for searching.");
        assert!(!entities.iter().any(|e| e == "Use"), "{entities:?}");
    }

    #[test]
    fn extract_entities_finds_chinese_name_before_predicate() {
        assert!(extract_entities("王小明爱吃酸的").contains(&"王小明".to_string()));
        assert!(extract_entities("李小红喜欢健身").contains(&"李小红".to_string()));
    }

    #[test]
    fn heuristic_extractor_keeps_body_and_fills_entities() {
        let extracted = HeuristicExtractor.extract("王小明爱吃酸的");
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].body, "王小明爱吃酸的");
        assert!(extracted[0].entities.contains(&"王小明".to_string()));
    }

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
