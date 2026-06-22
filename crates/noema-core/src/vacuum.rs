use std::path::Path;

use crate::error::Result;
use crate::hippocampus::{load_candidates, load_decisions, pending_candidates, ReviewDecision};
use crate::jsonl::append_jsonl;
use crate::lock::{atomic_write, atomic_write_locked, FileLock};

pub fn compact_hippocampus(tenant_dir: &Path) -> Result<()> {
    // Serialize the whole read→rewrite cycle against the engine's mutating
    // operations (submit_candidate / review_decide), which also take this lock.
    // Without it, a candidate appended between our read of the inbox and our
    // rewrite of it would be silently dropped. Callers must NOT already hold
    // this lock (fs4 advisory locks are per-process and would self-deadlock).
    let _lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
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
    // Overwrite the snapshot with the current pending set; "-latest" is a
    // point-in-time view, not an append log, so it must not grow unbounded.
    atomic_write(&snapshot, &jsonl_bytes(&pending)?)?;
    // Archive terminal candidates and the full decision history (append-only).
    for candidate in candidates
        .iter()
        .filter(|candidate| terminal.contains(&candidate.id))
    {
        append_jsonl(&archive, candidate)?;
    }
    for decision in &decisions_loaded {
        append_jsonl(&archive, decision)?;
    }
    // Replace the inbox with only the survivors in a SINGLE atomic write. The
    // previous truncate-then-re-append left a crash window in which the inbox
    // was empty and every pending candidate was lost. Edits are already baked
    // into `pending`, so the decisions log can be cleared afterwards; a crash
    // between the two writes is harmless (stale decisions reference candidates
    // that are no longer in the inbox, or re-apply an idempotent edit).
    atomic_write_locked(&hip.join("inbox.lock"), &inbox, &jsonl_bytes(&pending)?)?;
    atomic_write_locked(&hip.join("decisions.lock"), &decisions, b"")?;
    Ok(())
}

fn jsonl_bytes<T: serde::Serialize>(rows: &[T]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    for row in rows {
        buf.extend_from_slice(&serde_json::to_vec(row)?);
        buf.push(b'\n');
    }
    Ok(buf)
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

    #[test]
    fn compact_does_not_drop_concurrently_submitted_candidates() {
        use crate::api::{NoemaEngine, RememberRequest};
        use crate::ids::UserId;
        use crate::memory::{MemoryKind, Scope};
        use crate::sensitivity::{Principal, SensitivityLevel};
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(NoemaEngine::new(dir.path()).unwrap());
        engine.init_personal(&UserId::new("kay")).unwrap();
        let principal = Principal::personal("kay", "noema-cli");
        let tenant_dir = engine.paths.tenant_dir(&principal.tenant_id);

        let writers = 4usize;
        let per = 40usize;
        let mut handles = Vec::new();
        for w in 0..writers {
            let engine = engine.clone();
            let principal = principal.clone();
            handles.push(std::thread::spawn(move || {
                for s in 0..per {
                    engine
                        .submit_candidate(RememberRequest {
                            principal: principal.clone(),
                            text: format!("candidate {w}:{s}"),
                            scope: Scope::User,
                            project_path: None,
                            kind: MemoryKind::Preference,
                            sensitivity: SensitivityLevel::Internal,
                            tags: vec![],
                            entities: vec![],
                            confidence: 1.0,
                            importance: 0.5,
                        })
                        .unwrap();
                }
            }));
        }
        // A compactor running concurrently must serialize against submissions; a
        // read→rewrite window without the tenant lock would silently drop any
        // candidate appended between the read and the write.
        let compactor = {
            let tenant_dir = tenant_dir.clone();
            std::thread::spawn(move || {
                for _ in 0..60 {
                    compact_hippocampus(&tenant_dir).unwrap();
                }
            })
        };
        for handle in handles {
            handle.join().unwrap();
        }
        compactor.join().unwrap();
        compact_hippocampus(&tenant_dir).unwrap();

        // Default WritePolicy::Review keeps every candidate pending, so all of
        // them must survive in the live inbox after compaction.
        let hip = tenant_dir.join("hippocampus");
        let pending = crate::hippocampus::load_candidates(&hip.join("inbox.jsonl")).unwrap();
        let archived =
            crate::hippocampus::load_candidates(&hip.join("archive/compacted-latest.jsonl"))
                .unwrap();
        assert_eq!(
            pending.len() + archived.len(),
            writers * per,
            "no candidate may be lost across concurrent compaction"
        );
    }
}
