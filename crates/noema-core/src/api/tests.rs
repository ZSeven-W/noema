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
        })
        .unwrap();
    assert_eq!(tiny.memories.len(), 0);

    let generous = engine
        .recall(RecallRequest {
            principal,
            query: "rust memory".to_string(),
            cwd: None,
            budget_tokens: 1200,
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
fn forget_rejects_memory_owned_by_another_user() {
    use crate::ids::{MemoryId, ProjectId, TenantId};
    use crate::memory::{MemoryRecord, Scope, Visibility};
    use crate::store::write_memory;

    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    engine.init_personal(&UserId::new("kay")).unwrap();

    // Seed a project-scoped memory owned by a different user directly on disk.
    let tenant = TenantId::new("personal");
    let project = ProjectId::new("git_shared");
    let mut foreign = MemoryRecord::new_user_preference(
        MemoryId::new("mem_foreign"),
        tenant.clone(),
        UserId::new("other"),
        "Another user's project secret.",
    );
    foreign.scope = Scope::Project;
    foreign.project_id = Some(project.clone());
    foreign.visibility = Visibility::Project;
    let path = engine
        .paths
        .project_cortex_dir(&tenant, &project)
        .join("mem_foreign.md");
    write_memory(&path, &foreign).unwrap();

    // kay must not be able to forget a memory she neither owns nor has ACL on.
    let principal = Principal::personal("kay", "zode");
    let result = engine.forget(ForgetRequest {
        principal,
        memory_id: "mem_foreign".to_string(),
        hard: false,
    });
    assert!(result.is_err(), "must not forget another user's memory");
    assert!(path.exists(), "foreign memory file must remain on disk");
}

#[test]
fn policy_set_takes_effect_on_same_engine_and_disk() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    let principal = Principal::personal("kay", "noema-cli");

    let view = engine
        .policy_set(PolicySetRequest {
            principal: principal.clone(),
            write: Some(crate::config::WritePolicy::Manual),
        })
        .unwrap();
    // The returned view must reflect the change, not the stale prior policy.
    assert_eq!(view.write, crate::config::WritePolicy::Manual);
    // A subsequent read on the SAME long-lived engine must see the new policy.
    assert_eq!(
        engine.policy_get(&principal).unwrap().write,
        crate::config::WritePolicy::Manual
    );
    // And it must be persisted to disk.
    let reloaded = crate::config::NoemaConfig::load(dir.path()).unwrap();
    assert_eq!(reloaded.policy.write, crate::config::WritePolicy::Manual);
}

#[test]
fn recall_bumps_use_count_and_last_used_at() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    let principal = Principal::personal("kay", "zode");
    engine.init_personal(&UserId::new("kay")).unwrap();
    seed_memory(
        &engine,
        &principal,
        "Prefer Rust for Noema.",
        vec!["rust".to_string()],
        vec![],
    );

    let pack = engine
        .recall(RecallRequest {
            principal: principal.clone(),
            query: "rust noema".to_string(),
            cwd: None,
            budget_tokens: 1200,
        })
        .unwrap();
    assert_eq!(pack.memories.len(), 1);
    let id = pack.memories[0].id.to_string();

    // Serving a memory must record its usage so recency ranking has a signal.
    let path = engine
        .paths
        .user_cortex_dir(&principal.tenant_id, &principal.user_id)
        .join(format!("{id}.md"));
    let record = crate::store::read_memory(&path).unwrap();
    assert_eq!(record.use_count, 1);
    assert!(record.last_used_at.is_some());
}

#[test]
fn browse_navigates_catalog_to_entity_memories() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    let principal = Principal::personal("kay", "zode");
    engine.init_personal(&UserId::new("kay")).unwrap();
    seed_memory(
        &engine,
        &principal,
        "Melanie's favorite book is Charlotte's Web.",
        vec![],
        vec!["Melanie".to_string()],
    );
    seed_memory(
        &engine,
        &principal,
        "Melanie enjoys pottery on weekends.",
        vec![],
        vec!["Melanie".to_string()],
    );

    // "pottery" is absent from the query, but browsing the Melanie page returns
    // both of her memories — the catalog collapses the multi-hop lookup.
    let found = engine
        .browse(&principal, "What does Melanie like?", 8, None)
        .unwrap();
    let bodies: Vec<&str> = found.iter().map(|m| m.body.as_str()).collect();
    assert!(
        bodies.iter().any(|b| b.contains("Charlotte's Web")),
        "{bodies:?}"
    );
    assert!(bodies.iter().any(|b| b.contains("pottery")), "{bodies:?}");

    let catalog = engine.catalog(&principal, None).unwrap();
    assert!(catalog.to_markdown().contains("## Melanie"));
}

#[test]
fn submit_candidate_auto_fills_entities_when_absent() {
    let dir = tempfile::tempdir().unwrap();
    let engine = NoemaEngine::new(dir.path()).unwrap();
    let principal = Principal::personal("kay", "zode");
    engine.init_personal(&UserId::new("kay")).unwrap();
    // Caller supplies NO entities — the engine must extract them so the entity
    // recall boosts and the PageIndex catalog have something to work with.
    seed_memory(&engine, &principal, "王小明爱吃酸的", vec![], vec![]);

    let catalog = engine.catalog(&principal, None).unwrap();
    assert!(
        catalog.to_markdown().contains("## 王小明"),
        "auto-extracted entity should form a catalog page: {}",
        catalog.to_markdown()
    );
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
        })
        .unwrap();

    assert_eq!(profiled.pack.memories.len(), 1);
    assert_eq!(profiled.timings.loaded_memories, 1);
    assert_eq!(profiled.timings.scored_memories, 1);
    assert!(profiled.timings.load_memories_us > 0.0);
    assert!(profiled.timings.score_memories_us > 0.0);
    assert!(profiled.timings.build_pack_us > 0.0);
}
