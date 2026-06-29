//! Shared tokenizer used by recall scoring, candidate dedup, and the lexical /
//! PageIndex catalogs. Centralizing it keeps "what counts as a match" identical
//! everywhere — previously recall, review, and index each had a slightly
//! different splitter, so dedup semantics drifted from recall semantics.
//!
//! Tokenization rules:
//! - lowercased, split on non-alphanumeric boundaries;
//! - CJK runs are segmented with jieba (search mode) into dictionary words plus
//!   recall-friendly sub-words, with common function particles dropped;
//! - Latin/numeric words are kept when at least `MIN_WORD_LEN` bytes and not a
//!   stopword, then reduced with the Snowball (Porter2) English stemmer so
//!   query and document inflections collapse to the same token.

use std::collections::{HashMap, HashSet};

/// Minimum byte length for a Latin/numeric word token. Two so that meaningful
/// tech acronyms (ai, ml, db, os, ci, ux, vm) survive; common 2-letter function
/// words are filtered by the stopword list instead.
const MIN_WORD_LEN: usize = 2;

/// Unique token set, used for query terms and membership tests.
pub fn tokenize(text: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    for_each_token(text, |token| {
        tokens.insert(token);
    });
    tokens
}

/// Tokenize every value in a list and union the results.
pub fn tokenize_values(values: &[String]) -> HashSet<String> {
    values.iter().flat_map(|value| tokenize(value)).collect()
}

/// Term-frequency counts, used as the per-document TF signal for BM25.
pub fn term_counts(text: &str) -> HashMap<String, u32> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for_each_token(text, |token| {
        *counts.entry(token).or_insert(0) += 1;
    });
    counts
}

/// Emit every token occurrence (with duplicates) to `sink`.
fn for_each_token(text: &str, mut sink: impl FnMut(String)) {
    let lower = text.to_lowercase();
    for segment in lower.split(|c: char| !c.is_alphanumeric()) {
        if segment.is_empty() {
            continue;
        }
        if segment.chars().any(is_cjk) {
            push_cjk_segment(segment, &mut sink);
        } else {
            push_word_token(segment, &mut sink);
        }
    }
}

/// Segment a CJK (possibly CJK+Latin) run with jieba's search mode, which emits
/// dictionary words plus finer sub-words tuned for retrieval recall — far less
/// noisy than character n-grams (no cross-word fragments). Single-character
/// function particles are dropped; embedded Latin fragments fall through to the
/// English path so they get stopword + stemmer treatment.
fn push_cjk_segment(segment: &str, sink: &mut impl FnMut(String)) {
    for piece in jieba().cut_for_search(segment, true) {
        let word = piece.word;
        if word.chars().any(is_cjk) {
            if word.chars().count() == 1 && is_cjk_stopword(word) {
                continue;
            }
            sink(word.to_string());
        } else {
            push_word_token(word, sink);
        }
    }
}

fn push_word_token(token: &str, sink: &mut impl FnMut(String)) {
    if token.len() >= MIN_WORD_LEN && !is_stopword(token) {
        sink(english_stem(token));
    }
}

/// Snowball (Porter2) English stemmer so query and document inflections reduce
/// to the same stem (search / searching / searched → search). Numbers, acronyms,
/// and very short tokens are left untouched.
fn english_stem(token: &str) -> String {
    if token.len() <= 3 || !token.bytes().all(|b| b.is_ascii_alphabetic()) {
        return token.to_string();
    }
    english_stemmer().stem(token).into_owned()
}

fn jieba() -> &'static jieba_rs::Jieba {
    static JIEBA: std::sync::OnceLock<jieba_rs::Jieba> = std::sync::OnceLock::new();
    JIEBA.get_or_init(jieba_rs::Jieba::new)
}

fn english_stemmer() -> &'static rust_stemmers::Stemmer {
    static STEMMER: std::sync::OnceLock<rust_stemmers::Stemmer> = std::sync::OnceLock::new();
    STEMMER.get_or_init(|| rust_stemmers::Stemmer::create(rust_stemmers::Algorithm::English))
}

/// Common single-character Chinese function particles — dropped so they do not
/// match everywhere (IDF already down-weights them, but the recall filter keys
/// off any overlap, so removing them avoids spurious matches).
fn is_cjk_stopword(token: &str) -> bool {
    matches!(
        token,
        "的" | "了"
            | "着"
            | "过"
            | "是"
            | "在"
            | "和"
            | "与"
            | "或"
            | "也"
            | "都"
            | "就"
            | "而"
            | "这"
            | "那"
            | "之"
            | "其"
            | "我"
            | "你"
            | "他"
            | "她"
            | "它"
            | "们"
            | "把"
            | "被"
            | "个"
            | "不"
            | "有"
    )
}

pub fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        // Common 2-letter function words (kept out so the 2-char floor admits
        // acronyms, not glue words).
        "am" | "an"
            | "as"
            | "at"
            | "be"
            | "by"
            | "do"
            | "go"
            | "he"
            | "if"
            | "in"
            | "is"
            | "it"
            | "me"
            | "my"
            | "no"
            | "of"
            | "on"
            | "or"
            | "so"
            | "to"
            | "up"
            | "us"
            | "we"
            | "about"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_drops_stopwords_and_keeps_stemmed_content() {
        let tokens = tokenize("The Rust tools of AI");
        assert!(tokens.contains("rust"));
        assert!(tokens.contains("tool")); // tools -> tool (Snowball)
        assert!(!tokens.contains("the")); // stopword
        assert!(!tokens.contains("of")); // 2-letter stopword
    }

    #[test]
    fn tokenize_segments_chinese_into_words_not_cross_word_ngrams() {
        let tokens = tokenize("自然语言处理");
        // Real word boundaries.
        assert!(tokens.contains("语言"), "{tokens:?}");
        assert!(tokens.contains("处理"), "{tokens:?}");
        // The cross-word bigram an n-gram tokenizer would wrongly emit.
        assert!(
            !tokens.contains("言处"),
            "spurious cross-word bigram: {tokens:?}"
        );
    }

    #[test]
    fn tokenize_keeps_acronyms_but_drops_two_letter_glue_words() {
        let tokens = tokenize("AI and ML for the DB of an OS");
        assert!(tokens.contains("ai"));
        assert!(tokens.contains("ml"));
        assert!(tokens.contains("db"));
        assert!(tokens.contains("os"));
        // 2-letter function words must not become tokens.
        for glue in ["of", "an", "to", "is", "by"] {
            assert!(!tokens.contains(glue), "glue word leaked: {glue}");
        }
    }

    #[test]
    fn tokenize_collapses_english_inflections_to_one_stem() {
        // Inflections of the same word must produce identical token sets so a
        // query inflection matches a document inflection.
        assert_eq!(tokenize("searching"), tokenize("searched"));
        assert_eq!(tokenize("running tools"), tokenize("run tool"));
        assert!(tokenize("prefers pnpm").contains("prefer"));
        // Words that merely end in -s/-ss must not be mangled apart.
        assert_eq!(tokenize("analysis"), tokenize("analysis"));
        assert!(tokenize("address").contains("address"));
    }

    #[test]
    fn term_counts_tracks_repeated_terms() {
        let counts = term_counts("rust rust noema");
        assert_eq!(counts.get("rust"), Some(&2));
        assert_eq!(counts.get("noema"), Some(&1));
    }
}
