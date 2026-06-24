mod types;
pub use types::*;

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use uuid::Uuid;

use crate::audit::{append_audit, AuditAction, AuditEvent};
use crate::config::{NoemaConfig, TenantMode};
use crate::error::{NoemaError, Result};
use crate::hippocampus::{
    append_candidate, append_decision, load_candidates, load_decisions, pending_candidates,
    Candidate, ReviewDecision,
};
use crate::ids::{CandidateId, MemoryId, ProjectId, TenantId, UserId};
use crate::lock::FileLock;
use crate::memory::{AccessLevel, MemoryRecord, MemoryStatus, RecallMode, Scope, Visibility};
use crate::memorypack::{MemoryPack, MemoryPackItem};
use crate::pageindex::PageIndex;
use crate::paths::NoemaPaths;
use crate::project::project_id_from_path;
use crate::review::{route_candidate, CandidateRoute};
use crate::sensitivity::{Principal, SensitivityLevel};
use crate::store::{read_memory, write_memory};

#[derive(Debug, Clone)]
pub struct NoemaEngine {
    pub root: PathBuf,
    pub paths: NoemaPaths,
    // Interior-mutable so policy_set takes effect on a long-lived engine (e.g.
    // the shared Arc<NoemaEngine> in noema-server / noema-mcp) without a restart.
    config: Arc<RwLock<NoemaConfig>>,
}

impl NoemaEngine {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let config = NoemaConfig::load(&root).unwrap_or_default();
        Ok(Self {
            paths: NoemaPaths::new(&root),
            root,
            config: Arc::new(RwLock::new(config)),
        })
    }

    pub fn from_config(config: &NoemaConfig) -> Result<Self> {
        Ok(Self {
            paths: NoemaPaths::new(&config.storage.local_root),
            root: config.storage.local_root.clone(),
            config: Arc::new(RwLock::new(config.clone())),
        })
    }

    /// Snapshot of the current config. Returns a clone because the live config
    /// is behind a lock; callers that need a stable view should hold the result.
    pub fn config(&self) -> NoemaConfig {
        self.config.read().expect("config lock poisoned").clone()
    }

    pub fn init_personal(&self, user: &UserId) -> Result<()> {
        self.paths.init_personal_layout(user)
    }

    pub fn recall(&self, request: RecallRequest) -> Result<MemoryPack> {
        let tenant = request.principal.tenant_id.clone();
        let user = request.principal.user_id.clone();
        let pack = self.recall_profiled(request)?.pack;
        // Record usage for the memories actually served so recency ranking has a
        // signal. Best-effort: a failure here must not fail the recall.
        self.bump_usage(&tenant, &user, &pack);
        Ok(pack)
    }

    /// Increment `use_count` and stamp `last_used_at` for each served memory.
    /// Runs under `tenant.lock` and re-reads each record so a concurrent
    /// `forget` cannot be lost or resurrected; tombstoned/erased memories are
    /// skipped. `recall_profiled` deliberately does NOT call this, keeping the
    /// benchmark's measurement path free of write side-effects.
    fn bump_usage(&self, tenant: &TenantId, user: &UserId, pack: &MemoryPack) {
        if pack.memories.is_empty() {
            return;
        }
        let tenant_dir = self.paths.tenant_dir(tenant);
        let Ok(_lock) = FileLock::exclusive(tenant_dir.join("tenant.lock")) else {
            return;
        };
        let now = time::OffsetDateTime::now_utc();
        for item in &pack.memories {
            let Some(path) = self.find_memory_path(tenant, user, &item.id) else {
                continue;
            };
            if let Ok(mut record) = read_memory(&path) {
                if record.status == MemoryStatus::Active {
                    record.use_count = record.use_count.saturating_add(1);
                    record.last_used_at = Some(now);
                    let _ = write_memory(&path, &record);
                }
            }
        }
    }

    pub fn recall_profiled(&self, request: RecallRequest) -> Result<ProfiledRecall> {
        let total_start = Instant::now();
        let tenant = request.principal.tenant_id.clone();
        let user = request.principal.user_id.clone();
        let project = request.cwd.as_deref().map(project_id_from_path);

        let load_start = Instant::now();
        let memories = load_recall_memories(&self.paths, &tenant, &user, project.as_ref())?;
        let load_memories_us = elapsed_us(load_start);

        let score_start = Instant::now();
        let scored = crate::fusion::fusion_recall(
            &request.query,
            &request.principal,
            project.as_ref(),
            &memories,
            crate::fusion::FusionOptions::default(),
        );
        let score_memories_us = elapsed_us(score_start);
        let scored_memories = scored.len();

        let build_start = Instant::now();
        let mut pack = MemoryPack::empty(tenant);
        let mut used_tokens = 0usize;
        for score in scored.into_iter().take(8) {
            if let Some(memory) = memories
                .iter()
                .find(|memory| memory.id.as_str() == score.id)
            {
                // Rough token estimate; stop once the budget would be exceeded.
                let item_tokens = memory.body.chars().count() / 4 + 1;
                if used_tokens + item_tokens > request.budget_tokens {
                    break;
                }
                used_tokens += item_tokens;
                pack.memories.push(MemoryPackItem {
                    id: memory.id.clone(),
                    scope: format!("{:?}", memory.scope).to_lowercase(),
                    kind: format!("{:?}", memory.kind).to_lowercase(),
                    text: Some(memory.body.clone()),
                    score: score.score,
                });
            }
        }
        let build_pack_us = elapsed_us(build_start);
        let total_us = elapsed_us(total_start);
        Ok(ProfiledRecall {
            pack,
            timings: RecallTimings {
                loaded_memories: memories.len(),
                scored_memories,
                load_memories_us,
                score_memories_us,
                build_pack_us,
                total_us,
            },
        })
    }

    /// Deterministic multi-hop recall: lexical seed, then walk links + shared
    /// entities outward up to `max_hops`. Returns the reached memories ordered by
    /// (hop-decayed) score. The no-LLM path to deep multi-hop that any host gets;
    /// agentic hosts can instead chain `search`/`browse`/`neighbors` themselves.
    pub fn recall_graph(
        &self,
        request: SearchRequest,
        max_hops: usize,
    ) -> Result<Vec<crate::recall::ScoredMemory>> {
        let tenant = request.principal.tenant_id.clone();
        let user = request.principal.user_id.clone();
        let project = request.cwd.as_deref().map(project_id_from_path);
        let memories = load_recall_memories(&self.paths, &tenant, &user, project.as_ref())?;
        Ok(crate::multihop::recall_multihop(
            &request.query,
            &request.principal,
            project.as_ref(),
            &memories,
            max_hops,
        ))
    }

    /// One graph hop from a memory: the recallable memories it links to or shares
    /// an entity with. The primitive an agentic host steps through for guided
    /// multi-hop retrieval.
    pub fn neighbors(
        &self,
        principal: &Principal,
        memory_id: &str,
        cwd: Option<&Path>,
    ) -> Result<Vec<MemoryRecord>> {
        let tenant = principal.tenant_id.clone();
        let user = principal.user_id.clone();
        let project = cwd.map(project_id_from_path);
        let memories = load_recall_memories(&self.paths, &tenant, &user, project.as_ref())?;
        Ok(crate::multihop::neighbors_of(
            memory_id,
            principal,
            project.as_ref(),
            &memories,
        ))
    }

    pub fn search(&self, request: SearchRequest) -> Result<Vec<crate::recall::ScoredMemory>> {
        let tenant = request.principal.tenant_id.clone();
        let user = request.principal.user_id.clone();
        let project = request.cwd.as_deref().map(project_id_from_path);
        let memories = load_recall_memories(&self.paths, &tenant, &user, project.as_ref())?;
        Ok(crate::recall::recall(
            &request.query,
            &request.principal,
            project.as_ref(),
            &memories,
        ))
    }

    pub fn explain(&self, request: ExplainRequest) -> Result<Option<crate::recall::ScoredMemory>> {
        let tenant = request.principal.tenant_id.clone();
        let user = request.principal.user_id.clone();
        let project = request.cwd.as_deref().map(project_id_from_path);
        let memories = load_recall_memories(&self.paths, &tenant, &user, project.as_ref())?;
        let Some(memory) = memories.iter().find(|m| m.id.as_str() == request.memory_id) else {
            return Ok(None);
        };
        Ok(crate::recall::explain_memory(
            &request.query,
            &request.principal,
            project.as_ref(),
            memory,
        ))
    }

    /// Build the PageIndex catalog (LLM-Wiki `index.md`) over the principal's
    /// memories — the "read this first" view that groups memories into entity /
    /// topic pages without any vectors.
    pub fn catalog(&self, principal: &Principal, cwd: Option<&Path>) -> Result<PageIndex> {
        let tenant = principal.tenant_id.clone();
        let user = principal.user_id.clone();
        let project = cwd.map(project_id_from_path);
        let memories = load_recall_memories(&self.paths, &tenant, &user, project.as_ref())?;
        Ok(PageIndex::build(&memories))
    }

    /// Navigate the catalog for `query` and return the memories on the best
    /// pages. Unlike `recall`, this surfaces every memory under a matched entity
    /// page even when it shares no lexical term with the query — the catalog's
    /// way of answering a multi-hop question in a single lookup.
    pub fn browse(
        &self,
        principal: &Principal,
        query: &str,
        limit: usize,
        cwd: Option<&Path>,
    ) -> Result<Vec<MemoryRecord>> {
        let tenant = principal.tenant_id.clone();
        let user = principal.user_id.clone();
        let project = cwd.map(project_id_from_path);
        let memories = load_recall_memories(&self.paths, &tenant, &user, project.as_ref())?;
        let ids = PageIndex::build(&memories).retrieve(query, limit);
        let by_id: std::collections::HashMap<&str, &MemoryRecord> =
            memories.iter().map(|m| (m.id.as_str(), m)).collect();
        Ok(ids
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).map(|m| (*m).clone()))
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    fn audit(
        &self,
        tenant: &TenantId,
        user: &UserId,
        scope: Scope,
        action: AuditAction,
        candidate_id: Option<CandidateId>,
        memory_id: Option<MemoryId>,
        reason: Option<String>,
    ) -> Result<()> {
        let mut event = AuditEvent::new(tenant.clone(), user.clone(), scope, action);
        event.candidate_id = candidate_id;
        event.memory_id = memory_id;
        event.reason = reason;
        append_audit(&self.paths.tenant_dir(tenant), &event)
    }

    pub fn submit_candidate(&self, request: RememberRequest) -> Result<SubmitOutcome> {
        let tenant = request.principal.tenant_id.clone();
        let user = request.principal.user_id.clone();
        self.init_personal(&user)?;
        let tenant_dir = self.paths.tenant_dir(&tenant);
        let _lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;

        let (mode, write_policy, auto_accept_max) = {
            let cfg = self.config.read().expect("config lock poisoned");
            (
                cfg.tenant.mode,
                cfg.policy.write,
                cfg.sensitive.auto_accept_max_sensitivity,
            )
        };
        reject_unsupported_personal_scope(mode, request.scope)?;
        reject_unsupported_personal_sensitivity(mode, request.sensitivity)?;

        let mut candidate = Candidate::new(
            CandidateId::new(format!("cand_{}", Uuid::new_v4())),
            request.text,
        );
        candidate.tenant_id = tenant.clone();
        candidate.owner_user_id = user.clone();
        candidate.scope = request.scope;
        candidate.kind = request.kind;
        candidate.sensitivity = request.sensitivity;
        candidate.confidence = request.confidence;
        candidate.importance = request.importance;
        candidate.tags = request.tags;
        candidate.entities = request.entities;
        // Ingest step: when the caller supplies no entities, extract them so the
        // entity recall boosts, dedup, and the PageIndex catalog have a signal.
        if candidate.entities.is_empty() {
            candidate.entities = crate::extraction::extract_entities(&candidate.body);
        }
        if request.scope == Scope::Project {
            let cwd = request
                .project_path
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            candidate.project_id = Some(project_id_from_path(&cwd));
        }

        let inbox = tenant_dir.join("hippocampus/inbox.jsonl");
        let active =
            load_recall_memories(&self.paths, &tenant, &user, candidate.project_id.as_ref())?;
        // Compute novelty against the live corpus so the auto-accept gate can
        // route near-duplicates to review instead of treating every candidate
        // as maximally novel.
        candidate.novelty = crate::review::candidate_novelty(&candidate, &active);
        match route_candidate(write_policy, auto_accept_max, &candidate, &active) {
            CandidateRoute::RejectSecret => {
                self.audit(
                    &tenant,
                    &user,
                    candidate.scope,
                    AuditAction::CandidateRejectedSecret,
                    Some(candidate.id.clone()),
                    None,
                    Some("secret sensitivity cannot enter review".into()),
                )?;
                Ok(SubmitOutcome::RejectedSecret)
            }
            CandidateRoute::PendingReview => {
                append_candidate(&inbox, &candidate)?;
                self.audit(
                    &tenant,
                    &user,
                    candidate.scope,
                    AuditAction::CandidateQueued,
                    Some(candidate.id.clone()),
                    None,
                    None,
                )?;
                Ok(SubmitOutcome::Queued {
                    candidate_id: candidate.id.to_string(),
                })
            }
            CandidateRoute::AutoAccept => {
                let memory = memory_from_candidate(&tenant, &user, &candidate);
                let path = memory_path(&self.paths, &tenant, &user, &memory)?;
                write_memory(&path, &memory)?;
                self.audit(
                    &tenant,
                    &user,
                    candidate.scope,
                    AuditAction::CandidateAutoAccepted,
                    Some(candidate.id.clone()),
                    Some(memory.id.clone()),
                    None,
                )?;
                self.audit(
                    &tenant,
                    &user,
                    candidate.scope,
                    AuditAction::MemoryWritten,
                    None,
                    Some(memory.id.clone()),
                    None,
                )?;
                Ok(SubmitOutcome::AutoAccepted {
                    memory_id: memory.id.to_string(),
                })
            }
        }
    }

    /// Finds the on-disk path for a memory by ID, checking user cortex then all
    /// project cortex directories under the tenant.
    fn find_memory_path(&self, tenant: &TenantId, user: &UserId, id: &MemoryId) -> Option<PathBuf> {
        let user_path = self
            .paths
            .user_cortex_dir(tenant, user)
            .join(format!("{id}.md"));
        if user_path.exists() {
            return Some(user_path);
        }
        let projects = self.paths.tenant_dir(tenant).join("projects");
        if let Ok(entries) = std::fs::read_dir(&projects) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("cortex").join(format!("{id}.md"));
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    pub fn forget(&self, request: ForgetRequest) -> Result<ForgetOutcome> {
        let tenant = request.principal.tenant_id.clone();
        let user = request.principal.user_id.clone();
        let tenant_dir = self.paths.tenant_dir(&tenant);
        let _lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
        let id = MemoryId::new(&request.memory_id);
        let path = self
            .find_memory_path(&tenant, &user, &id)
            .ok_or_else(|| NoemaError::NotFound(format!("memory not found: {id}")))?;
        let mut memory = read_memory(&path)?;
        // Authorize before mutating: find_memory_path scans every project cortex
        // under the tenant, so without this check any principal could tombstone
        // or hard-erase another user's project memory by id. Report NotFound on
        // denial so the API does not leak the existence of inaccessible memories.
        if !principal_can_modify(&memory, &request.principal) {
            return Err(NoemaError::NotFound(format!("memory not found: {id}")));
        }
        if request.hard {
            std::fs::remove_file(&path)?;
        } else {
            memory.status = MemoryStatus::Tombstoned;
            memory.recall_policy.mode = RecallMode::Never;
            write_memory(&path, &memory)?;
        }
        let mode = if request.hard {
            "hard-erased"
        } else {
            "tombstoned"
        };
        self.audit(
            &tenant,
            &user,
            memory.scope,
            AuditAction::MemoryTombstoned,
            None,
            Some(id.clone()),
            Some(mode.to_string()),
        )?;
        Ok(ForgetOutcome {
            memory_id: id.to_string(),
            mode: mode.to_string(),
        })
    }

    pub fn policy_get(&self, _principal: &Principal) -> Result<PolicyView> {
        let cfg = self.config.read().expect("config lock poisoned");
        Ok(PolicyView {
            write: cfg.policy.write,
            auto_accept_max_sensitivity: cfg.sensitive.auto_accept_max_sensitivity,
            external_llm_max_sensitivity: cfg.sensitive.external_llm_max_sensitivity,
        })
    }

    pub fn policy_set(&self, request: PolicySetRequest) -> Result<PolicyView> {
        // Enterprise gating: only reviewer/admin roles may change policy.
        let mode = self
            .config
            .read()
            .expect("config lock poisoned")
            .tenant
            .mode;
        if mode == crate::config::TenantMode::Enterprise {
            use crate::policy::{AclDecision, EnterprisePolicy};
            if EnterprisePolicy::default().can_write_org_memory(&request.principal.roles)
                == AclDecision::Deny
            {
                return Err(NoemaError::PolicyDenied(
                    "policy change requires reviewer role".into(),
                ));
            }
        }
        let tenant_dir = self.paths.tenant_dir(&request.principal.tenant_id);
        let _lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
        // Apply the change to the live config AND persist it, so subsequent
        // operations on this same engine observe the new policy and the returned
        // view is the updated value rather than the stale prior one.
        let view = {
            let mut cfg = self.config.write().expect("config lock poisoned");
            if let Some(write) = request.write {
                cfg.policy.write = write;
            }
            std::fs::write(self.root.join("config.toml"), cfg.to_toml()?)?;
            PolicyView {
                write: cfg.policy.write,
                auto_accept_max_sensitivity: cfg.sensitive.auto_accept_max_sensitivity,
                external_llm_max_sensitivity: cfg.sensitive.external_llm_max_sensitivity,
            }
        };
        self.audit(
            &request.principal.tenant_id,
            &request.principal.user_id,
            Scope::User,
            AuditAction::PolicyChanged,
            None,
            None,
            None,
        )?;
        Ok(view)
    }

    pub fn status(&self, principal: &Principal) -> Result<StatusView> {
        let (mode, write_policy) = {
            let cfg = self.config.read().expect("config lock poisoned");
            (cfg.tenant.mode, cfg.policy.write)
        };
        Ok(StatusView {
            tenant: principal.tenant_id.to_string(),
            user: principal.user_id.to_string(),
            mode: format!("{mode:?}").to_lowercase(),
            write_policy,
            ok: true,
        })
    }

    fn hip_dir(&self, tenant: &TenantId) -> PathBuf {
        self.paths.tenant_dir(tenant).join("hippocampus")
    }

    pub fn review_list(&self, principal: &Principal) -> Result<Vec<Candidate>> {
        let hip = self.hip_dir(&principal.tenant_id);
        let candidates = load_candidates(&hip.join("inbox.jsonl"))?;
        let decisions = load_decisions(&hip.join("decisions.jsonl"))?;
        Ok(pending_candidates(&candidates, &decisions))
    }

    pub fn review_decide(&self, request: ReviewDecisionRequest) -> Result<ReviewOutcome> {
        let tenant = request.principal.tenant_id.clone();
        let user = request.principal.user_id.clone();
        let tenant_dir = self.paths.tenant_dir(&tenant);
        let _lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
        let hip = self.hip_dir(&tenant);
        let id = CandidateId::new(request.candidate_id);
        let candidate = self
            .review_list(&request.principal)?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| NoemaError::NotFound("candidate not found or already decided".into()))?;
        let decisions_path = hip.join("decisions.jsonl");

        match request.action {
            ReviewAction::Accept => {
                let memory = memory_from_candidate(&tenant, &user, &candidate);
                let path = memory_path(&self.paths, &tenant, &user, &memory)?;
                // Write memory BEFORE recording the decision: if the write fails
                // the candidate stays pending and can be retried without data loss.
                write_memory(&path, &memory)?;
                append_decision(
                    &decisions_path,
                    &ReviewDecision::Accept {
                        candidate_id: id.clone(),
                    },
                )?;
                self.audit(
                    &tenant,
                    &user,
                    candidate.scope,
                    AuditAction::CandidateAccepted,
                    Some(id.clone()),
                    Some(memory.id.clone()),
                    None,
                )?;
                self.audit(
                    &tenant,
                    &user,
                    candidate.scope,
                    AuditAction::MemoryWritten,
                    None,
                    Some(memory.id.clone()),
                    None,
                )?;
                Ok(ReviewOutcome::Accepted {
                    memory_id: memory.id.to_string(),
                })
            }
            ReviewAction::Reject { reason } => {
                append_decision(
                    &decisions_path,
                    &ReviewDecision::Reject {
                        candidate_id: id.clone(),
                        reason: reason.clone(),
                    },
                )?;
                self.audit(
                    &tenant,
                    &user,
                    candidate.scope,
                    AuditAction::CandidateRejected,
                    Some(id.clone()),
                    None,
                    Some(reason),
                )?;
                Ok(ReviewOutcome::Rejected)
            }
            ReviewAction::Edit { body, reason } => {
                append_decision(
                    &decisions_path,
                    &ReviewDecision::Edit {
                        candidate_id: id.clone(),
                        body,
                        reason: reason.clone(),
                    },
                )?;
                self.audit(
                    &tenant,
                    &user,
                    candidate.scope,
                    AuditAction::CandidateEdited,
                    Some(id.clone()),
                    None,
                    Some(reason),
                )?;
                Ok(ReviewOutcome::Edited)
            }
            ReviewAction::Merge {
                target_memory_id,
                reason,
            } => {
                let target = MemoryId::new(target_memory_id);
                let active = load_recall_memories(
                    &self.paths,
                    &tenant,
                    &user,
                    candidate.project_id.as_ref(),
                )?;
                if !active.iter().any(|m| m.id.as_str() == target.as_str()) {
                    return Err(NoemaError::NotFound("target memory not found".into()));
                }
                append_decision(
                    &decisions_path,
                    &ReviewDecision::Merge {
                        candidate_id: id.clone(),
                        target_memory_id: target.clone(),
                        reason: reason.clone(),
                    },
                )?;
                self.audit(
                    &tenant,
                    &user,
                    candidate.scope,
                    AuditAction::CandidateMerged,
                    Some(id.clone()),
                    Some(target),
                    Some(reason),
                )?;
                Ok(ReviewOutcome::Merged)
            }
        }
    }
}

fn reject_unsupported_personal_scope(mode: TenantMode, scope: Scope) -> Result<()> {
    if mode == TenantMode::Personal && matches!(scope, Scope::Team | Scope::Org) {
        return Err(NoemaError::PolicyDenied(
            "team and org scope require enterprise mode".into(),
        ));
    }
    Ok(())
}

fn reject_unsupported_personal_sensitivity(mode: TenantMode, s: SensitivityLevel) -> Result<()> {
    if mode == TenantMode::Personal
        && matches!(
            s,
            SensitivityLevel::Confidential | SensitivityLevel::Restricted
        )
    {
        return Err(NoemaError::PolicyDenied(
            "confidential and restricted sensitivity require enterprise mode".into(),
        ));
    }
    Ok(())
}

fn elapsed_us(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1_000_000.0
}

/// Whether a principal is allowed to tombstone or hard-erase a memory. Owners
/// may always modify their own memories; non-owners may modify a shared project
/// memory only with an explicit Write or Admin ACL grant (by user id or group).
fn principal_can_modify(memory: &MemoryRecord, principal: &Principal) -> bool {
    if memory.tenant_id != principal.tenant_id {
        return false;
    }
    if memory.owner_user_id == principal.user_id {
        return true;
    }
    memory.scope == Scope::Project
        && memory.acl.iter().any(|entry| {
            matches!(entry.access, AccessLevel::Write | AccessLevel::Admin)
                && (entry.principal == principal.user_id.as_str()
                    || principal
                        .groups
                        .iter()
                        .any(|group| entry.principal == group.as_str()))
        })
}

fn memory_from_candidate(tenant: &TenantId, user: &UserId, candidate: &Candidate) -> MemoryRecord {
    let memory_id = MemoryId::new(candidate.id.as_str().replacen("cand_", "mem_", 1));
    let mut memory = MemoryRecord::new_user_preference(
        memory_id,
        tenant.clone(),
        user.clone(),
        candidate.body.clone(),
    );
    memory.scope = candidate.scope;
    memory.project_id = candidate.project_id.clone();
    memory.team_id = candidate.team_id.clone();
    memory.kind = candidate.kind;
    memory.visibility = match candidate.scope {
        Scope::Project => Visibility::Project,
        Scope::Team => Visibility::Team,
        Scope::Org => Visibility::Org,
        Scope::User => Visibility::Private,
    };
    memory.confidence = candidate.confidence;
    memory.importance = candidate.importance;
    memory.sensitivity = candidate.sensitivity;
    if !candidate.sensitivity.can_auto_accept() {
        memory.recall_policy.mode = RecallMode::Never;
    }
    memory.data_classes = candidate.data_classes.clone();
    memory.tags = candidate.tags.clone();
    memory.entities = candidate.entities.clone();
    memory.source = candidate.source.clone();
    memory
}

fn memory_path(
    paths: &NoemaPaths,
    tenant: &TenantId,
    user: &UserId,
    memory: &MemoryRecord,
) -> Result<PathBuf> {
    let dir = match memory.scope {
        Scope::Project => {
            let project = memory.project_id.as_ref().ok_or_else(|| {
                NoemaError::InvalidRecord("project memory missing project_id".into())
            })?;
            paths.project_cortex_dir(tenant, project)
        }
        Scope::User | Scope::Team | Scope::Org => paths.user_cortex_dir(tenant, user),
    };
    Ok(dir.join(format!("{}.md", memory.id)))
}

fn load_recall_memories(
    paths: &NoemaPaths,
    tenant: &TenantId,
    user: &UserId,
    project: Option<&ProjectId>,
) -> Result<Vec<MemoryRecord>> {
    let mut out = Vec::new();
    load_memory_dir(&paths.user_cortex_dir(tenant, user), &mut out)?;
    if let Some(project) = project {
        load_memory_dir(&paths.project_cortex_dir(tenant, project), &mut out)?;
    }
    Ok(out)
}

fn load_memory_dir(dir: &Path, out: &mut Vec<MemoryRecord>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|value| value.to_str()) == Some("md") {
            out.push(read_memory(&entry.path())?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
