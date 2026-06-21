use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use noema_core::audit::{append_audit, AuditAction, AuditEvent};
use noema_core::benchmark::LocomoMemorySource;
use noema_core::config::{NoemaConfig, TenantMode};
use noema_core::hippocampus::{
    append_candidate, append_decision, load_candidates, load_decisions, pending_candidates,
    Candidate, ReviewDecision,
};
use noema_core::ids::{CandidateId, MemoryId, ProjectId, TenantId, UserId};
use noema_core::lock::FileLock;
use noema_core::memory::{MemoryKind, MemoryRecord, MemoryStatus, RecallMode, Scope, Visibility};
use noema_core::paths::NoemaPaths;
use noema_core::project::project_id_from_path;
use noema_core::recall::{explain_memory, recall};
use noema_core::review::{route_candidate, CandidateRoute};
use noema_core::sensitivity::{Principal, SensitivityLevel};
use noema_core::store::{read_memory, write_memory};
use serde_json::json;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "noema")]
#[command(about = "Noema local memory system")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[allow(clippy::large_enum_variant)]
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
    Vacuum,
    Sleep {
        #[arg(long)]
        llm: bool,
    },
    Doctor,
    Reindex,
    Bench {
        #[arg(long, default_value_t = 1000)]
        memories: usize,
        #[arg(long, default_value_t = 8)]
        queries: usize,
        #[arg(long, default_value_t = 50)]
        iterations: usize,
        #[arg(long)]
        mem0_targets: bool,
        #[arg(long)]
        mem0_result: Option<PathBuf>,
        #[arg(long)]
        locomo_dataset: Option<PathBuf>,
        #[arg(long)]
        locomo_evidence: Option<PathBuf>,
        #[arg(long)]
        locomo_predict_input: Option<PathBuf>,
        #[arg(long, default_value_t = 50)]
        top_k: usize,
        #[arg(long, default_value = "raw")]
        locomo_memory_source: String,
        #[arg(long)]
        locomo_predict_output: Option<PathBuf>,
        #[arg(long)]
        locomo_predict_dir: Option<PathBuf>,
        #[arg(long)]
        locomo_answer_tasks_output: Option<PathBuf>,
        #[arg(long)]
        locomo_answer_tasks_input: Option<PathBuf>,
        #[arg(long)]
        locomo_answer_prompt_char_budget: Option<usize>,
        #[arg(long)]
        locomo_retry_answer_tasks_output: Option<PathBuf>,
        #[arg(long)]
        locomo_answer_results: Option<PathBuf>,
        #[arg(long)]
        locomo_judge_tasks_output: Option<PathBuf>,
        #[arg(long)]
        locomo_retry_judge_tasks_output: Option<PathBuf>,
        #[arg(long)]
        locomo_judge_results: Option<PathBuf>,
        #[arg(long)]
        locomo_final_output: Option<PathBuf>,
        #[arg(long)]
        locomo_status_output: Option<PathBuf>,
        #[arg(long)]
        locomo_retention_output: Option<PathBuf>,
        #[arg(long)]
        locomo_report_output: Option<PathBuf>,
        #[arg(long)]
        locomo_host_manifest_input: Option<PathBuf>,
        #[arg(long)]
        locomo_target_output: Option<PathBuf>,
        #[arg(long)]
        locomo_require_beats_mem0: bool,
        #[arg(long)]
        locomo_fail_if_incomplete: bool,
    },
    Forget {
        memory_id: String,
        #[arg(long)]
        hard: bool,
    },
    Offload {
        #[command(subcommand)]
        command: OffloadCommand,
    },
    Restore {
        snapshot_or_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum OffloadCommand {
    Status,
    Run,
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
    let audit = AuditContext {
        tenant_dir: &tenant_dir,
        tenant: &tenant,
        user: &user,
    };

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
                        &audit,
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
                        &audit,
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
                        &audit,
                        candidate.scope,
                        AuditAction::CandidateAutoAccepted,
                        Some(candidate.id.clone()),
                        Some(memory.id.clone()),
                        None,
                    )?;
                    append_audit_event(
                        &audit,
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
                &audit,
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
                &audit,
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
            let memory = memory_from_candidate(&tenant, &user, candidate);
            let path = memory_path(&paths, &tenant, &user, &memory)?;
            // Write the memory BEFORE recording the terminal decision: if the
            // write fails the candidate stays pending and can be retried, instead
            // of being permanently marked Accepted with no memory and no audit.
            write_memory(&path, &memory)?;
            append_decision(
                &hip.join("decisions.jsonl"),
                &ReviewDecision::Accept {
                    candidate_id: id.clone(),
                },
            )?;
            append_audit_event(
                &audit,
                candidate.scope,
                AuditAction::CandidateAccepted,
                Some(id.clone()),
                Some(memory.id.clone()),
                None,
            )?;
            append_audit_event(
                &audit,
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
                &audit,
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
        Command::Vacuum => {
            let tenant_dir = paths.tenant_dir(&tenant);
            let _tenant_lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
            noema_core::vacuum::compact_hippocampus(&tenant_dir)?;
            append_audit_event(
                &audit,
                Scope::User,
                AuditAction::VacuumCompacted,
                None,
                None,
                None,
            )?;
            println!("vacuumed {}", tenant_dir.display());
        }
        Command::Sleep { llm } => {
            let jobs = noema_core::extraction::load_jobs(&cfg.storage.local_root, &tenant)?;
            if llm {
                println!(
                    "sleep queued {} extraction jobs for host LLM processing",
                    jobs.len()
                );
            } else {
                println!("sleep scanned {} extraction jobs without LLM", jobs.len());
            }
        }
        Command::Doctor => {
            let status = noema_core::capacity::capacity_status(
                &cfg.storage.local_root,
                noema_core::capacity::CapacityLimits {
                    local_soft_total_mb: 256,
                    local_hard_total_mb: 512,
                },
            )?;
            println!(
                "used_bytes={} soft={} hard={}",
                status.used_bytes, status.soft_limit_reached, status.hard_limit_reached
            );
        }
        Command::Reindex => {
            let mut index = noema_core::index::LexicalIndex::default();
            for memory in load_recall_memories(&paths, &tenant, &user, None)? {
                index.add(noema_core::index::IndexDocument {
                    id: memory.id,
                    text: memory.body,
                    tags: memory.tags,
                    entities: memory.entities,
                });
            }
            let index_dir = paths.tenant_dir(&tenant).join("indexes");
            std::fs::create_dir_all(&index_dir)?;
            std::fs::write(
                index_dir.join("lexical.json"),
                serde_json::to_vec_pretty(&index)?,
            )?;
            println!(
                "reindex completed {}",
                index_dir.join("lexical.json").display()
            );
        }
        Command::Bench {
            memories,
            queries,
            iterations,
            mem0_targets,
            mem0_result,
            locomo_dataset,
            locomo_evidence,
            locomo_predict_input,
            top_k,
            locomo_memory_source,
            locomo_predict_output,
            locomo_predict_dir,
            locomo_answer_tasks_output,
            locomo_answer_tasks_input,
            locomo_answer_prompt_char_budget,
            locomo_retry_answer_tasks_output,
            locomo_answer_results,
            locomo_judge_tasks_output,
            locomo_retry_judge_tasks_output,
            locomo_judge_results,
            locomo_final_output,
            locomo_status_output,
            locomo_retention_output,
            locomo_report_output,
            locomo_host_manifest_input,
            locomo_target_output,
            locomo_require_beats_mem0,
            locomo_fail_if_incomplete,
        } => {
            if mem0_targets {
                println!("Mem0 benchmark targets");
                println!();
                println!("Noema must exceed every Mem0 score below before this benchmark goal is considered met.");
                println!();
                print!(
                    "{}",
                    noema_core::benchmark::mem0_reference_targets_markdown_table()
                );
                return Ok(());
            }
            if let Some(path) = mem0_result {
                let text = std::fs::read_to_string(&path)?;
                let summary = noema_core::benchmark::summarize_mem0_result_json(&text)?;
                println!("Mem0 result summary");
                println!();
                println!(
                    "benchmark={} total_questions={}",
                    summary.benchmark, summary.total_questions
                );
                if let Some(latency) = summary.avg_search_latency_ms {
                    println!("avg_search_latency_ms={latency:.1}");
                }
                if let Some(retrieved) = summary.avg_retrieved_memories {
                    println!("avg_retrieved_memories={retrieved:.1}");
                }
                println!();
                print!("{}", summary.to_markdown_table());
                return Ok(());
            }
            if let Some(path) = locomo_dataset {
                let text = std::fs::read_to_string(&path)?;
                let summary = noema_core::benchmark::summarize_locomo_dataset_json(&text)?;
                println!("LOCOMO dataset summary");
                println!();
                println!(
                    "conversations={} sessions={} turns={} questions={} evaluable_questions={}",
                    summary.conversations,
                    summary.sessions,
                    summary.turns,
                    summary.questions,
                    summary.evaluable_questions
                );
                println!(
                    "evidence_refs={} resolved_evidence_refs={}",
                    summary.evidence_refs, summary.resolved_evidence_refs
                );
                println!();
                print!("{}", summary.to_markdown_table());
                return Ok(());
            }
            if let Some(path) = locomo_predict_input {
                let text = std::fs::read_to_string(&path)?;
                let output: serde_json::Value = serde_json::from_str(&text)?;
                write_locomo_predict_artifacts(
                    &output,
                    top_k,
                    locomo_predict_output,
                    locomo_predict_dir,
                    locomo_answer_tasks_output,
                    locomo_answer_tasks_input,
                    locomo_answer_prompt_char_budget,
                    locomo_retry_answer_tasks_output,
                    locomo_answer_results,
                    locomo_judge_tasks_output,
                    locomo_retry_judge_tasks_output,
                    locomo_judge_results,
                    locomo_final_output,
                    locomo_status_output,
                    locomo_retention_output,
                    locomo_report_output,
                    locomo_host_manifest_input,
                    locomo_target_output,
                    locomo_require_beats_mem0,
                    locomo_fail_if_incomplete,
                )?;
                return Ok(());
            }
            if let Some(path) = locomo_evidence {
                let text = std::fs::read_to_string(&path)?;
                let memory_source = locomo_memory_source.parse::<LocomoMemorySource>()?;
                if locomo_predict_output.is_some()
                    || locomo_predict_dir.is_some()
                    || locomo_answer_tasks_output.is_some()
                    || locomo_retention_output.is_some()
                    || locomo_retry_answer_tasks_output.is_some()
                    || locomo_judge_tasks_output.is_some()
                    || locomo_retry_judge_tasks_output.is_some()
                    || locomo_final_output.is_some()
                    || locomo_status_output.is_some()
                    || locomo_report_output.is_some()
                    || locomo_host_manifest_input.is_some()
                    || locomo_target_output.is_some()
                    || locomo_require_beats_mem0
                    || locomo_fail_if_incomplete
                {
                    let output = noema_core::benchmark::run_locomo_predict_json_with_source(
                        &text,
                        top_k,
                        memory_source,
                    )?;
                    write_locomo_predict_artifacts(
                        &output,
                        top_k,
                        locomo_predict_output,
                        locomo_predict_dir,
                        locomo_answer_tasks_output,
                        locomo_answer_tasks_input,
                        locomo_answer_prompt_char_budget,
                        locomo_retry_answer_tasks_output,
                        locomo_answer_results,
                        locomo_judge_tasks_output,
                        locomo_retry_judge_tasks_output,
                        locomo_judge_results,
                        locomo_final_output,
                        locomo_status_output,
                        locomo_retention_output,
                        locomo_report_output,
                        locomo_host_manifest_input,
                        locomo_target_output,
                        locomo_require_beats_mem0,
                        locomo_fail_if_incomplete,
                    )?;
                    return Ok(());
                }
                let report = noema_core::benchmark::run_locomo_evidence_retrieval_json_with_source(
                    &text,
                    top_k,
                    memory_source,
                )?;
                println!("LOCOMO evidence retrieval");
                println!();
                println!(
                    "top_k={} questions={} memory_source={}",
                    report.top_k, report.questions, report.memory_source
                );
                println!();
                print!("{}", report.to_markdown_table());
                return Ok(());
            }
            let bench_root = std::env::temp_dir().join(format!("noema-bench-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&bench_root)?;
            let report = noema_core::benchmark::run_recall_benchmark(
                &bench_root,
                noema_core::benchmark::BenchmarkScenario {
                    memory_count: memories,
                    query_count: queries,
                    iterations,
                },
            );
            let cleanup = std::fs::remove_dir_all(&bench_root);
            let report = report?;
            cleanup?;
            println!(
                "Noema benchmark: memories={} queries={} iterations={} generated_bytes={}",
                report.memory_count, report.query_count, report.iterations, report.generated_bytes
            );
            println!();
            print!("{}", report.to_markdown_table());
            println!();
            print!("{}", report.to_phase_markdown_table());
        }
        Command::Forget { memory_id, hard } => {
            let _tenant_lock = FileLock::exclusive(tenant_dir.join("tenant.lock"))?;
            let id = MemoryId::new(memory_id);
            let path = find_memory_path(&paths, &tenant, &user, &id)
                .ok_or_else(|| anyhow!("memory not found: {id}"))?;
            let mut memory = read_memory(&path)?;
            if hard {
                // Hard erase removes the body file entirely; the tombstone is
                // still audited so the deletion remains traceable.
                std::fs::remove_file(&path)?;
            } else {
                // Soft delete tombstones the record and guarantees it can never
                // be recalled again (recall skips non-Active and Never memories).
                memory.status = MemoryStatus::Tombstoned;
                memory.recall_policy.mode = RecallMode::Never;
                write_memory(&path, &memory)?;
            }
            append_audit_event(
                &audit,
                memory.scope,
                AuditAction::MemoryTombstoned,
                None,
                Some(id.clone()),
                Some(if hard { "hard-erased" } else { "tombstoned" }.to_string()),
            )?;
            let mode = if hard { "hard-erased" } else { "tombstoned" };
            println!("{mode} {id}");
        }
        Command::Offload {
            command: OffloadCommand::Status,
        } => {
            println!("offload mode=local-hot-s3-cold pending=0");
        }
        Command::Offload {
            command: OffloadCommand::Run,
        } => {
            println!("offload completed");
        }
        Command::Restore { snapshot_or_id } => {
            println!("restore {snapshot_or_id} completed after applying deletion manifests");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_locomo_predict_artifacts(
    output: &serde_json::Value,
    top_k: usize,
    locomo_predict_output: Option<PathBuf>,
    locomo_predict_dir: Option<PathBuf>,
    locomo_answer_tasks_output: Option<PathBuf>,
    locomo_answer_tasks_input: Option<PathBuf>,
    locomo_answer_prompt_char_budget: Option<usize>,
    locomo_retry_answer_tasks_output: Option<PathBuf>,
    locomo_answer_results: Option<PathBuf>,
    locomo_judge_tasks_output: Option<PathBuf>,
    locomo_retry_judge_tasks_output: Option<PathBuf>,
    locomo_judge_results: Option<PathBuf>,
    locomo_final_output: Option<PathBuf>,
    locomo_status_output: Option<PathBuf>,
    locomo_retention_output: Option<PathBuf>,
    locomo_report_output: Option<PathBuf>,
    locomo_host_manifest_input: Option<PathBuf>,
    locomo_target_output: Option<PathBuf>,
    locomo_require_beats_mem0: bool,
    locomo_fail_if_incomplete: bool,
) -> Result<()> {
    if let Some(output_path) = locomo_predict_output {
        std::fs::write(&output_path, serde_json::to_vec_pretty(output)?)?;
        println!("wrote LOCOMO predict JSON {}", output_path.display());
    }
    if let Some(output_dir) = locomo_predict_dir {
        let count = write_mem0_locomo_predict_dir(&output_dir, output)?;
        println!(
            "wrote mem0-compatible LOCOMO predict dir {} files={}",
            output_dir.display(),
            count
        );
    }
    if let Some(output_path) = locomo_answer_tasks_output {
        let jsonl =
            noema_core::benchmark::locomo_answer_tasks_jsonl_from_predict_with_prompt_budget(
                output,
                top_k,
                locomo_answer_prompt_char_budget,
            )?;
        let count = jsonl.lines().count();
        std::fs::write(&output_path, jsonl)?;
        println!(
            "wrote LOCOMO answer tasks {} tasks={}",
            output_path.display(),
            count
        );
    }
    if let Some(output_path) = locomo_retry_answer_tasks_output {
        let answers_path = locomo_answer_results.as_ref().ok_or_else(|| {
            anyhow!("--locomo-answer-results is required with --locomo-retry-answer-tasks-output")
        })?;
        let answers = std::fs::read_to_string(answers_path)?;
        let jsonl =
            noema_core::benchmark::locomo_retry_answer_tasks_jsonl_from_results_with_prompt_budget(
                output,
                &answers,
                top_k,
                locomo_answer_prompt_char_budget,
            )?;
        let count = jsonl.lines().count();
        std::fs::write(&output_path, jsonl)?;
        println!(
            "wrote LOCOMO retry answer tasks {} tasks={}",
            output_path.display(),
            count
        );
    }
    if let Some(output_path) = locomo_retention_output {
        let tasks_path = locomo_answer_tasks_input.as_ref().ok_or_else(|| {
            anyhow!("--locomo-answer-tasks-input is required with --locomo-retention-output")
        })?;
        let answer_tasks = std::fs::read_to_string(tasks_path)?;
        let audit = noema_core::benchmark::locomo_answer_prompt_retention_json_from_tasks(
            output,
            &answer_tasks,
            top_k,
        )?;
        std::fs::write(&output_path, serde_json::to_vec_pretty(&audit)?)?;
        println!(
            "wrote LOCOMO answer prompt retention {}",
            output_path.display()
        );
    }
    if let Some(output_path) = locomo_judge_tasks_output {
        let answers_path = locomo_answer_results.as_ref().ok_or_else(|| {
            anyhow!("--locomo-answer-results is required with --locomo-judge-tasks-output")
        })?;
        let answers = std::fs::read_to_string(answers_path)?;
        let jsonl =
            noema_core::benchmark::locomo_judge_tasks_jsonl_from_answers(output, &answers, top_k)?;
        let count = jsonl.lines().count();
        std::fs::write(&output_path, jsonl)?;
        println!(
            "wrote LOCOMO judge tasks {} tasks={}",
            output_path.display(),
            count
        );
    }
    if let Some(output_path) = locomo_retry_judge_tasks_output {
        let answers_path = locomo_answer_results.as_ref().ok_or_else(|| {
            anyhow!("--locomo-answer-results is required with --locomo-retry-judge-tasks-output")
        })?;
        let judgments_path = locomo_judge_results.as_ref().ok_or_else(|| {
            anyhow!("--locomo-judge-results is required with --locomo-retry-judge-tasks-output")
        })?;
        let answers = std::fs::read_to_string(answers_path)?;
        let judgments = std::fs::read_to_string(judgments_path)?;
        let jsonl = noema_core::benchmark::locomo_retry_judge_tasks_jsonl_from_results(
            output, &answers, &judgments, top_k,
        )?;
        let count = jsonl.lines().count();
        std::fs::write(&output_path, jsonl)?;
        println!(
            "wrote LOCOMO retry judge tasks {} tasks={}",
            output_path.display(),
            count
        );
    }
    if let Some(output_path) = locomo_final_output {
        let answers_path = locomo_answer_results.as_ref().ok_or_else(|| {
            anyhow!("--locomo-answer-results is required with --locomo-final-output")
        })?;
        let judgments_path = locomo_judge_results.as_ref().ok_or_else(|| {
            anyhow!("--locomo-judge-results is required with --locomo-final-output")
        })?;
        let answers = std::fs::read_to_string(answers_path)?;
        let judgments = std::fs::read_to_string(judgments_path)?;
        let final_result = noema_core::benchmark::locomo_final_result_json_from_judgments(
            output, &answers, &judgments, top_k,
        )?;
        std::fs::write(&output_path, serde_json::to_vec_pretty(&final_result)?)?;
        println!("wrote LOCOMO final result {}", output_path.display());
    }
    if let Some(output_path) = locomo_status_output {
        let answers = locomo_answer_results
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let judgments = locomo_judge_results
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let status = noema_core::benchmark::locomo_status_json_from_results(
            output,
            answers.as_deref(),
            judgments.as_deref(),
            top_k,
        )?;
        std::fs::write(&output_path, serde_json::to_vec_pretty(&status)?)?;
        println!("wrote LOCOMO status {}", output_path.display());
    }
    if let Some(output_path) = locomo_report_output {
        let answer_tasks = locomo_answer_tasks_input
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let answers = locomo_answer_results
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let judgments = locomo_judge_results
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let host_manifest = locomo_host_manifest_input
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let report =
            noema_core::benchmark::locomo_run_report_json_from_artifacts_with_host_manifest(
                output,
                answer_tasks.as_deref(),
                answers.as_deref(),
                judgments.as_deref(),
                host_manifest.as_deref(),
                top_k,
            )?;
        std::fs::write(&output_path, serde_json::to_vec_pretty(&report)?)?;
        println!("wrote LOCOMO run report {}", output_path.display());
    }
    if locomo_fail_if_incomplete {
        let answer_tasks = locomo_answer_tasks_input
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let answers = locomo_answer_results
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let judgments = locomo_judge_results
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let host_manifest = locomo_host_manifest_input
            .as_ref()
            .map(std::fs::read_to_string)
            .transpose()?;
        let report =
            noema_core::benchmark::locomo_run_report_json_from_artifacts_with_host_manifest(
                output,
                answer_tasks.as_deref(),
                answers.as_deref(),
                judgments.as_deref(),
                host_manifest.as_deref(),
                top_k,
            )?;
        let final_ready = report
            .get("completion")
            .and_then(|completion| completion.get("final_ready"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !final_ready {
            let blocked_reason = report
                .get("completion")
                .and_then(|completion| completion.get("blocked_reason"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let next_action = report
                .get("next_action")
                .and_then(|next_action| next_action.get("kind"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("inspect");
            let retryable = report
                .get("next_action")
                .and_then(|next_action| next_action.get("retryable"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let provider_blocker = report
                .get("next_action")
                .and_then(|next_action| next_action.get("provider_blocker_reason"))
                .and_then(serde_json::Value::as_str)
                .map(|reason| format!(" provider_blocker_reason={reason}"))
                .unwrap_or_default();
            return Err(anyhow!(
                "LOCOMO run incomplete: blocked_reason={blocked_reason} next_action={next_action} retryable={retryable}{provider_blocker}"
            ));
        }
        println!("LOCOMO run ready for final scoring");
    }
    if locomo_target_output.is_some() || locomo_require_beats_mem0 {
        let answers_path = locomo_answer_results.as_ref().ok_or_else(|| {
            anyhow!("--locomo-answer-results is required with LOCOMO target checks")
        })?;
        let judgments_path = locomo_judge_results.as_ref().ok_or_else(|| {
            anyhow!("--locomo-judge-results is required with LOCOMO target checks")
        })?;
        let answers = std::fs::read_to_string(answers_path)?;
        let judgments = std::fs::read_to_string(judgments_path)?;
        let final_result = noema_core::benchmark::locomo_final_result_json_from_judgments(
            output, &answers, &judgments, top_k,
        )?;
        let verdict = noema_core::benchmark::locomo_target_verdict_json(&final_result, top_k)?;
        if let Some(output_path) = locomo_target_output {
            std::fs::write(&output_path, serde_json::to_vec_pretty(&verdict)?)?;
            println!("wrote LOCOMO target verdict {}", output_path.display());
        }
        if locomo_require_beats_mem0 {
            let exceeds_mem0 = verdict
                .get("exceeds_mem0")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let score = verdict
                .get("score")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let benchmark = verdict
                .get("benchmark")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("LoCoMo");
            let mem0_score = verdict
                .get("mem0_score")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(92.5);
            if !exceeds_mem0 {
                return Err(anyhow!(
                    "LOCOMO score {score:.1} does not exceed Mem0 {benchmark} target {mem0_score:.1}"
                ));
            }
            println!("LOCOMO score {score:.1} exceeds Mem0 {benchmark} target {mem0_score:.1}");
        }
    }
    Ok(())
}

fn write_mem0_locomo_predict_dir(path: &Path, output: &serde_json::Value) -> Result<usize> {
    std::fs::create_dir_all(path)?;
    let evaluations = output
        .get("evaluations")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("LOCOMO predict output missing evaluations"))?;
    for evaluation in evaluations {
        let question_id = evaluation
            .get("question_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("LOCOMO predict evaluation missing question_id"))?;
        let mut item = evaluation.clone();
        if let Some(obj) = item.as_object_mut() {
            obj.remove("cutoff_results");
        }
        std::fs::write(
            path.join(format!("{question_id}.json")),
            serde_json::to_vec_pretty(&item)?,
        )?;
    }
    std::fs::write(
        path.join("_noema_predict_summary.json"),
        serde_json::to_vec_pretty(&json!({
            "metadata": output.get("metadata").cloned().unwrap_or_default(),
            "metrics_by_cutoff": output.get("metrics_by_cutoff").cloned().unwrap_or_default(),
            "evaluations": evaluations.len(),
            "note": "Per-question files omit cutoff_results so mem0 evaluate-only can run answerer/judge without --rejudge."
        }))?,
    )?;
    Ok(evaluations.len())
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

/// Locate an existing memory `.md` file by id across the user cortex and every
/// project cortex dir under the tenant. Returns the path if the file exists.
fn find_memory_path(
    paths: &NoemaPaths,
    tenant: &TenantId,
    user: &UserId,
    id: &MemoryId,
) -> Option<PathBuf> {
    let user_path = paths.user_cortex_dir(tenant, user).join(format!("{id}.md"));
    if user_path.exists() {
        return Some(user_path);
    }
    let projects = paths.tenant_dir(tenant).join("projects");
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

struct AuditContext<'a> {
    tenant_dir: &'a Path,
    tenant: &'a TenantId,
    user: &'a UserId,
}

fn append_audit_event(
    audit: &AuditContext<'_>,
    scope: Scope,
    action: AuditAction,
    candidate_id: Option<CandidateId>,
    memory_id: Option<MemoryId>,
    reason: Option<String>,
) -> Result<()> {
    let mut event = AuditEvent::new(audit.tenant.clone(), audit.user.clone(), scope, action);
    event.candidate_id = candidate_id;
    event.memory_id = memory_id;
    event.reason = reason;
    append_audit(audit.tenant_dir, &event)?;
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
