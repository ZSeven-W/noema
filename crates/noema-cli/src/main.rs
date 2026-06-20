use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use noema_core::audit::{append_audit, AuditAction, AuditEvent};
use noema_core::config::{NoemaConfig, TenantMode};
use noema_core::hippocampus::{
    append_candidate, append_decision, load_candidates, load_decisions, pending_candidates,
    Candidate, ReviewDecision,
};
use noema_core::ids::{CandidateId, MemoryId, ProjectId, TenantId, UserId};
use noema_core::lock::FileLock;
use noema_core::memory::{MemoryKind, MemoryRecord, RecallMode, Scope, Visibility};
use noema_core::paths::NoemaPaths;
use noema_core::project::project_id_from_path;
use noema_core::recall::{explain_memory, recall};
use noema_core::review::{route_candidate, CandidateRoute};
use noema_core::sensitivity::{Principal, SensitivityLevel};
use noema_core::store::{read_memory, write_memory};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "noema")]
#[command(about = "Noema local memory system")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    Remember {
        text: String,
        #[arg(long, default_value = "user")]
        scope: String,
        #[arg(long, default_value = "preference")]
        kind: String,
        #[arg(long, default_value = "internal")]
        sensitivity: String,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long = "entity")]
        entities: Vec<String>,
        #[arg(long, default_value_t = 1.0)]
        confidence: f32,
        #[arg(long, default_value_t = 0.5)]
        importance: f32,
    },
    Review,
    Edit {
        candidate_id: String,
        #[arg(long)]
        body: String,
        #[arg(long)]
        reason: String,
    },
    Merge {
        candidate_id: String,
        target_memory_id: String,
        #[arg(long)]
        reason: String,
    },
    Accept {
        candidate_id: String,
    },
    Reject {
        candidate_id: String,
        #[arg(long)]
        reason: String,
    },
    Search {
        query: String,
    },
    Explain {
        memory_id: String,
        #[arg(long)]
        query: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = if matches!(&cli.command, Command::Init) {
        NoemaConfig::default()
    } else {
        NoemaConfig::load_or_default()?
    };
    let paths = NoemaPaths::new(&cfg.storage.local_root);
    let user = UserId::new(cfg.tenant.default_user_id.clone());
    let tenant = TenantId::new(cfg.tenant.id.clone());
    let tenant_dir = paths.tenant_dir(&tenant);

    match cli.command {
        Command::Init => {
            paths.init_personal_layout(&user)?;
            std::fs::write(cfg.storage.local_root.join("config.toml"), cfg.to_toml()?)?;
            println!("initialized {}", cfg.storage.local_root.display());
        }
        Command::Remember {
            text,
            scope,
            kind,
            sensitivity,
            tags,
            entities,
            confidence,
            importance,
        } => {
            paths.init_personal_layout(&user)?;
            let _tenant_lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
            let current_project = project_id_from_path(&std::env::current_dir()?);
            let mut candidate =
                Candidate::new(CandidateId::new(format!("cand_{}", Uuid::new_v4())), text);
            candidate.tenant_id = tenant.clone();
            candidate.owner_user_id = user.clone();
            candidate.scope = parse_scope(&scope)?;
            reject_unsupported_personal_scope(cfg.tenant.mode, candidate.scope)?;
            candidate.kind = parse_kind(&kind)?;
            candidate.sensitivity = parse_sensitivity(&sensitivity)?;
            reject_unsupported_personal_sensitivity(cfg.tenant.mode, candidate.sensitivity)?;
            candidate.confidence = confidence;
            candidate.importance = importance;
            candidate.tags = tags;
            candidate.entities = entities;
            if candidate.scope == Scope::Project {
                candidate.project_id = Some(current_project);
            }

            let inbox = paths.tenant_dir(&tenant).join("hippocampus/inbox.jsonl");
            let active_memories =
                load_recall_memories(&paths, &tenant, &user, candidate.project_id.as_ref())?;
            match route_candidate(
                cfg.policy.write,
                cfg.sensitive.auto_accept_max_sensitivity,
                &candidate,
                &active_memories,
            ) {
                CandidateRoute::RejectSecret => {
                    append_audit_event(
                        &tenant_dir,
                        &tenant,
                        &user,
                        candidate.scope,
                        AuditAction::CandidateRejectedSecret,
                        Some(candidate.id.clone()),
                        None,
                        Some("secret sensitivity cannot enter review".to_string()),
                    )?;
                    return Err(anyhow!("secret candidates are rejected before review"));
                }
                CandidateRoute::PendingReview => {
                    append_candidate(&inbox, &candidate)?;
                    append_audit_event(
                        &tenant_dir,
                        &tenant,
                        &user,
                        candidate.scope,
                        AuditAction::CandidateQueued,
                        Some(candidate.id.clone()),
                        None,
                        None,
                    )?;
                    println!("queued {}", candidate.id);
                }
                CandidateRoute::AutoAccept => {
                    let memory = memory_from_candidate(&tenant, &user, &candidate);
                    let path = memory_path(&paths, &tenant, &user, &memory)?;
                    write_memory(&path, &memory)?;
                    append_audit_event(
                        &tenant_dir,
                        &tenant,
                        &user,
                        candidate.scope,
                        AuditAction::CandidateAutoAccepted,
                        Some(candidate.id.clone()),
                        Some(memory.id.clone()),
                        None,
                    )?;
                    append_audit_event(
                        &tenant_dir,
                        &tenant,
                        &user,
                        candidate.scope,
                        AuditAction::MemoryWritten,
                        None,
                        Some(memory.id.clone()),
                        None,
                    )?;
                    println!("accepted {}", memory.id);
                }
            }
        }
        Command::Review => {
            let hip = paths.tenant_dir(&tenant).join("hippocampus");
            let candidates = load_candidates(&hip.join("inbox.jsonl"))?;
            let decisions = load_decisions(&hip.join("decisions.jsonl"))?;
            let pending = pending_candidates(&candidates, &decisions);
            for candidate in pending {
                println!("{} {}", candidate.id, candidate.body);
            }
        }
        Command::Edit {
            candidate_id,
            body,
            reason,
        } => {
            let _tenant_lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
            let hip = paths.tenant_dir(&tenant).join("hippocampus");
            let id = CandidateId::new(candidate_id);
            let candidates = load_candidates(&hip.join("inbox.jsonl"))?;
            let decisions = load_decisions(&hip.join("decisions.jsonl"))?;
            let candidate = pending_candidates(&candidates, &decisions)
                .into_iter()
                .find(|candidate| candidate.id == id)
                .ok_or_else(|| anyhow!("candidate not found or already decided"))?;
            append_decision(
                &hip.join("decisions.jsonl"),
                &ReviewDecision::Edit {
                    candidate_id: id.clone(),
                    body,
                    reason: reason.clone(),
                },
            )?;
            append_audit_event(
                &tenant_dir,
                &tenant,
                &user,
                candidate.scope,
                AuditAction::CandidateEdited,
                Some(id.clone()),
                None,
                Some(reason),
            )?;
            println!("edited {}", id);
        }
        Command::Merge {
            candidate_id,
            target_memory_id,
            reason,
        } => {
            let _tenant_lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
            let hip = paths.tenant_dir(&tenant).join("hippocampus");
            let id = CandidateId::new(candidate_id);
            let target = MemoryId::new(target_memory_id);
            let candidates = load_candidates(&hip.join("inbox.jsonl"))?;
            let decisions = load_decisions(&hip.join("decisions.jsonl"))?;
            let candidate = pending_candidates(&candidates, &decisions)
                .into_iter()
                .find(|candidate| candidate.id == id)
                .ok_or_else(|| anyhow!("candidate not found or already decided"))?;
            let active_memories =
                load_recall_memories(&paths, &tenant, &user, candidate.project_id.as_ref())?;
            if !active_memories
                .iter()
                .any(|memory| memory.id.as_str() == target.as_str())
            {
                return Err(anyhow!("target memory not found"));
            }
            // P0 merge records the duplicate relationship and removes the candidate
            // from review. Content consolidation into the target memory is deferred.
            append_decision(
                &hip.join("decisions.jsonl"),
                &ReviewDecision::Merge {
                    candidate_id: id.clone(),
                    target_memory_id: target.clone(),
                    reason: reason.clone(),
                },
            )?;
            append_audit_event(
                &tenant_dir,
                &tenant,
                &user,
                candidate.scope,
                AuditAction::CandidateMerged,
                Some(id.clone()),
                Some(target),
                Some(reason),
            )?;
            println!("merged {}", id);
        }
        Command::Accept { candidate_id } => {
            let _tenant_lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
            let hip = paths.tenant_dir(&tenant).join("hippocampus");
            let candidates = load_candidates(&hip.join("inbox.jsonl"))?;
            let decisions = load_decisions(&hip.join("decisions.jsonl"))?;
            let id = CandidateId::new(candidate_id);
            let pending = pending_candidates(&candidates, &decisions);
            let candidate = pending
                .iter()
                .find(|candidate| candidate.id == id)
                .ok_or_else(|| anyhow!("candidate not found or already decided"))?;
            append_decision(
                &hip.join("decisions.jsonl"),
                &ReviewDecision::Accept {
                    candidate_id: id.clone(),
                },
            )?;
            let memory = memory_from_candidate(&tenant, &user, candidate);
            let path = memory_path(&paths, &tenant, &user, &memory)?;
            write_memory(&path, &memory)?;
            append_audit_event(
                &tenant_dir,
                &tenant,
                &user,
                candidate.scope,
                AuditAction::CandidateAccepted,
                Some(id.clone()),
                Some(memory.id.clone()),
                None,
            )?;
            append_audit_event(
                &tenant_dir,
                &tenant,
                &user,
                candidate.scope,
                AuditAction::MemoryWritten,
                None,
                Some(memory.id.clone()),
                None,
            )?;
            println!("accepted {}", id);
        }
        Command::Reject {
            candidate_id,
            reason,
        } => {
            let _tenant_lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
            let hip = paths.tenant_dir(&tenant).join("hippocampus");
            let id = CandidateId::new(candidate_id);
            let candidates = load_candidates(&hip.join("inbox.jsonl"))?;
            let decisions = load_decisions(&hip.join("decisions.jsonl"))?;
            let pending = pending_candidates(&candidates, &decisions);
            let candidate = pending
                .iter()
                .find(|candidate| candidate.id == id)
                .ok_or_else(|| anyhow!("candidate not found or already decided"))?;
            append_decision(
                &hip.join("decisions.jsonl"),
                &ReviewDecision::Reject {
                    candidate_id: id.clone(),
                    reason: reason.clone(),
                },
            )?;
            append_audit_event(
                &tenant_dir,
                &tenant,
                &user,
                candidate.scope,
                AuditAction::CandidateRejected,
                Some(id.clone()),
                None,
                Some(reason),
            )?;
            println!("rejected {}", id);
        }
        Command::Search { query } => {
            let current_project = project_id_from_path(&std::env::current_dir()?);
            let principal = personal_principal(&tenant, &user);
            let memories = load_recall_memories(&paths, &tenant, &user, Some(&current_project))?;
            for scored in recall(&query, &principal, Some(&current_project), &memories) {
                println!("{:.3} {}", scored.score, scored.id);
            }
        }
        Command::Explain { memory_id, query } => {
            let current_project = project_id_from_path(&std::env::current_dir()?);
            let principal = personal_principal(&tenant, &user);
            let memories = load_recall_memories(&paths, &tenant, &user, Some(&current_project))?;
            let memory = memories
                .iter()
                .find(|memory| memory.id.as_str() == memory_id)
                .ok_or_else(|| anyhow!("memory not found"))?;
            if let Some(scored) = explain_memory(&query, &principal, Some(&current_project), memory)
            {
                println!("{}", scored.explanation.join("\n"));
            }
        }
    }
    Ok(())
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
            let project = memory
                .project_id
                .as_ref()
                .ok_or_else(|| anyhow!("project memory missing project_id"))?;
            paths.project_cortex_dir(tenant, project)
        }
        Scope::User | Scope::Team | Scope::Org => paths.user_cortex_dir(tenant, user),
    };
    Ok(dir.join(format!("{}.md", memory.id)))
}

fn append_audit_event(
    tenant_dir: &Path,
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
    append_audit(tenant_dir, &event)?;
    Ok(())
}

fn personal_principal(tenant: &TenantId, user: &UserId) -> Principal {
    let mut principal = Principal::personal(user.as_str(), "noema-cli");
    principal.tenant_id = tenant.clone();
    principal
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
        if entry.path().extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(read_memory(&entry.path())?);
        }
    }
    Ok(())
}

fn parse_scope(value: &str) -> Result<Scope> {
    match value {
        "user" => Ok(Scope::User),
        "project" => Ok(Scope::Project),
        "team" => Ok(Scope::Team),
        "org" => Ok(Scope::Org),
        _ => Err(anyhow!("invalid scope: {value}")),
    }
}

fn reject_unsupported_personal_scope(mode: TenantMode, scope: Scope) -> Result<()> {
    if mode == TenantMode::Personal && matches!(scope, Scope::Team | Scope::Org) {
        return Err(anyhow!("team and org scope require enterprise mode"));
    }
    Ok(())
}

fn reject_unsupported_personal_sensitivity(
    mode: TenantMode,
    sensitivity: SensitivityLevel,
) -> Result<()> {
    if mode == TenantMode::Personal
        && matches!(
            sensitivity,
            SensitivityLevel::Confidential | SensitivityLevel::Restricted
        )
    {
        return Err(anyhow!(
            "confidential and restricted sensitivity require enterprise mode"
        ));
    }
    Ok(())
}

fn parse_kind(value: &str) -> Result<MemoryKind> {
    match value {
        "preference" => Ok(MemoryKind::Preference),
        "decision" => Ok(MemoryKind::Decision),
        "constraint" => Ok(MemoryKind::Constraint),
        "fact" => Ok(MemoryKind::Fact),
        "reference" => Ok(MemoryKind::Reference),
        "workflow" => Ok(MemoryKind::Workflow),
        "warning" => Ok(MemoryKind::Warning),
        _ => Err(anyhow!("invalid memory kind: {value}")),
    }
}

fn parse_sensitivity(value: &str) -> Result<SensitivityLevel> {
    match value {
        "public" => Ok(SensitivityLevel::Public),
        "internal" => Ok(SensitivityLevel::Internal),
        "confidential" => Ok(SensitivityLevel::Confidential),
        "restricted" => Ok(SensitivityLevel::Restricted),
        "secret" => Ok(SensitivityLevel::Secret),
        _ => Err(anyhow!("invalid sensitivity: {value}")),
    }
}
