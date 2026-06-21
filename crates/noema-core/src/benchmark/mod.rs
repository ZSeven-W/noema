// Benchmark harness split into submodules:
//   mod.rs        — shared types, impls, re-exports, mem0 helpers
//   recall_bench  — run_recall_benchmark and timing helpers
//   locomo        — LOCOMO dataset/predict/evidence/answer-task pipeline
//   pipeline      — LOCOMO run report, judge tasks, final result, status
//   memory        — LOCOMO memory construction from dataset
//   score         — scoring, parsing, distribution helpers
//   prompt        — answer/judge prompt builders, memory compaction, candidates
//   token         — clue relevance scoring, token matching, stopwords

mod locomo;
mod memory;
mod pipeline;
mod prompt;
mod recall_bench;
mod score;
mod token;

// ---------------------------------------------------------------------------
// Public re-exports — preserve existing call-site paths unchanged
// ---------------------------------------------------------------------------

pub use locomo::{
    locomo_answer_prompt_retention_json_from_tasks, locomo_answer_tasks_jsonl_from_predict,
    locomo_answer_tasks_jsonl_from_predict_with_prompt_budget,
    locomo_retry_answer_tasks_jsonl_from_results,
    locomo_retry_answer_tasks_jsonl_from_results_with_prompt_budget,
    run_locomo_evidence_retrieval_json, run_locomo_evidence_retrieval_json_with_source,
    run_locomo_predict_json_with_source, summarize_locomo_dataset_json,
};

pub use pipeline::{
    locomo_final_result_json_from_judgments, locomo_judge_tasks_jsonl_from_answers,
    locomo_retry_judge_tasks_jsonl_from_results, locomo_run_report_json_from_artifacts,
    locomo_run_report_json_from_artifacts_with_host_manifest, locomo_status_json_from_results,
};

pub use recall_bench::run_recall_benchmark;

// summarize_mem0_result_json is defined in this file (below)

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde_json::{json, Value};

use crate::error::{NoemaError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BenchmarkScenario {
    pub memory_count: usize,
    pub query_count: usize,
    pub iterations: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkReport {
    pub memory_count: usize,
    pub query_count: usize,
    pub iterations: usize,
    pub generated_bytes: usize,
    pub samples: Vec<BenchmarkSample>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkSample {
    pub name: &'static str,
    pub operations: usize,
    pub total_ms: f64,
    pub mean_us: f64,
    pub p50_us: f64,
    pub p95_us: f64,
    pub phases: Vec<BenchmarkPhaseSample>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkPhaseSample {
    pub name: &'static str,
    pub operations: usize,
    pub total_ms: f64,
    pub mean_us: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkTarget {
    pub benchmark: &'static str,
    pub metric: &'static str,
    pub mem0_score: f64,
    pub noema_target_score: f64,
    pub notes: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mem0ResultSummary {
    pub benchmark: String,
    pub total_questions: usize,
    pub avg_search_latency_ms: Option<f64>,
    pub avg_retrieved_memories: Option<f64>,
    pub cutoffs: Vec<Mem0CutoffSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mem0CutoffSummary {
    pub cutoff: String,
    pub score_label: String,
    pub score: f64,
    pub total: usize,
    pub groups: Vec<Mem0GroupSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mem0GroupSummary {
    pub name: String,
    pub score_label: String,
    pub score: f64,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocomoDatasetSummary {
    pub conversations: usize,
    pub sessions: usize,
    pub turns: usize,
    pub questions: usize,
    pub evaluable_questions: usize,
    pub evidence_refs: usize,
    pub resolved_evidence_refs: usize,
    pub category_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocomoEvidenceReport {
    pub memory_source: LocomoMemorySource,
    pub questions: usize,
    pub top_k: usize,
    pub any_evidence_hits: usize,
    pub all_evidence_hits: usize,
    pub any_evidence_hit_rate: f64,
    pub all_evidence_hit_rate: f64,
}

// Internal types used across submodules

#[derive(Debug, Clone, Default)]
pub(super) struct LocomoMetricAccumulator {
    pub category_id: i64,
    pub total: usize,
    pub correct: usize,
    pub score_sum: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LocomoJudgeResult {
    pub label: String,
    pub reasoning: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LocomoAnswerResultState {
    Valid,
    Empty,
    HostFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LocomoJudgeResultState {
    Valid(LocomoJudgeResult),
    InvalidLabel,
    HostFailed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocomoMemorySource {
    Raw,
    Observation,
    RawPlusObservation,
    FactLayer,
    RawPlusFactLayer,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PhaseMeasurement {
    pub name: &'static str,
    pub us: f64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PhaseAccumulator {
    pub name: &'static str,
    pub operations: usize,
    pub total_us: f64,
}

// Used by locomo raw memory building
pub(super) const LOCOMO_EPISODE_PROMPT_LINES: usize = 6;

// ---------------------------------------------------------------------------
// Type impls
// ---------------------------------------------------------------------------

impl BenchmarkScenario {
    pub(super) fn validate(self) -> Result<Self> {
        if self.memory_count == 0 {
            return Err(NoemaError::InvalidRecord(
                "benchmark memory_count must be greater than zero".into(),
            ));
        }
        if self.query_count == 0 {
            return Err(NoemaError::InvalidRecord(
                "benchmark query_count must be greater than zero".into(),
            ));
        }
        if self.iterations == 0 {
            return Err(NoemaError::InvalidRecord(
                "benchmark iterations must be greater than zero".into(),
            ));
        }
        Ok(self)
    }
}

impl BenchmarkReport {
    pub fn to_markdown_table(&self) -> String {
        let mut out = String::from(
            "| Scenario | Operations | Total ms | Mean us/op | p50 us | p95 us |\n\
             | --- | ---: | ---: | ---: | ---: | ---: |\n",
        );
        for sample in &self.samples {
            out.push_str(&format!(
                "| {} | {} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
                sample.name,
                sample.operations,
                sample.total_ms,
                sample.mean_us,
                sample.p50_us,
                sample.p95_us
            ));
        }
        out
    }

    pub fn to_phase_markdown_table(&self) -> String {
        let mut out = String::from(
            "| Scenario | Phase | Operations | Total ms | Mean us/op |\n\
             | --- | --- | ---: | ---: | ---: |\n",
        );
        for sample in &self.samples {
            for phase in &sample.phases {
                out.push_str(&format!(
                    "| {} | {} | {} | {:.3} | {:.3} |\n",
                    sample.name, phase.name, phase.operations, phase.total_ms, phase.mean_us
                ));
            }
        }
        out
    }
}

impl Mem0ResultSummary {
    pub fn to_markdown_table(&self) -> String {
        let mut out = String::from("| Cutoff | Metric | Score | Total |\n");
        out.push_str("| --- | --- | ---: | ---: |\n");
        for cutoff in &self.cutoffs {
            out.push_str(&format!(
                "| {} | {} | {:.1} | {} |\n",
                cutoff.cutoff, cutoff.score_label, cutoff.score, cutoff.total
            ));
        }
        out
    }
}

impl LocomoDatasetSummary {
    pub fn to_markdown_table(&self) -> String {
        let mut out = String::from("| Category | Questions |\n");
        out.push_str("| --- | ---: |\n");
        for (category, count) in &self.category_counts {
            out.push_str(&format!("| {category} | {count} |\n"));
        }
        out
    }
}

impl LocomoEvidenceReport {
    pub fn to_markdown_table(&self) -> String {
        format!(
            "| Metric | Count | Rate |\n\
             | --- | ---: | ---: |\n\
             | any_evidence_hit | {}/{} | {:.1} |\n\
             | all_evidence_hit | {}/{} | {:.1} |\n",
            self.any_evidence_hits,
            self.questions,
            self.any_evidence_hit_rate,
            self.all_evidence_hits,
            self.questions,
            self.all_evidence_hit_rate
        )
    }
}

impl LocomoMemorySource {
    pub(super) fn includes_raw(self) -> bool {
        matches!(
            self,
            Self::Raw | Self::RawPlusObservation | Self::RawPlusFactLayer
        )
    }

    pub(super) fn includes_observation(self) -> bool {
        matches!(
            self,
            Self::Observation | Self::RawPlusObservation | Self::FactLayer | Self::RawPlusFactLayer
        )
    }

    pub(super) fn includes_fact_summary(self) -> bool {
        matches!(self, Self::FactLayer | Self::RawPlusFactLayer)
    }
}

impl fmt::Display for LocomoMemorySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raw => f.write_str("raw"),
            Self::Observation => f.write_str("observation"),
            Self::RawPlusObservation => f.write_str("raw-plus-observation"),
            Self::FactLayer => f.write_str("fact-layer"),
            Self::RawPlusFactLayer => f.write_str("raw-plus-fact-layer"),
        }
    }
}

impl FromStr for LocomoMemorySource {
    type Err = NoemaError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "raw" => Ok(Self::Raw),
            "observation" => Ok(Self::Observation),
            "raw-plus-observation" => Ok(Self::RawPlusObservation),
            "fact-layer" => Ok(Self::FactLayer),
            "raw-plus-fact-layer" => Ok(Self::RawPlusFactLayer),
            other => Err(NoemaError::InvalidRecord(format!(
                "unsupported LOCOMO memory source {other:?}; expected raw, observation, raw-plus-observation, fact-layer, or raw-plus-fact-layer"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Mem0 reference targets
// ---------------------------------------------------------------------------

pub fn mem0_reference_targets() -> Vec<BenchmarkTarget> {
    vec![
        BenchmarkTarget {
            benchmark: "LoCoMo",
            metric: "overall score",
            mem0_score: 92.5,
            noema_target_score: 92.6,
            notes: "multi-session dialogue memory",
        },
        BenchmarkTarget {
            benchmark: "LongMemEval",
            metric: "overall score",
            mem0_score: 94.4,
            noema_target_score: 94.5,
            notes: "long-term memory questions",
        },
        BenchmarkTarget {
            benchmark: "BEAM 1M",
            metric: "average score",
            mem0_score: 64.1,
            noema_target_score: 64.2,
            notes: "1M-token memory ability benchmark",
        },
        BenchmarkTarget {
            benchmark: "BEAM 10M",
            metric: "average score",
            mem0_score: 48.6,
            noema_target_score: 48.7,
            notes: "10M-token memory ability benchmark",
        },
    ]
}

pub fn mem0_reference_targets_markdown_table() -> String {
    let mut out = String::from(
        "| Benchmark | Metric | Mem0 score | Noema must exceed | Notes |\n\
         | --- | --- | ---: | ---: | --- |\n",
    );
    for target in mem0_reference_targets() {
        out.push_str(&format!(
            "| {} | {} | {:.1} | > {:.1} | {} |\n",
            target.benchmark, target.metric, target.mem0_score, target.mem0_score, target.notes
        ));
    }
    out
}

pub fn locomo_target_verdict_json(final_result: &Value, top_k: usize) -> Result<Value> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO target verdict top_k must be greater than zero".into(),
        ));
    }
    let target = mem0_reference_targets()
        .into_iter()
        .find(|target| target.benchmark == "LoCoMo")
        .ok_or_else(|| NoemaError::InvalidRecord("LoCoMo target is not configured".into()))?;
    let cutoff = format!("top_{top_k}");
    let overall = final_result
        .get("metrics_by_cutoff")
        .and_then(|metrics| metrics.get(&cutoff))
        .and_then(|cutoff_metrics| cutoff_metrics.get("overall"))
        .ok_or_else(|| {
            NoemaError::InvalidRecord(format!(
                "LOCOMO final result missing metrics_by_cutoff.{cutoff}.overall"
            ))
        })?;
    let score = overall
        .get("accuracy")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            NoemaError::InvalidRecord(format!(
                "LOCOMO final result missing metrics_by_cutoff.{cutoff}.overall.accuracy"
            ))
        })?;
    let total = overall.get("total").and_then(Value::as_u64).unwrap_or(0);
    let correct = overall.get("correct").and_then(Value::as_u64).unwrap_or(0);

    Ok(json!({
        "benchmark": target.benchmark,
        "cutoff": cutoff,
        "metric": target.metric,
        "score": score,
        "mem0_score": target.mem0_score,
        "noema_target_score": target.noema_target_score,
        "exceeds_mem0": score > target.mem0_score,
        "meets_noema_target": score >= target.noema_target_score,
        "margin_vs_mem0": score - target.mem0_score,
        "total": total,
        "correct": correct,
    }))
}

// ---------------------------------------------------------------------------
// Mem0 result summary
// ---------------------------------------------------------------------------

pub fn summarize_mem0_result_json(text: &str) -> Result<Mem0ResultSummary> {
    let root: Value = serde_json::from_str(text)?;
    let root_obj = root
        .as_object()
        .ok_or_else(|| NoemaError::InvalidRecord("mem0 result must be a JSON object".into()))?;
    let metadata = root_obj
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or_else(|| NoemaError::InvalidRecord("mem0 result missing metadata".into()))?;
    let evaluations = root_obj
        .get("evaluations")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let metrics_by_cutoff = root_obj
        .get("metrics_by_cutoff")
        .and_then(Value::as_object)
        .ok_or_else(|| NoemaError::InvalidRecord("mem0 result missing metrics_by_cutoff".into()))?;

    let benchmark = metadata
        .get("benchmark")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("project_name").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();
    let total_questions = metadata
        .get("total_questions")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(evaluations.len());
    let mut cutoffs = Vec::new();
    for (name, value) in metrics_by_cutoff {
        let Some(cutoff_obj) = value.as_object() else {
            continue;
        };
        let Some(overall) = cutoff_obj.get("overall").and_then(Value::as_object) else {
            continue;
        };
        let (score_label, score_val) = score::metric_score(overall);
        let total = overall
            .get("total")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(total_questions);
        cutoffs.push(Mem0CutoffSummary {
            cutoff: name.clone(),
            score_label,
            score: score_val,
            total,
            groups: score::cutoff_groups(cutoff_obj),
        });
    }
    cutoffs.sort_by(|left, right| left.cutoff.cmp(&right.cutoff));

    Ok(Mem0ResultSummary {
        benchmark,
        total_questions,
        avg_search_latency_ms: score::average_search_latency(evaluations),
        avg_retrieved_memories: score::average_retrieved_memories(evaluations),
        cutoffs,
    })
}
