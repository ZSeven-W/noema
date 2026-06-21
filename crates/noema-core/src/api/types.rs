use std::path::PathBuf;

use crate::config::WritePolicy;
use crate::memory::{MemoryKind, Scope};
use crate::memorypack::MemoryPack;
use crate::sensitivity::{Principal, SensitivityLevel};

#[derive(Debug, Clone)]
pub struct RecallRequest {
    pub principal: Principal,
    pub query: String,
    pub cwd: Option<PathBuf>,
    pub budget_tokens: usize,
    pub host: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfiledRecall {
    pub pack: MemoryPack,
    pub timings: RecallTimings,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecallTimings {
    pub loaded_memories: usize,
    pub scored_memories: usize,
    pub load_memories_us: f64,
    pub score_memories_us: f64,
    pub build_pack_us: f64,
    pub total_us: f64,
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub principal: Principal,
    pub query: String,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ExplainRequest {
    pub principal: Principal,
    pub memory_id: String,
    pub query: String,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RememberRequest {
    pub principal: Principal,
    pub text: String,
    pub scope: Scope,
    pub project_path: Option<PathBuf>,
    pub kind: MemoryKind,
    pub sensitivity: SensitivityLevel,
    pub tags: Vec<String>,
    pub entities: Vec<String>,
    pub confidence: f32,
    pub importance: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "route")]
pub enum SubmitOutcome {
    Queued { candidate_id: String },
    AutoAccepted { memory_id: String },
    RejectedSecret,
}

#[derive(Debug, Clone)]
pub enum ReviewAction {
    Accept,
    Reject {
        reason: String,
    },
    Edit {
        body: String,
        reason: String,
    },
    Merge {
        target_memory_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct ReviewDecisionRequest {
    pub principal: Principal,
    pub candidate_id: String,
    pub action: ReviewAction,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ReviewOutcome {
    Accepted { memory_id: String },
    Rejected,
    Edited,
    Merged,
}

#[derive(Debug, Clone)]
pub struct ForgetRequest {
    pub principal: Principal,
    pub memory_id: String,
    pub hard: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ForgetOutcome {
    pub memory_id: String,
    pub mode: String, // "tombstoned" | "hard-erased"
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PolicyView {
    pub write: WritePolicy,
    pub auto_accept_max_sensitivity: SensitivityLevel,
    pub external_llm_max_sensitivity: SensitivityLevel,
}

#[derive(Debug, Clone)]
pub struct PolicySetRequest {
    pub principal: Principal,
    pub write: Option<WritePolicy>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StatusView {
    pub tenant: String,
    pub user: String,
    pub mode: String,
    pub write_policy: WritePolicy,
    pub ok: bool,
}
