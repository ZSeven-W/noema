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

fn has_opposing_assertion(candidate: &str, existing: &str) -> bool {
    let candidate = candidate.to_lowercase();
    let existing = existing.to_lowercase();
    [
        ("use", "avoid"),
        ("prefer", "avoid"),
        ("enable", "disable"),
        ("allow", "deny"),
        ("always", "never"),
    ]
    .iter()
    .any(|(positive, negative)| {
        (candidate.contains(positive) && existing.contains(negative))
            || (candidate.contains(negative) && existing.contains(positive))
    })
}

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
}
