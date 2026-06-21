use super::*;
use crate::ids::{TenantId, UserId};
use crate::sensitivity::Principal;

/// Submits a candidate and immediately accepts it so the memory is
/// available for recall. Mirrors `remember_text` for test fixtures.
fn seed_memory(
    engine: &NoemaEngine,
    principal: &Principal,
    text: &str,
    tags: Vec<String>,
    entities: Vec<String>,
) {
    engine
        .submit_candidate(RememberRequest {
            principal: principal.clone(),
            text: text.to_string(),
            scope: crate::memory::Scope::User,
            project_path: None,
            kind: crate::memory::MemoryKind::Preference,
            sensitivity: crate::sensitivity::SensitivityLevel::Internal,
            tags,
            entities,
            confidence: 1.0,
            importance: 0.5,
        })
        .unwrap();
    let pending = engine.review_list(principal).unwrap();
    engine
        .review_decide(ReviewDecisionRequest {
            principal: principal.clone(),
            candidate_id: pending[0].id.to_string(),
            action: ReviewAction::Accept,
        })
        .unwrap();
}

#[test]
fn engine_recall_returns_memorypack_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    let principal = Principal::personal("kay", "zode");
    engine.init_personal(&UserId::new("kay")).unwrap();
    seed_memory(
        &engine,
        &principal,
        "Prefer Rust for Noema.",
        vec!["rust".to_string()],
        vec!["Noema".to_string()],
    );

    let pack = engine
        .recall(RecallRequest {
            principal,
            query: "rust memory".to_string(),
            cwd: None,
            budget_tokens: 1200,
            host: "zode".to_string(),
        })
        .unwrap();

    assert_eq!(pack.tenant_id, TenantId::new("personal"));
    assert!(pack.to_markdown().contains("Relevant Memories"));
}

#[test]
fn engine_recall_enforces_budget_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    let principal = Principal::personal("kay", "zode");
    engine.init_personal(&UserId::new("kay")).unwrap();
    seed_memory(
        &engine,
        &principal,
        "Prefer Rust for Noema.",
        vec!["rust".to_string()],
        vec!["Noema".to_string()],
    );

    let tiny = engine
        .recall(RecallRequest {
            principal: principal.clone(),
            query: "rust memory".to_string(),
            cwd: None,
            budget_tokens: 1,
            host: "zode".to_string(),
        })
        .unwrap();
    assert_eq!(tiny.memories.len(), 0);

    let generous = engine
        .recall(RecallRequest {
            principal,
            query: "rust memory".to_string(),
            cwd: None,
            budget_tokens: 1200,
            host: "zode".to_string(),
        })
        .unwrap();
    assert_eq!(generous.memories.len(), 1);
}

#[test]
fn engine_carries_config() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    assert_eq!(
        engine.config().tenant.mode,
        crate::config::TenantMode::Personal
    );
}

#[test]
fn forget_tombstones_and_audits() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    let principal = Principal::personal("kay", "noema-cli");
    engine.init_personal(&UserId::new("kay")).unwrap();
    engine
        .submit_candidate(RememberRequest {
            principal: principal.clone(),
            text: "x".into(),
            scope: crate::memory::Scope::User,
            project_path: None,
            kind: crate::memory::MemoryKind::Preference,
            sensitivity: crate::sensitivity::SensitivityLevel::Internal,
            tags: vec![],
            entities: vec![],
            confidence: 1.0,
            importance: 0.5,
        })
        .unwrap();
    let pending = engine.review_list(&principal).unwrap();
    let cid = pending
        .first()
        .expect("candidate not queued — check default WritePolicy")
        .id
        .to_string();
    let id = match engine
        .review_decide(ReviewDecisionRequest {
            principal: principal.clone(),
            candidate_id: cid,
            action: ReviewAction::Accept,
        })
        .unwrap()
    {
        ReviewOutcome::Accepted { memory_id } => memory_id,
        _ => panic!("expected Accepted"),
    };

    let out = engine
        .forget(ForgetRequest {
            principal: principal.clone(),
            memory_id: id.clone(),
            hard: false,
        })
        .unwrap();
    assert_eq!(out.mode, "tombstoned");
    // Tombstoned memory must not appear in search results.
    let hits = engine
        .search(SearchRequest {
            principal,
            query: "x".into(),
            cwd: None,
        })
        .unwrap();
    assert!(hits.iter().all(|h| h.id != id));
}

#[test]
fn status_reports_personal_tenant() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    let principal = Principal::personal("kay", "noema-cli");
    let s = engine.status(&principal).unwrap();
    assert_eq!(s.tenant, "personal");
    assert!(s.ok);
}

#[test]
fn engine_profiled_recall_reports_phase_timings() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    let principal = Principal::personal("kay", "zode");
    engine.init_personal(&UserId::new("kay")).unwrap();
    seed_memory(
        &engine,
        &principal,
        "Prefer Rust for profiled Noema recall.",
        vec!["rust".to_string()],
        vec!["Noema".to_string()],
    );

    let profiled = engine
        .recall_profiled(RecallRequest {
            principal,
            query: "rust noema".to_string(),
            cwd: None,
            budget_tokens: 1200,
            host: "zode".to_string(),
        })
        .unwrap();

    assert_eq!(profiled.pack.memories.len(), 1);
    assert_eq!(profiled.timings.loaded_memories, 1);
    assert_eq!(profiled.timings.scored_memories, 1);
    assert!(profiled.timings.load_memories_us > 0.0);
    assert!(profiled.timings.score_memories_us > 0.0);
    assert!(profiled.timings.build_pack_us > 0.0);
}
