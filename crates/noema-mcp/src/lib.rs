//! rmcp tool surface over NoemaEngine, shared by the stdio and HTTP servers.
use std::sync::Arc;

use noema_core::api::{
    ExplainRequest, ForgetRequest, NoemaEngine, PolicySetRequest, RememberRequest, ReviewAction,
    ReviewDecisionRequest, ReviewOutcome, SearchRequest, SubmitOutcome,
};
use noema_core::config::NoemaConfig;
use noema_core::ids::TenantId;
use noema_core::memory::{MemoryKind, Scope};
use noema_core::sensitivity::{Principal, SensitivityLevel};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::schemars::JsonSchema;
use rmcp::service::RequestContext;
use rmcp::{tool, tool_router, ErrorData, RoleServer};
use serde::{Deserialize, Serialize};

/// Serialize `t` to a pretty-printed JSON string, mapping errors to ErrorData.
fn to_json_str<T: Serialize>(t: T) -> Result<String, ErrorData> {
    serde_json::to_string_pretty(&t)
        .map_err(|e| ErrorData::internal_error(format!("serialization error: {e}"), None))
}

fn internal(e: tokio::task::JoinError) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

fn domain(e: noema_core::error::NoemaError) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

// --------------------------------------------------------------------------
// Argument structs
// --------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RecallArgs {
    pub query: String,
    #[serde(default)]
    pub budget_tokens: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchArgs {
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RecallGraphArgs {
    pub query: String,
    /// How many hops to walk outward from the lexical seeds (default 3).
    #[serde(default)]
    pub max_hops: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct NeighborsArgs {
    pub memory_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ExplainArgs {
    pub memory_id: String,
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RememberArgs {
    pub text: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    /// Persist explicit memory immediately. Defaults to true for host agents.
    #[serde(default)]
    pub accept: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DecideArgs {
    pub candidate_id: String,
    /// One of: accept, reject, edit, merge.
    pub decision: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub target_memory_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ForgetArgs {
    pub memory_id: String,
    #[serde(default)]
    pub hard: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct PolicySetArgs {
    /// New write policy: "manual", "review", "auto-safe", or "auto".
    #[serde(default)]
    pub write: Option<String>,
}

// --------------------------------------------------------------------------
// NoemaTools
// --------------------------------------------------------------------------

/// rmcp ServerHandler wrapping a NoemaEngine.
#[derive(Clone)]
pub struct NoemaTools {
    engine: Arc<NoemaEngine>,
    default_principal: Principal,
}

/// Return type for tool handlers: serialized JSON text content.
/// Using String avoids the MCP outputSchema "must be object" restriction that
/// applies only to `Json<T>` wrappers.
type ToolResult = Result<String, ErrorData>;

#[tool_router(server_handler)]
impl NoemaTools {
    pub fn new(engine: Arc<NoemaEngine>, default_principal: Principal) -> Self {
        Self {
            engine,
            default_principal,
        }
    }

    /// Return the principal for this request.
    ///
    /// For the streamable-HTTP transport, rmcp injects `http::request::Parts`
    /// into the message extensions before routing to the handler, and our auth
    /// tower layer inserts a `Principal` into those parts' HTTP extensions.
    /// We walk the chain here:
    ///   `ctx.extensions` (rmcp) → `http::request::Parts` → `parts.extensions` (http) → `Principal`
    ///
    /// For the stdio transport no `Parts` are present, so we fall back to the
    /// configured default principal unchanged.
    fn principal_for(&self, ctx: &RequestContext<RoleServer>) -> Principal {
        ctx.extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<Principal>())
            .cloned()
            .unwrap_or_else(|| self.default_principal.clone())
    }

    // -----------------------------------------------------------------
    // Tools
    // -----------------------------------------------------------------

    #[tool(description = "Recall relevant memories for a query")]
    async fn noema_recall(
        &self,
        Parameters(args): Parameters<RecallArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let pack = tokio::task::spawn_blocking(move || {
            engine.recall(noema_core::api::RecallRequest {
                principal: principal.clone(),
                query: args.query,
                cwd: None,
                budget_tokens: args.budget_tokens.unwrap_or(1200),
            })
        })
        .await
        .map_err(internal)?
        .map_err(domain)?;
        to_json_str(pack)
    }

    #[tool(description = "Full-text search over memories")]
    async fn noema_search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let results = tokio::task::spawn_blocking(move || {
            engine.search(SearchRequest {
                principal,
                query: args.query,
                cwd: None,
            })
        })
        .await
        .map_err(internal)?
        .map_err(domain)?;
        // ScoredMemory does not derive Serialize; flatten to a plain JSON value.
        let value: Vec<serde_json::Value> = results
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "score": s.score,
                    "explanation": s.explanation,
                })
            })
            .collect();
        to_json_str(value)
    }

    #[tool(
        description = "Multi-hop recall: lexical seed, then walk links + shared entities outward up to max_hops. Use for questions whose answer spans several connected memories."
    )]
    async fn noema_recall_graph(
        &self,
        Parameters(args): Parameters<RecallGraphArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let max_hops = args.max_hops.unwrap_or(3);
        let results = tokio::task::spawn_blocking(move || {
            engine.recall_graph(
                SearchRequest {
                    principal,
                    query: args.query,
                    cwd: None,
                },
                max_hops,
            )
        })
        .await
        .map_err(internal)?
        .map_err(domain)?;
        let value: Vec<serde_json::Value> = results
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "score": s.score,
                    "explanation": s.explanation,
                })
            })
            .collect();
        to_json_str(value)
    }

    #[tool(
        description = "One graph hop from a memory: the memories it links to or shares an entity with. Step through these for guided multi-hop retrieval."
    )]
    async fn noema_neighbors(
        &self,
        Parameters(args): Parameters<NeighborsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let neighbors = tokio::task::spawn_blocking(move || {
            engine.neighbors(&principal, &args.memory_id, None)
        })
        .await
        .map_err(internal)?
        .map_err(domain)?;
        let value: Vec<serde_json::Value> = neighbors
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "kind": format!("{:?}", m.kind).to_lowercase(),
                    "entities": m.entities,
                    "text": m.body,
                })
            })
            .collect();
        to_json_str(value)
    }

    #[tool(description = "Explain why a specific memory was or was not recalled")]
    async fn noema_explain(
        &self,
        Parameters(args): Parameters<ExplainArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let result = tokio::task::spawn_blocking(move || {
            engine.explain(ExplainRequest {
                principal,
                memory_id: args.memory_id,
                query: args.query,
                cwd: None,
            })
        })
        .await
        .map_err(internal)?
        .map_err(domain)?;
        // ScoredMemory does not derive Serialize; flatten to a plain JSON value.
        let value = result.map(|s| {
            serde_json::json!({
                "id": s.id,
                "score": s.score,
                "explanation": s.explanation,
            })
        });
        to_json_str(value)
    }

    #[tool(description = "Submit a memory candidate for review")]
    async fn noema_remember(
        &self,
        Parameters(args): Parameters<RememberArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let outcome = engine.submit_candidate(RememberRequest {
                principal: principal.clone(),
                text: args.text,
                scope: Scope::User,
                project_path: None,
                kind: MemoryKind::Preference,
                sensitivity: SensitivityLevel::Internal,
                tags: args.tags,
                entities: args.entities,
                confidence: 1.0,
                importance: 0.5,
            })?;
            if !args.accept.unwrap_or(true) {
                return Ok(serde_json::to_value(outcome)?);
            }
            match outcome {
                SubmitOutcome::Queued { candidate_id } => {
                    let accepted = engine.review_decide(ReviewDecisionRequest {
                        principal,
                        candidate_id,
                        action: ReviewAction::Accept,
                    })?;
                    Ok(serde_json::to_value(accepted)?)
                }
                SubmitOutcome::AutoAccepted { memory_id } => {
                    Ok(serde_json::to_value(ReviewOutcome::Accepted { memory_id })?)
                }
                SubmitOutcome::RejectedSecret => Ok(serde_json::to_value(outcome)?),
            }
        })
        .await
        .map_err(internal)?
        .map_err(domain)?;
        to_json_str(outcome)
    }

    #[tool(description = "List pending review candidates")]
    async fn noema_review_list(&self, ctx: RequestContext<RoleServer>) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let pending = tokio::task::spawn_blocking(move || engine.review_list(&principal))
            .await
            .map_err(internal)?
            .map_err(domain)?;
        let items: Vec<String> = pending
            .into_iter()
            .map(|c| format!("{} {}", c.id, c.body))
            .collect();
        to_json_str(items)
    }

    #[tool(description = "Decide a pending candidate: accept, reject, edit, or merge")]
    async fn noema_review_decide(
        &self,
        Parameters(args): Parameters<DecideArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let action = match args.decision.as_str() {
            "accept" => ReviewAction::Accept,
            "reject" => ReviewAction::Reject {
                reason: args.reason,
            },
            "edit" => ReviewAction::Edit {
                body: args.body,
                reason: args.reason,
            },
            "merge" => ReviewAction::Merge {
                target_memory_id: args.target_memory_id,
                reason: args.reason,
            },
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown decision: {other}"),
                    None,
                ))
            }
        };
        let outcome = tokio::task::spawn_blocking(move || {
            engine.review_decide(ReviewDecisionRequest {
                principal,
                candidate_id: args.candidate_id,
                action,
            })
        })
        .await
        .map_err(internal)?
        .map_err(domain)?;
        to_json_str(outcome)
    }

    #[tool(description = "Permanently remove or tombstone a memory")]
    async fn noema_forget(
        &self,
        Parameters(args): Parameters<ForgetArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            engine.forget(ForgetRequest {
                principal,
                memory_id: args.memory_id,
                hard: args.hard,
            })
        })
        .await
        .map_err(internal)?
        .map_err(domain)?;
        to_json_str(outcome)
    }

    #[tool(description = "Get the current write policy and sensitivity settings")]
    async fn noema_policy_get(&self, ctx: RequestContext<RoleServer>) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let view = tokio::task::spawn_blocking(move || engine.policy_get(&principal))
            .await
            .map_err(internal)?
            .map_err(domain)?;
        to_json_str(view)
    }

    #[tool(description = "Update the write policy")]
    async fn noema_policy_set(
        &self,
        Parameters(args): Parameters<PolicySetArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let write = args.write.as_deref().map(|s| {
            serde_json::from_value::<noema_core::config::WritePolicy>(serde_json::Value::String(
                s.to_string(),
            ))
            .map_err(|e| ErrorData::invalid_params(format!("invalid write policy: {e}"), None))
        });
        let write = match write {
            Some(Ok(p)) => Some(p),
            Some(Err(e)) => return Err(e),
            None => None,
        };
        let view = tokio::task::spawn_blocking(move || {
            engine.policy_set(PolicySetRequest { principal, write })
        })
        .await
        .map_err(internal)?
        .map_err(domain)?;
        to_json_str(view)
    }

    #[tool(description = "Server and tenant status")]
    async fn noema_status(&self, ctx: RequestContext<RoleServer>) -> ToolResult {
        let principal = self.principal_for(&ctx);
        let engine = self.engine.clone();
        let view = tokio::task::spawn_blocking(move || engine.status(&principal))
            .await
            .map_err(internal)?
            .map_err(domain)?;
        to_json_str(view)
    }
}

// --------------------------------------------------------------------------
// Convenience constructor for the personal (single-user) principal.
// --------------------------------------------------------------------------

/// Build the default personal principal from a NoemaConfig.
pub fn personal_principal(cfg: &NoemaConfig) -> Principal {
    let mut p = Principal::personal(&cfg.tenant.default_user_id, "noema-mcp");
    p.tenant_id = TenantId::new(cfg.tenant.id.clone());
    p
}
