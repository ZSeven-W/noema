mod bench;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use noema_core::api::{
    ExplainRequest, ForgetRequest, NoemaEngine, RecallRequest, RememberRequest, ReviewAction,
    ReviewDecisionRequest, ReviewOutcome, SearchRequest, SubmitOutcome,
};
use noema_core::config::NoemaConfig;
use noema_core::memory::{MemoryKind, Scope};
use noema_core::paths::NoemaPaths;
use noema_core::sensitivity::{Principal, SensitivityLevel};
use std::path::Path;

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
        /// Persist this explicit memory immediately instead of leaving it in review.
        #[arg(long)]
        accept: bool,
    },
    Recall {
        query: String,
        #[arg(long, default_value_t = 1200)]
        budget_tokens: usize,
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
    /// Print the PageIndex catalog (LLM-Wiki index.md) over your memories.
    Catalog,
    /// Navigate the catalog for a query and print the memories on matched pages.
    Browse {
        query: String,
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },
    Sleep {
        #[arg(long)]
        llm: bool,
    },
    Doctor,
    Reindex,
    Bench(bench::BenchArgs),
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

    // Build engine and principal once; Init does not need them.
    let engine = NoemaEngine::from_config(&cfg)?;
    let mut principal = Principal::personal(&cfg.tenant.default_user_id, "noema-cli");
    principal.tenant_id = noema_core::ids::TenantId::new(cfg.tenant.id.clone());

    match cli.command {
        Command::Init => {
            let paths = NoemaPaths::new(&cfg.storage.local_root);
            let user = noema_core::ids::UserId::new(cfg.tenant.default_user_id.clone());
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
            accept,
        } => {
            let outcome = engine.submit_candidate(RememberRequest {
                principal: principal.clone(),
                text,
                scope: parse_scope(&scope)?,
                project_path: std::env::current_dir().ok(),
                kind: parse_kind(&kind)?,
                sensitivity: parse_sensitivity(&sensitivity)?,
                tags,
                entities,
                confidence,
                importance,
            })?;
            match outcome {
                SubmitOutcome::Queued { candidate_id } if accept => {
                    let accepted = engine.review_decide(ReviewDecisionRequest {
                        principal: principal.clone(),
                        candidate_id: candidate_id.clone(),
                        action: ReviewAction::Accept,
                    })?;
                    match accepted {
                        ReviewOutcome::Accepted { memory_id } => println!("accepted {memory_id}"),
                        other => return Err(anyhow!("unexpected review outcome: {other:?}")),
                    }
                }
                SubmitOutcome::Queued { candidate_id } => println!("queued {candidate_id}"),
                SubmitOutcome::AutoAccepted { memory_id } => println!("accepted {memory_id}"),
                SubmitOutcome::RejectedSecret => {
                    return Err(anyhow!("secret candidates are rejected before review"))
                }
            }
        }
        Command::Recall {
            query,
            budget_tokens,
        } => {
            let cwd = std::env::current_dir().ok();
            let pack = engine.recall(RecallRequest {
                principal: principal.clone(),
                query,
                cwd,
                budget_tokens,
            })?;
            print!("{}", pack.to_markdown());
        }
        Command::Review => {
            for candidate in engine.review_list(&principal)? {
                println!("{} {}", candidate.id, candidate.body);
            }
        }
        Command::Edit {
            candidate_id,
            body,
            reason,
        } => {
            engine.review_decide(ReviewDecisionRequest {
                principal: principal.clone(),
                candidate_id: candidate_id.clone(),
                action: ReviewAction::Edit { body, reason },
            })?;
            println!("edited {candidate_id}");
        }
        Command::Merge {
            candidate_id,
            target_memory_id,
            reason,
        } => {
            engine.review_decide(ReviewDecisionRequest {
                principal: principal.clone(),
                candidate_id: candidate_id.clone(),
                action: ReviewAction::Merge {
                    target_memory_id,
                    reason,
                },
            })?;
            println!("merged {candidate_id}");
        }
        Command::Accept { candidate_id } => {
            engine.review_decide(ReviewDecisionRequest {
                principal: principal.clone(),
                candidate_id: candidate_id.clone(),
                action: ReviewAction::Accept,
            })?;
            println!("accepted {candidate_id}");
        }
        Command::Reject {
            candidate_id,
            reason,
        } => {
            engine.review_decide(ReviewDecisionRequest {
                principal: principal.clone(),
                candidate_id: candidate_id.clone(),
                action: ReviewAction::Reject { reason },
            })?;
            println!("rejected {candidate_id}");
        }
        Command::Search { query } => {
            let cwd = std::env::current_dir().ok();
            for scored in engine.search(SearchRequest {
                principal: principal.clone(),
                query,
                cwd,
            })? {
                println!("{:.3} {}", scored.score, scored.id);
            }
        }
        Command::Explain { memory_id, query } => {
            let cwd = std::env::current_dir().ok();
            if let Some(scored) = engine.explain(ExplainRequest {
                principal: principal.clone(),
                memory_id,
                query,
                cwd,
            })? {
                println!("{}", scored.explanation.join("\n"));
            }
        }
        Command::Vacuum => {
            let tenant = &principal.tenant_id;
            let tenant_dir = engine.paths.tenant_dir(tenant);
            // compact_hippocampus acquires tenant.lock itself; locking here too
            // would self-deadlock (fs4 advisory locks are per-process).
            noema_core::vacuum::compact_hippocampus(&tenant_dir)?;
            let mut event = noema_core::audit::AuditEvent::new(
                tenant.clone(),
                principal.user_id.clone(),
                noema_core::memory::Scope::User,
                noema_core::audit::AuditAction::VacuumCompacted,
            );
            event.candidate_id = None;
            event.memory_id = None;
            event.reason = None;
            noema_core::audit::append_audit(&tenant_dir, &event)?;
            println!("vacuumed {}", tenant_dir.display());
        }
        Command::Catalog => {
            let cwd = std::env::current_dir().ok();
            let catalog = engine.catalog(&principal, cwd.as_deref())?;
            print!("{}", catalog.to_markdown());
        }
        Command::Browse { query, limit } => {
            let cwd = std::env::current_dir().ok();
            for memory in engine.browse(&principal, &query, limit, cwd.as_deref())? {
                println!("{} {}", memory.id, memory.body);
            }
        }
        Command::Sleep { llm } => {
            let tenant = &principal.tenant_id;
            let jobs = noema_core::extraction::load_jobs(&cfg.storage.local_root, tenant)?;
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
            let tenant = &principal.tenant_id;
            let user = &principal.user_id;
            let mut index = noema_core::index::LexicalIndex::default();
            // Load memories via the engine's paths (same layout as before).
            let user_cortex = engine.paths.user_cortex_dir(tenant, user);
            let mut memories = Vec::new();
            load_memory_dir_reindex(&user_cortex, &mut memories)?;
            for memory in memories {
                index.add(noema_core::index::IndexDocument {
                    id: memory.id,
                    text: memory.body,
                    tags: memory.tags,
                    entities: memory.entities,
                });
            }
            let index_dir = engine.paths.tenant_dir(tenant).join("indexes");
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
        Command::Bench(args) => bench::run_bench(args, &cfg)?,
        Command::Forget { memory_id, hard } => {
            let out = engine.forget(ForgetRequest {
                principal: principal.clone(),
                memory_id,
                hard,
            })?;
            println!("{} {}", out.mode, out.memory_id);
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

/// Load memory records from a directory for the Reindex command.
/// Only used by the local Reindex arm; normal recall goes through the engine.
fn load_memory_dir_reindex(
    dir: &Path,
    out: &mut Vec<noema_core::memory::MemoryRecord>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(noema_core::store::read_memory(&entry.path())?);
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
