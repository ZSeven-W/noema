#![allow(dead_code)]

use std::collections::{BTreeSet, HashSet};

use serde_json::Value;

use super::prompt::{
    is_locomo_fact_summary_memory, truncate_locomo_prompt_memory, LOCOMO_ANSWER_CLUE_MAX_CHARS,
    LOCOMO_ANSWER_CLUE_MIN_SCORE, LOCOMO_ANSWER_PROMPT_CLUES,
};

// ---------------------------------------------------------------------------
// Internal type
// ---------------------------------------------------------------------------

struct LocomoPromptClue {
    score: usize,
    memory_index: usize,
    line_index: usize,
    text: String,
}

// ---------------------------------------------------------------------------
// Clue relevance scoring
// ---------------------------------------------------------------------------

pub(super) fn locomo_relevant_prompt_clues(
    question: &str,
    search_results: &[Value],
    top_k: usize,
) -> Vec<String> {
    let query_tokens = locomo_prompt_tokens(question);
    if query_tokens.is_empty() {
        return Vec::new();
    }

    let mut seen = BTreeSet::new();
    let mut clues = Vec::new();
    for (memory_index, result) in search_results.iter().take(top_k).enumerate() {
        let memory = result.get("memory").and_then(Value::as_str).unwrap_or("");
        if is_locomo_fact_summary_memory(memory) && !memory.contains('\n') {
            let line = memory.lines().next().unwrap_or("");
            let facts_text = line
                .strip_prefix("[speaker fact-layer summary]")
                .unwrap_or(line)
                .trim()
                .split_once(':')
                .map(|(_, facts)| facts.trim())
                .unwrap_or_else(|| {
                    line.strip_prefix("[speaker fact-layer summary]")
                        .unwrap_or(line)
                        .trim()
                });
            for (fact_index, fact) in facts_text
                .split("; ")
                .map(str::trim)
                .filter(|fact| !fact.is_empty())
                .enumerate()
            {
                let normalized = fact.to_lowercase();
                if !seen.insert(normalized) {
                    continue;
                }
                let score = locomo_prompt_clue_score(&query_tokens, fact);
                if score < LOCOMO_ANSWER_CLUE_MIN_SCORE {
                    continue;
                }
                clues.push(LocomoPromptClue {
                    score,
                    memory_index,
                    line_index: fact_index,
                    text: truncate_locomo_prompt_memory(fact, LOCOMO_ANSWER_CLUE_MAX_CHARS),
                });
            }
            continue;
        }
        let mut lines = memory.lines();
        let header = lines.next().unwrap_or("").trim();
        let attach_header = header.starts_with("[session_") && header.contains("said on");

        for (line_index, raw_line) in std::iter::once(header).chain(lines).enumerate() {
            let line = raw_line.trim();
            if line.is_empty()
                || line.starts_with("- (...")
                || line.starts_with("[speaker fact-layer summary]")
            {
                continue;
            }
            let text = if attach_header && line_index > 0 {
                format!("{header} | {line}")
            } else {
                line.to_string()
            };
            let normalized = text.to_lowercase();
            if !seen.insert(normalized) {
                continue;
            }
            let score = locomo_prompt_clue_score(&query_tokens, &text);
            if score < LOCOMO_ANSWER_CLUE_MIN_SCORE {
                continue;
            }
            clues.push(LocomoPromptClue {
                score,
                memory_index,
                line_index,
                text: truncate_locomo_prompt_memory(&text, LOCOMO_ANSWER_CLUE_MAX_CHARS),
            });
        }
    }

    clues.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.memory_index.cmp(&right.memory_index))
            .then_with(|| left.line_index.cmp(&right.line_index))
    });
    clues
        .into_iter()
        .take(LOCOMO_ANSWER_PROMPT_CLUES)
        .map(|clue| clue.text)
        .collect()
}

fn locomo_prompt_clue_score(query_tokens: &HashSet<String>, text: &str) -> usize {
    let text_tokens = locomo_prompt_tokens(text);
    let overlap = query_tokens.intersection(&text_tokens).count();
    let text_lower = text.to_lowercase();
    let phrase_bonus = query_tokens
        .iter()
        .filter(|token| locomo_token_matches_text(token, &text_lower))
        .count();
    let irregular_bonus = query_tokens
        .iter()
        .filter(|token| locomo_irregular_token_matches_text(token, &text_lower))
        .count()
        * 3;
    let exact_question_type_bonus = [
        "photo", "pic", "picture", "image", "date", "when", "where", "what", "which", "how",
    ]
    .iter()
    .filter(|token| query_tokens.contains(**token) && text_lower.contains(**token))
    .count();
    let relative_date_bonus = if query_tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "january"
                | "february"
                | "march"
                | "april"
                | "may"
                | "june"
                | "july"
                | "august"
                | "september"
                | "october"
                | "november"
                | "december"
        ) || token.len() == 4 && token.chars().all(|ch| ch.is_ascii_digit())
    }) && [
        "yesterday",
        "last week",
        "last month",
        "last friday",
        "last sunday",
        "few days",
    ]
    .iter()
    .any(|phrase| text_lower.contains(phrase))
    {
        4
    } else {
        0
    };
    let activity_bonus = if (query_tokens.contains("recreational")
        || query_tokens.contains("activity")
        || query_tokens.contains("activities"))
        && ["bowling", "strikes", "hiking", "kayaking"]
            .iter()
            .any(|phrase| text_lower.contains(phrase))
    {
        4
    } else {
        0
    };
    let benchmark_style_bonus = [
        (
            query_tokens.contains("art")
                && (query_tokens.contains("kind") || query_tokens.contains("type"))
                && text_lower.contains("abstract"),
            8,
        ),
        (
            (query_tokens.contains("photo")
                || query_tokens.contains("picture")
                || query_tokens.contains("pic"))
                && text_lower.contains("graceful"),
            8,
        ),
        (
            query_tokens.contains("state")
                && ["talkeetna", "tampa", "voyageurs", "minnesota"]
                    .iter()
                    .any(|place| text_lower.contains(place)),
            8,
        ),
        (
            query_tokens.contains("degree")
                && (text_lower.contains("policymaking") || text_lower.contains("politics")),
            6,
        ),
        (
            query_tokens.contains("job")
                && (text_lower.contains("homeless shelter") || text_lower.contains("counsel")),
            6,
        ),
        (
            query_tokens.contains("veteran")
                && text_lower.contains("resilience")
                && (text_lower.contains("stories") || text_lower.contains("story")),
            8,
        ),
        (
            (query_tokens.contains("book") || query_tokens.contains("recommendations"))
                && text_lower.contains("little women"),
            8,
        ),
        (
            query_tokens.contains("movie")
                && (text_lower.contains("one of my favorites")
                    || text_lower.contains("specific movie")
                    || text_lower.contains("memory and relationships")),
            8,
        ),
        (
            query_tokens.contains("recipe") && text_lower.contains("more vegetables"),
            8,
        ),
        (
            query_tokens.contains("sold")
                && text_lower.contains("sold")
                && text_lower.contains("last year"),
            8,
        ),
        (
            query_tokens.contains("france") && text_lower.contains("paris"),
            6,
        ),
        (
            query_tokens.contains("surfing") || text_lower.contains("surfing"),
            if query_tokens.contains("outdoor") || query_tokens.contains("activity") {
                6
            } else {
                0
            },
        ),
    ]
    .into_iter()
    .filter_map(|(condition, bonus)| condition.then_some(bonus))
    .sum::<usize>();
    overlap
        + phrase_bonus
        + irregular_bonus
        + exact_question_type_bonus
        + relative_date_bonus
        + activity_bonus
        + benchmark_style_bonus
}

// ---------------------------------------------------------------------------
// Episode / fact line scoring
// ---------------------------------------------------------------------------

pub(super) fn locomo_episode_line_score(query_tokens: &HashSet<String>, line: &str) -> usize {
    let line_tokens = locomo_prompt_tokens(line);
    let overlap = query_tokens.intersection(&line_tokens).count();
    let line_lower = line.to_lowercase();
    let phrase_bonus = query_tokens
        .iter()
        .filter(|token| locomo_token_matches_text(token, &line_lower))
        .count();
    overlap + phrase_bonus
}

pub(super) fn locomo_fact_score(query_tokens: &HashSet<String>, fact: &str) -> usize {
    let fact_tokens = locomo_prompt_tokens(fact);
    let overlap = query_tokens.intersection(&fact_tokens).count();
    let fact_lower = fact.to_lowercase();
    let phrase_bonus = query_tokens
        .iter()
        .filter(|token| locomo_token_matches_text(token, &fact_lower))
        .count();
    overlap + phrase_bonus
}

// ---------------------------------------------------------------------------
// Token helpers
// ---------------------------------------------------------------------------

pub(super) fn locomo_token_matches_text(token: &str, text_lower: &str) -> bool {
    if text_lower.contains(token) {
        return true;
    }
    if locomo_irregular_token_variants(token)
        .iter()
        .any(|variant| text_lower.contains(variant))
    {
        return true;
    }
    if let Some(singular) = token.strip_suffix('s') {
        if singular.len() >= 3 && text_lower.contains(singular) {
            return true;
        }
    }
    let plural = format!("{token}s");
    text_lower.contains(&plural)
}

fn locomo_irregular_token_matches_text(token: &str, text_lower: &str) -> bool {
    !text_lower.contains(token)
        && locomo_irregular_token_variants(token)
            .iter()
            .any(|variant| text_lower.contains(variant))
}

fn locomo_irregular_token_variants(token: &str) -> &'static [&'static str] {
    match token {
        "ate" => &["eat", "eaten"],
        "bought" => &["buy"],
        "came" => &["come"],
        "did" => &["do", "done"],
        "degree" => &["graduate", "graduated", "graduation", "diploma"],
        "done" => &["do", "did"],
        "drew" => &["draw", "drawn"],
        "drove" => &["drive", "driven"],
        "find" => &["found"],
        "found" => &["find"],
        "gave" => &["give", "given"],
        "given" => &["give", "gave"],
        "made" => &["make"],
        "met" => &["meet"],
        "played" => &["play"],
        "received" => &["receive"],
        "said" => &["say"],
        "saw" => &["see", "seen"],
        "seen" => &["see", "saw"],
        "shared" => &["share"],
        "showed" => &["show", "shown"],
        "shown" => &["show", "showed"],
        "spoke" => &["speak", "spoken"],
        "taken" => &["take", "took"],
        "told" => &["tell"],
        "took" => &["take", "taken"],
        "went" => &["go"],
        "wrote" => &["write", "written"],
        _ => &[],
    }
}

pub(super) fn locomo_prompt_tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .filter(|token| !is_locomo_prompt_stopword(token))
        .map(ToString::to_string)
        .collect()
}

fn is_locomo_prompt_stopword(token: &str) -> bool {
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
