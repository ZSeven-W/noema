use std::path::Path;

use crate::error::Result;
use crate::hippocampus::{load_candidates, load_decisions, pending_candidates, ReviewDecision};
use crate::jsonl::append_jsonl;
use crate::lock::atomic_write_locked;

pub fn compact_hippocampus(tenant_dir: &Path) -> Result<()> {
    let hip = tenant_dir.join("hippocampus");
    let inbox = hip.join("inbox.jsonl");
    let decisions = hip.join("decisions.jsonl");
    let candidates = load_candidates(&inbox)?;
    let decisions_loaded = load_decisions(&decisions)?;
    let pending = pending_candidates(&candidates, &decisions_loaded);
    let terminal: std::collections::HashSet<_> = decisions_loaded
        .iter()
        .filter_map(|decision| match decision {
            ReviewDecision::Accept { candidate_id }
            | ReviewDecision::Reject { candidate_id, .. }
            | ReviewDecision::Merge { candidate_id, .. } => Some(candidate_id.clone()),
            ReviewDecision::Edit { .. } => None,
        })
        .collect();

    let snapshot = hip.join("snapshots").join("pending-latest.jsonl");
    let archive = hip.join("archive").join("compacted-latest.jsonl");
    for candidate in &pending {
        append_jsonl(&snapshot, candidate)?;
    }
    for candidate in candidates
        .iter()
        .filter(|candidate| terminal.contains(&candidate.id))
    {
        append_jsonl(&archive, candidate)?;
    }
    for decision in &decisions_loaded {
        append_jsonl(&archive, decision)?;
    }
    atomic_write_locked(&hip.join("inbox.lock"), &inbox, b"")?;
    atomic_write_locked(&hip.join("decisions.lock"), &decisions, b"")?;
    for candidate in &pending {
        append_jsonl(&inbox, candidate)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hippocampus::{append_candidate, append_decision, Candidate, ReviewDecision};
    use crate::ids::CandidateId;

    #[test]
    fn vacuum_creates_snapshot_and_live_tail() {
        let dir = tempfile::tempdir().unwrap();
        let tenant = dir.path();
        let hip = tenant.join("hippocampus");
        let inbox = hip.join("inbox.jsonl");
        let decisions = hip.join("decisions.jsonl");
        let keep = Candidate::new(CandidateId::new("cand_keep"), "Keep");
        let reject = Candidate::new(CandidateId::new("cand_reject"), "Reject");
        append_candidate(&inbox, &keep).unwrap();
        append_candidate(&inbox, &reject).unwrap();
        append_decision(
            &decisions,
            &ReviewDecision::Reject {
                candidate_id: CandidateId::new("cand_reject"),
                reason: "wrong".into(),
            },
        )
        .unwrap();

        compact_hippocampus(tenant).unwrap();
        assert!(hip.join("snapshots").read_dir().unwrap().next().is_some());
        assert!(hip.join("archive").read_dir().unwrap().next().is_some());
        let archive = std::fs::read_to_string(hip.join("archive/compacted-latest.jsonl")).unwrap();
        assert!(archive.contains("cand_reject"));
        assert!(!archive.contains("cand_keep"));
    }
}
