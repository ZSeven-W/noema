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

fn tokenize(text: &str) -> std::collections::HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(ToString::to_string)
        .collect()
}

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
