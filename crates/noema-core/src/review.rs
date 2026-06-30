use crate::config::WritePolicy;
use crate::hippocampus::Candidate;
use crate::memory::MemoryRecord;
use crate::sensitivity::SensitivityLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateRoute {
    PendingReview,
    AutoAccept,
    RejectSecret,
}

pub fn route_candidate(
    policy: WritePolicy,
    auto_accept_max_sensitivity: SensitivityLevel,
    candidate: &Candidate,
    active_memories: &[MemoryRecord],
) -> CandidateRoute {
    if matches!(
        candidate.sensitivity,
        crate::sensitivity::SensitivityLevel::Secret
    ) {
        return CandidateRoute::RejectSecret;
    }
    if !candidate.sensitivity.can_auto_accept()
        || !auto_accept_max_sensitivity.allows(candidate.sensitivity)
    {
        return CandidateRoute::PendingReview;
    }
    if has_duplicate_or_conflict(candidate, active_memories) {
        return CandidateRoute::PendingReview;
    }
    match policy {
        WritePolicy::Manual | WritePolicy::Review => CandidateRoute::PendingReview,
        WritePolicy::AutoSafe | WritePolicy::Auto => {
            if candidate.confidence >= 0.80
                && candidate.importance >= 0.45
                && candidate.novelty >= 0.50
            {
                CandidateRoute::AutoAccept
            } else {
                CandidateRoute::PendingReview
            }
        }
    }
}

pub fn has_duplicate_or_conflict(candidate: &Candidate, active_memories: &[MemoryRecord]) -> bool {
    active_memories.iter().any(|memory| {
        if memory.tenant_id != candidate.tenant_id
            || memory.scope != candidate.scope
            || memory.project_id != candidate.project_id
            || memory.kind != candidate.kind
        {
            return false;
        }
        let shared_entity = !candidate.entities.is_empty()
            && candidate.entities.iter().any(|entity| {
                memory
                    .entities
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(entity))
            });
        let token_overlap = tokenize(&candidate.body)
            .intersection(&tokenize(&memory.body))
            .count();
        let duplicate = token_overlap >= 4 || (shared_entity && token_overlap >= 2);
        let conflict = shared_entity && has_opposing_assertion(&candidate.body, &memory.body);
        duplicate || conflict
    })
}

/// Graded novelty in `[0, 1]`: `1.0` when nothing comparable exists, lower as
/// the candidate's tokens overlap an existing same-scope/kind memory. Feeds the
/// auto-accept gate so a near-duplicate — one below the hard duplicate
/// threshold in [`has_duplicate_or_conflict`] — still routes to review instead
/// of silently accumulating near-identical memories.
pub fn candidate_novelty(candidate: &Candidate, active_memories: &[MemoryRecord]) -> f32 {
    let candidate_tokens = tokenize(&candidate.body);
    if candidate_tokens.is_empty() {
        return 1.0;
    }
    let mut max_overlap = 0.0f32;
    for memory in active_memories {
        if memory.scope != candidate.scope
            || memory.project_id != candidate.project_id
            || memory.kind != candidate.kind
        {
            continue;
        }
        let shared = candidate_tokens
            .intersection(&tokenize(&memory.body))
            .count();
        let fraction = shared as f32 / candidate_tokens.len() as f32;
        if fraction > max_overlap {
            max_overlap = fraction;
        }
    }
    (1.0 - max_overlap).clamp(0.0, 1.0)
}

/// Whether two short assertions take OPPOSITE stances on the same action —
/// one "do / prefer / enable" against the other "don't / avoid / disable".
///
/// Each side is reduced to a [`Stance`] by scanning for markers, **checking
/// negative markers first** so a negated phrase wins over the affirmative verb
/// it contains (`"do not use"` ⊃ `"use"`, `"不用"` ⊃ `"用"`). They conflict only
/// when one side is clearly positive and the other clearly negative — so two
/// agreements (`"avoid X"` / `"don't use X"`) and unrelated text (`"不错"`, which
/// matches no marker) never register. Negative markers are whole phrases, never
/// the bare ambiguous `不` / `not`, to avoid spurious matches. Callers also gate
/// on a shared entity, bounding any residual ambiguity to related memories.
fn has_opposing_assertion(candidate: &str, existing: &str) -> bool {
    matches!(
        (stance(candidate), stance(existing)),
        (Stance::Positive, Stance::Negative) | (Stance::Negative, Stance::Positive)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stance {
    Positive,
    Negative,
    Neutral,
}

/// Classify an assertion's stance. Negative markers (avoidance / negated verbs)
/// are tested before positive ones so `"不用"` / `"do not use"` resolve negative
/// despite containing the positive stems `用` / `use`.
fn stance(text: &str) -> Stance {
    let text = text.to_lowercase();
    // Negative / avoidance markers — whole phrases (no bare 不 / not). English
    // markers match on word boundaries (see `contains_marker`), so "never use"
    // never fires on "nevertheless".
    const NEGATIVE: &[&str] = &[
        "avoid",
        "do not use",
        "don't use",
        "not use",
        "never use",
        "stop using",
        "disable",
        "deny",
        "dislike",
        "避免",
        "不用",
        "不要用",
        "别用",
        "勿用",
        "不使用",
        "不启用",
        "禁用",
        "禁止",
        "讨厌",
        "厌恶",
        "从不",
        "不喜欢",
        "不允许",
        "不需要",
        "无需",
    ];
    if NEGATIVE.iter().any(|m| contains_marker(&text, m)) {
        return Stance::Negative;
    }
    // Positive / preference markers.
    const POSITIVE: &[&str] = &[
        "use", "using", "prefer", "enable", "allow", "always", "like", "使用", "用", "启用",
        "允许", "总是", "偏好", "喜欢", "需要",
    ];
    if POSITIVE.iter().any(|m| contains_marker(&text, m)) {
        return Stance::Positive;
    }
    Stance::Neutral
}

/// Substring match for CJK markers (no word boundaries in Chinese) but a
/// WORD-boundary match for ASCII markers — so "use" doesn't fire on "used"/
/// "user", "like" not on "unlike"/"likely", "never use" not on "nevertheless".
fn contains_marker(haystack: &str, marker: &str) -> bool {
    if !marker.is_ascii() {
        return haystack.contains(marker);
    }
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(marker) {
        let start = from + rel;
        let end = start + marker.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The ids of active memories the candidate directly CONTRADICTS — same
/// tenant/scope/project/kind, a shared entity, and an opposing assertion. These
/// are the memories a newly-stored candidate should supersede (tombstone) so a
/// reversed preference ("prefer X" → "avoid X") doesn't leave both on file.
/// Tombstoned memories are skipped (already retired).
pub fn conflicting_memory_ids(
    candidate: &Candidate,
    active_memories: &[MemoryRecord],
) -> Vec<String> {
    active_memories
        .iter()
        .filter(|memory| {
            memory.status == MemoryStatus::Active
                && memory.tenant_id == candidate.tenant_id
                && memory.scope == candidate.scope
                && memory.project_id == candidate.project_id
                && memory.kind == candidate.kind
                && !candidate.entities.is_empty()
                && candidate.entities.iter().any(|entity| {
                    memory
                        .entities
                        .iter()
                        .any(|existing| existing.eq_ignore_ascii_case(entity))
                })
                && has_opposing_assertion(&candidate.body, &memory.body)
        })
        .map(|memory| memory.id.to_string())
        .collect()
}

use crate::memory::MemoryStatus;
use crate::text::tokenize;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hippocampus::Candidate;
    use crate::ids::{CandidateId, MemoryId, TenantId, UserId};
    use crate::memory::{MemoryKind, MemoryRecord};
    use crate::sensitivity::SensitivityLevel;

    #[test]
    fn auto_does_not_bypass_sensitive_ceiling() {
        let mut candidate = Candidate::new(CandidateId::new("cand"), "security finding");
        candidate.sensitivity = SensitivityLevel::Confidential;
        assert_eq!(
            route_candidate(
                WritePolicy::Auto,
                SensitivityLevel::Internal,
                &candidate,
                &[]
            ),
            CandidateRoute::PendingReview
        );
    }

    #[test]
    fn secret_is_rejected_before_review() {
        let mut candidate = Candidate::new(CandidateId::new("cand"), "sk-secret");
        candidate.sensitivity = SensitivityLevel::Secret;
        assert_eq!(
            route_candidate(
                WritePolicy::AutoSafe,
                SensitivityLevel::Internal,
                &candidate,
                &[]
            ),
            CandidateRoute::RejectSecret
        );
    }

    #[test]
    fn novelty_is_low_for_overlapping_candidate_and_high_for_fresh_one() {
        let existing = MemoryRecord::new_user_preference(
            MemoryId::new("mem_existing"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "ripgrep handles searching",
        );
        // Shares most of its tokens with an existing memory but is below the hard
        // duplicate threshold — novelty must still reflect the overlap.
        let mut overlapping = Candidate::new(CandidateId::new("cand_overlap"), "ripgrep searching");
        overlapping.kind = MemoryKind::Preference;
        assert!(
            candidate_novelty(&overlapping, std::slice::from_ref(&existing)) < 0.5,
            "overlapping candidate should be low novelty"
        );

        let mut fresh =
            Candidate::new(CandidateId::new("cand_fresh"), "kubernetes deployment cron");
        fresh.kind = MemoryKind::Preference;
        assert!(
            candidate_novelty(&fresh, std::slice::from_ref(&existing)) > 0.9,
            "unrelated candidate should be high novelty"
        );
    }

    #[test]
    fn auto_routes_duplicates_to_review() {
        let mut candidate =
            Candidate::new(CandidateId::new("cand"), "Prefer Rust for Noema memory.");
        candidate.kind = MemoryKind::Preference;
        let existing = MemoryRecord::new_user_preference(
            MemoryId::new("mem_existing"),
            TenantId::new("personal"),
            UserId::new(std::env::var("USER").unwrap_or_else(|_| "user".to_string())),
            "Prefer Rust for Noema memory.",
        );

        assert_eq!(
            route_candidate(
                WritePolicy::AutoSafe,
                SensitivityLevel::Internal,
                &candidate,
                &[existing]
            ),
            CandidateRoute::PendingReview
        );
    }

    #[test]
    fn auto_routes_entity_conflicts_to_review() {
        let mut candidate = Candidate::new(CandidateId::new("cand"), "Use yarn for Noema.");
        candidate.entities = vec!["Noema".into()];
        let mut existing = MemoryRecord::new_user_preference(
            MemoryId::new("mem_existing"),
            TenantId::new("personal"),
            UserId::new(std::env::var("USER").unwrap_or_else(|_| "user".to_string())),
            "Avoid yarn for Noema.",
        );
        existing.entities = vec!["Noema".into()];

        assert_eq!(
            route_candidate(
                WritePolicy::Auto,
                SensitivityLevel::Internal,
                &candidate,
                &[existing]
            ),
            CandidateRoute::PendingReview
        );
    }

    #[test]
    fn cjk_opposing_assertions_detected() {
        // 喜欢 (like) vs 讨厌 (dislike) — an antonym pair.
        assert!(has_opposing_assertion("我喜欢 ripgrep", "我讨厌 ripgrep"));
        // 用 (use) vs 避免 (avoid).
        assert!(has_opposing_assertion("用 yarn 构建", "避免 yarn"));
        // 启用 (enable) vs 禁用 (disable).
        assert!(has_opposing_assertion("启用沙箱", "禁用沙箱"));
        // 用 (use) vs 不用 (don't use) — negated CJK verb.
        assert!(has_opposing_assertion("用 ripgrep", "不用 ripgrep"));
        // Same polarity → not opposing.
        assert!(!has_opposing_assertion("喜欢 ripgrep", "喜欢 fd"));
    }

    #[test]
    fn english_enable_disable_and_like_word_boundaries() {
        // enable vs disable.
        assert!(has_opposing_assertion(
            "enable the sandbox",
            "disable the sandbox"
        ));
        // "likely" must not register as the positive "like".
        assert_eq!(stance("ripgrep will likely work"), Stance::Neutral);
        // but a real "like"/"dislike" does.
        assert!(has_opposing_assertion(
            "I like ripgrep",
            "I dislike ripgrep"
        ));
    }

    #[test]
    fn negation_polarity_detected_en_and_cjk() {
        // Same verb, opposite negation polarity.
        assert!(has_opposing_assertion("use ripgrep", "do not use ripgrep"));
        assert!(has_opposing_assertion("用 ripgrep", "不要用 ripgrep"));
        // Both affirmed → not opposing.
        assert!(!has_opposing_assertion("use ripgrep", "use ripgrep please"));
        // Both negated → not opposing (this was the substring trap: "do not
        // use" contains "use", "never use" contains "use").
        assert!(!has_opposing_assertion(
            "do not use ripgrep",
            "never use ripgrep"
        ));
        // "avoid X" and "do not use X" AGREE — must not be flagged as opposing.
        assert!(!has_opposing_assertion(
            "avoid ripgrep",
            "do not use ripgrep"
        ));
        // 不用 (don't use) and 避免 (avoid) AGREE.
        assert!(!has_opposing_assertion("不用 ripgrep", "避免 ripgrep"));
    }

    #[test]
    fn negation_does_not_false_positive_on_unrelated_text() {
        // 不错 ("not bad" = good) contains 不 but no negative marker — neutral.
        assert!(!has_opposing_assertion("ripgrep 不错", "用 ripgrep"));
        // Neutral statements (no stance markers) never oppose.
        assert!(!has_opposing_assertion(
            "ripgrep is a tool",
            "ripgrep was written in rust"
        ));
        // A bare 不 in another word (不仅) must not register negative.
        assert!(!has_opposing_assertion("不仅 ripgrep", "用 ripgrep"));
    }

    #[test]
    fn english_markers_match_on_word_boundaries() {
        // "nevertheless" must NOT register as the negative "never use".
        assert!(!has_opposing_assertion(
            "nevertheless ripgrep works",
            "use ripgrep"
        ));
        // "unlike" / "likely" must NOT register as the positive "like".
        assert!(!has_opposing_assertion(
            "unlike ripgrep, fd is fast",
            "avoid ripgrep"
        ));
        // "used"/"user" must NOT register as the positive "use".
        assert_eq!(stance("the user ran ripgrep"), Stance::Neutral);
        assert_eq!(stance("ripgrep was used once"), Stance::Neutral);
        // But a real "use" / "avoid" still classifies.
        assert_eq!(stance("use ripgrep"), Stance::Positive);
        assert_eq!(stance("avoid ripgrep"), Stance::Negative);
    }

    #[test]
    fn conflicting_memory_ids_finds_opposing_same_entity_kind() {
        let user = UserId::new("kay");
        let tenant = TenantId::new("personal");
        let mut candidate = Candidate::new(CandidateId::new("cand"), "避免用 ripgrep");
        candidate.kind = MemoryKind::Preference;
        candidate.entities = vec!["ripgrep".into()];

        let mut old = MemoryRecord::new_user_preference(
            MemoryId::new("mem_old"),
            tenant.clone(),
            user.clone(),
            "喜欢用 ripgrep",
        );
        old.entities = vec!["ripgrep".into()];

        // An unrelated memory (different entity) must NOT be flagged.
        let mut other = MemoryRecord::new_user_preference(
            MemoryId::new("mem_other"),
            tenant.clone(),
            user.clone(),
            "喜欢用 fd",
        );
        other.entities = vec!["fd".into()];

        let ids = conflicting_memory_ids(&candidate, &[old.clone(), other]);
        assert_eq!(ids, vec!["mem_old".to_string()]);

        // A tombstoned conflicting memory is ignored (already retired).
        let mut tombstoned = old;
        tombstoned.status = crate::memory::MemoryStatus::Tombstoned;
        assert!(conflicting_memory_ids(&candidate, &[tombstoned]).is_empty());
    }
}
