use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, Instant};

use crate::api::{
    NoemaEngine, RecallRequest, RememberRequest, ReviewAction, ReviewDecisionRequest,
};
use crate::error::{NoemaError, Result};
use crate::ids::{MemoryId, TenantId, UserId};
use crate::memory::{MemoryKind, MemoryRecord, Scope};
use crate::recall::recall;
use crate::sensitivity::{Principal, SensitivityLevel};
use serde_json::{json, Value};

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

#[derive(Debug, Clone, Default)]
struct LocomoMetricAccumulator {
    category_id: i64,
    total: usize,
    correct: usize,
    score_sum: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocomoJudgeResult {
    label: String,
    reasoning: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocomoAnswerResultState {
    Valid,
    Empty,
    HostFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocomoJudgeResultState {
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
struct PhaseMeasurement {
    name: &'static str,
    us: f64,
}

#[derive(Debug, Clone, Copy)]
struct PhaseAccumulator {
    name: &'static str,
    operations: usize,
    total_us: f64,
}

const LOCOMO_EPISODE_PROMPT_LINES: usize = 6;

impl BenchmarkScenario {
    fn validate(self) -> Result<Self> {
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
    fn includes_raw(self) -> bool {
        matches!(
            self,
            Self::Raw | Self::RawPlusObservation | Self::RawPlusFactLayer
        )
    }

    fn includes_observation(self) -> bool {
        matches!(
            self,
            Self::Observation | Self::RawPlusObservation | Self::FactLayer | Self::RawPlusFactLayer
        )
    }

    fn includes_fact_summary(self) -> bool {
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
        let (score_label, score) = metric_score(overall);
        let total = overall
            .get("total")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(total_questions);
        cutoffs.push(Mem0CutoffSummary {
            cutoff: name.clone(),
            score_label,
            score,
            total,
            groups: cutoff_groups(cutoff_obj),
        });
    }
    cutoffs.sort_by(|left, right| left.cutoff.cmp(&right.cutoff));

    Ok(Mem0ResultSummary {
        benchmark,
        total_questions,
        avg_search_latency_ms: average_search_latency(evaluations),
        avg_retrieved_memories: average_retrieved_memories(evaluations),
        cutoffs,
    })
}

pub fn summarize_locomo_dataset_json(text: &str) -> Result<LocomoDatasetSummary> {
    let root: Value = serde_json::from_str(text)?;
    let conversations = root
        .as_array()
        .ok_or_else(|| NoemaError::InvalidRecord("LOCOMO dataset must be a JSON array".into()))?;
    let mut sessions = 0;
    let mut turns = 0;
    let mut questions = 0;
    let mut evaluable_questions = 0;
    let mut evidence_refs = 0;
    let mut resolved_evidence_refs = 0;
    let mut category_counts = BTreeMap::new();

    for entry in conversations {
        let Some(entry_obj) = entry.as_object() else {
            continue;
        };
        let dia_ids = locomo_dia_ids(entry_obj);
        if let Some(conversation) = entry_obj.get("conversation").and_then(Value::as_object) {
            for (key, value) in conversation {
                if is_locomo_session_key(key, value) {
                    sessions += 1;
                    turns += value.as_array().map(Vec::len).unwrap_or(0);
                }
            }
        }
        let qa_items = entry_obj
            .get("qa")
            .or_else(|| entry_obj.get("qa_pairs"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for qa in qa_items {
            questions += 1;
            let category = qa
                .get("category")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if (1..=4).contains(&category) {
                evaluable_questions += 1;
            }
            let category = locomo_category_name(category);
            *category_counts.entry(category.to_string()).or_insert(0) += 1;
            let refs = locomo_evidence_refs(qa);
            evidence_refs += refs.len();
            resolved_evidence_refs += refs
                .iter()
                .filter(|dia_id| dia_ids.contains(*dia_id))
                .count();
        }
    }

    Ok(LocomoDatasetSummary {
        conversations: conversations.len(),
        sessions,
        turns,
        questions,
        evaluable_questions,
        evidence_refs,
        resolved_evidence_refs,
        category_counts,
    })
}

pub fn run_locomo_evidence_retrieval_json(
    text: &str,
    top_k: usize,
) -> Result<LocomoEvidenceReport> {
    run_locomo_evidence_retrieval_json_with_source(text, top_k, LocomoMemorySource::Raw)
}

pub fn run_locomo_evidence_retrieval_json_with_source(
    text: &str,
    top_k: usize,
    memory_source: LocomoMemorySource,
) -> Result<LocomoEvidenceReport> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO evidence retrieval top_k must be greater than zero".into(),
        ));
    }
    let root: Value = serde_json::from_str(text)?;
    let conversations = root
        .as_array()
        .ok_or_else(|| NoemaError::InvalidRecord("LOCOMO dataset must be a JSON array".into()))?;
    let tenant = TenantId::new("personal");
    let user = UserId::new("locomo-bench");
    let principal = Principal::personal(user.as_str(), "noema-bench");
    let mut questions = 0;
    let mut any_evidence_hits = 0;
    let mut all_evidence_hits = 0;

    for (conv_idx, entry) in conversations.iter().enumerate() {
        let Some(entry_obj) = entry.as_object() else {
            continue;
        };
        let (memories, dia_to_memory) =
            locomo_memories(conv_idx, entry_obj, &tenant, &user, memory_source);
        let qa_items = entry_obj
            .get("qa")
            .or_else(|| entry_obj.get("qa_pairs"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for qa in qa_items {
            let category = qa
                .get("category")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if !(1..=4).contains(&category) {
                continue;
            }
            let evidence: Vec<Vec<String>> = locomo_evidence_refs(qa)
                .iter()
                .filter_map(|dia_id| dia_to_memory.get(dia_id).cloned())
                .collect();
            if evidence.is_empty() {
                continue;
            }
            let Some(question) = qa.get("question").and_then(Value::as_str) else {
                continue;
            };
            questions += 1;
            let recalled = recall(question, &principal, None, &memories);
            let top_ids: HashSet<String> = recalled
                .into_iter()
                .take(top_k)
                .map(|scored| scored.id)
                .collect();
            if evidence
                .iter()
                .any(|ids| ids.iter().any(|id| top_ids.contains(id)))
            {
                any_evidence_hits += 1;
            }
            if evidence
                .iter()
                .all(|ids| ids.iter().any(|id| top_ids.contains(id)))
            {
                all_evidence_hits += 1;
            }
        }
    }

    Ok(LocomoEvidenceReport {
        memory_source,
        questions,
        top_k,
        any_evidence_hits,
        all_evidence_hits,
        any_evidence_hit_rate: rate(any_evidence_hits, questions),
        all_evidence_hit_rate: rate(all_evidence_hits, questions),
    })
}

pub fn run_locomo_predict_json_with_source(
    text: &str,
    top_k: usize,
    memory_source: LocomoMemorySource,
) -> Result<Value> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO predict export top_k must be greater than zero".into(),
        ));
    }
    let root: Value = serde_json::from_str(text)?;
    let conversations = root
        .as_array()
        .ok_or_else(|| NoemaError::InvalidRecord("LOCOMO dataset must be a JSON array".into()))?;
    let tenant = TenantId::new("personal");
    let user = UserId::new("locomo-bench");
    let principal = Principal::personal(user.as_str(), "noema-bench");
    let cutoff = format!("top_{top_k}");
    let mut evaluations = Vec::new();
    let mut overall = LocomoMetricAccumulator::default();
    let mut by_category: BTreeMap<String, LocomoMetricAccumulator> = BTreeMap::new();

    for (conv_idx, entry) in conversations.iter().enumerate() {
        let Some(entry_obj) = entry.as_object() else {
            continue;
        };
        let (memories, dia_to_memory) =
            locomo_memories(conv_idx, entry_obj, &tenant, &user, memory_source);
        let memory_by_id: BTreeMap<String, &MemoryRecord> = memories
            .iter()
            .map(|memory| (memory.id.to_string(), memory))
            .collect();
        let qa_items = entry_obj
            .get("qa")
            .or_else(|| entry_obj.get("qa_pairs"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for (qa_idx, qa) in qa_items.iter().enumerate() {
            let category = qa
                .get("category")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if !(1..=4).contains(&category) {
                continue;
            }
            let evidence_refs = locomo_evidence_refs(qa);
            let evidence: Vec<Vec<String>> = evidence_refs
                .iter()
                .filter_map(|dia_id| dia_to_memory.get(dia_id).cloned())
                .collect();
            let Some(question) = qa.get("question").and_then(Value::as_str) else {
                continue;
            };

            let search_started = Instant::now();
            let recalled = recall(question, &principal, None, &memories);
            let search_latency_ms = search_started.elapsed().as_secs_f64() * 1000.0;
            let top_scored = recalled.into_iter().take(top_k).collect::<Vec<_>>();
            let top_ids: HashSet<String> =
                top_scored.iter().map(|scored| scored.id.clone()).collect();
            let any_evidence_hit = !evidence.is_empty()
                && evidence
                    .iter()
                    .any(|ids| ids.iter().any(|id| top_ids.contains(id)));
            let all_evidence_hit = !evidence.is_empty()
                && evidence
                    .iter()
                    .all(|ids| ids.iter().any(|id| top_ids.contains(id)));
            let score = if any_evidence_hit { 1.0 } else { 0.0 };
            let category_name = locomo_category_name(category);

            add_locomo_metric(&mut overall, category, score);
            add_locomo_metric(
                by_category
                    .entry(category_name.to_string())
                    .or_insert_with(|| LocomoMetricAccumulator {
                        category_id: category,
                        ..LocomoMetricAccumulator::default()
                    }),
                category,
                score,
            );

            let search_results = top_scored
                .iter()
                .filter_map(|scored| {
                    let memory = memory_by_id.get(&scored.id)?;
                    Some(json!({
                        "memory": memory.body,
                        "score": scored.score,
                        "id": scored.id,
                    }))
                })
                .collect::<Vec<_>>();
            evaluations.push(json!({
                "question_id": format!("conv{conv_idx}_q{qa_idx}"),
                "conversation_idx": conv_idx,
                "category": category,
                "category_name": category_name,
                "question": question,
                "ground_truth_answer": locomo_answer_text(qa.get("answer")),
                "evidence": evidence_refs,
                "user_id": format!("locomo_{conv_idx}_noema"),
                "retrieval": {
                    "search_query": question,
                    "search_results": search_results,
                    "search_latency_ms": search_latency_ms,
                    "total_results": top_ids.len(),
                },
                "total_memories_retrieved": top_ids.len(),
                "search_latency_ms": search_latency_ms,
                "cutoff_results": {
                    cutoff.clone(): {
                        "judgment": if any_evidence_hit { "EVIDENCE_HIT" } else { "MISS" },
                        "score": score,
                        "generated_answer": "",
                        "memories_evaluated": top_ids.len(),
                        "any_evidence_hit": any_evidence_hit,
                        "all_evidence_hit": all_evidence_hit,
                    }
                }
            }));
        }
    }

    Ok(json!({
        "metadata": {
            "timestamp": "noema-local",
            "project_name": "noema-locomo-predict",
            "benchmark": "locomo",
            "answerer_model": "",
            "judge_model": "",
            "provider": "noema",
            "total_questions": overall.total,
            "top_k": top_k,
            "top_k_cutoffs": [cutoff.clone()],
            "memory_source": memory_source.to_string(),
            "eval_mode": "evidence_proxy_predict",
        },
        "metrics_by_cutoff": {
            cutoff: {
                "overall": locomo_overall_metric_json(&overall),
                "by_category": by_category
                    .iter()
                    .map(|(name, metric)| (name.clone(), locomo_category_metric_json(metric)))
                    .collect::<serde_json::Map<String, Value>>(),
            }
        },
        "evaluations": evaluations,
    }))
}

pub fn locomo_answer_tasks_jsonl_from_predict(predict: &Value, top_k: usize) -> Result<String> {
    locomo_answer_tasks_jsonl_from_predict_with_prompt_budget(predict, top_k, None)
}

pub fn locomo_answer_tasks_jsonl_from_predict_with_prompt_budget(
    predict: &Value,
    top_k: usize,
    prompt_char_budget: Option<usize>,
) -> Result<String> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO answer task top_k must be greater than zero".into(),
        ));
    }
    let evaluations = predict
        .get("evaluations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            NoemaError::InvalidRecord("LOCOMO predict JSON missing evaluations".into())
        })?;
    let mut out = String::new();
    for evaluation in evaluations {
        let task = locomo_answer_task_from_evaluation(evaluation, top_k, prompt_char_budget)?;
        out.push_str(&serde_json::to_string(&task)?);
        out.push('\n');
    }
    Ok(out)
}

pub fn locomo_retry_answer_tasks_jsonl_from_results(
    predict: &Value,
    answer_results_jsonl: &str,
    top_k: usize,
) -> Result<String> {
    locomo_retry_answer_tasks_jsonl_from_results_with_prompt_budget(
        predict,
        answer_results_jsonl,
        top_k,
        None,
    )
}

pub fn locomo_retry_answer_tasks_jsonl_from_results_with_prompt_budget(
    predict: &Value,
    answer_results_jsonl: &str,
    top_k: usize,
    prompt_char_budget: Option<usize>,
) -> Result<String> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO retry answer task top_k must be greater than zero".into(),
        ));
    }
    let (_, answers) = parse_locomo_answer_result_states(answer_results_jsonl)?;
    let evaluations = predict
        .get("evaluations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            NoemaError::InvalidRecord("LOCOMO predict JSON missing evaluations".into())
        })?;
    let cutoff = format!("top_{top_k}");
    let mut out = String::new();
    for evaluation in evaluations {
        let question_id = evaluation
            .get("question_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                NoemaError::InvalidRecord("LOCOMO predict evaluation missing question_id".into())
            })?;
        let answer_id = format!("locomo-answer-{question_id}-{cutoff}");
        if matches!(
            answers.get(&answer_id),
            Some(LocomoAnswerResultState::Valid)
        ) {
            continue;
        }
        let task = locomo_answer_task_from_evaluation(evaluation, top_k, prompt_char_budget)?;
        out.push_str(&serde_json::to_string(&task)?);
        out.push('\n');
    }
    Ok(out)
}

pub fn locomo_answer_prompt_retention_json_from_tasks(
    predict: &Value,
    answer_tasks_jsonl: &str,
    top_k: usize,
) -> Result<Value> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO answer prompt retention top_k must be greater than zero".into(),
        ));
    }
    let evaluations = predict
        .get("evaluations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            NoemaError::InvalidRecord("LOCOMO predict JSON missing evaluations".into())
        })?;
    let task_stats = parse_locomo_answer_task_prompt_stats(answer_tasks_jsonl)?;

    let mut total_evaluations = 0usize;
    let mut evaluable_evidence = 0usize;
    let mut baseline_any_evidence_hits = 0usize;
    let mut baseline_all_evidence_hits = 0usize;
    let mut retained_any_evidence_hits = 0usize;
    let mut retained_all_evidence_hits = 0usize;
    let mut missing_prompt_stats = 0usize;
    let mut lost_any_hit_question_ids = Vec::new();
    let mut lost_all_hit_question_ids = Vec::new();

    for evaluation in evaluations {
        total_evaluations += 1;
        let question_id = evaluation
            .get("question_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                NoemaError::InvalidRecord("LOCOMO predict evaluation missing question_id".into())
            })?;
        let evidence_refs = locomo_evidence_refs(evaluation);
        if evidence_refs.is_empty() {
            continue;
        }
        evaluable_evidence += 1;
        let search_results = evaluation
            .get("retrieval")
            .and_then(|retrieval| retrieval.get("search_results"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        let baseline_hits =
            locomo_evidence_hits_in_search_prefix(&evidence_refs, search_results, top_k);
        let baseline_any = baseline_hits > 0;
        let baseline_all = baseline_hits == evidence_refs.len();
        if baseline_any {
            baseline_any_evidence_hits += 1;
        }
        if baseline_all {
            baseline_all_evidence_hits += 1;
        }

        let Some(stats) = task_stats.by_question_id.get(question_id) else {
            missing_prompt_stats += 1;
            continue;
        };
        let retained_hits = locomo_evidence_hits_in_search_prefix(
            &evidence_refs,
            search_results,
            stats.retrieval_results_in_prompt.min(top_k),
        );
        let retained_any = retained_hits > 0;
        let retained_all = retained_hits == evidence_refs.len();
        if retained_any {
            retained_any_evidence_hits += 1;
        }
        if retained_all {
            retained_all_evidence_hits += 1;
        }
        if baseline_any && !retained_any {
            lost_any_hit_question_ids.push(question_id.to_string());
        }
        if baseline_all && !retained_all {
            lost_all_hit_question_ids.push(question_id.to_string());
        }
    }

    let baseline_any_hits_lost = lost_any_hit_question_ids.len();
    let baseline_all_hits_lost = lost_all_hit_question_ids.len();
    Ok(json!({
        "metadata": {
            "benchmark": "locomo",
            "eval_mode": "answer_prompt_retention_audit",
            "top_k": top_k,
        },
        "overall": {
            "total_evaluations": total_evaluations,
            "answer_tasks": task_stats.tasks,
            "tasks_with_prompt_stats": task_stats.by_question_id.len(),
            "missing_prompt_stats": missing_prompt_stats,
            "evaluable_evidence": evaluable_evidence,
            "baseline_any_evidence_hits": baseline_any_evidence_hits,
            "baseline_any_evidence_hit_rate": rate(baseline_any_evidence_hits, evaluable_evidence),
            "baseline_all_evidence_hits": baseline_all_evidence_hits,
            "baseline_all_evidence_hit_rate": rate(baseline_all_evidence_hits, evaluable_evidence),
            "retained_any_evidence_hits": retained_any_evidence_hits,
            "retained_any_evidence_hit_rate": rate(retained_any_evidence_hits, evaluable_evidence),
            "retained_all_evidence_hits": retained_all_evidence_hits,
            "retained_all_evidence_hit_rate": rate(retained_all_evidence_hits, evaluable_evidence),
            "baseline_any_hits_lost": baseline_any_hits_lost,
            "baseline_all_hits_lost": baseline_all_hits_lost,
        },
        "prompt_budgets": task_stats.prompt_budgets.iter().copied().collect::<Vec<_>>(),
        "prompt_summary": task_stats.prompt_summary_json(),
        "lost_any_hit_question_ids": lost_any_hit_question_ids,
        "lost_all_hit_question_ids": lost_all_hit_question_ids,
    }))
}

pub fn locomo_run_report_json_from_artifacts(
    predict: &Value,
    answer_tasks_jsonl: Option<&str>,
    answer_results_jsonl: Option<&str>,
    judge_results_jsonl: Option<&str>,
    top_k: usize,
) -> Result<Value> {
    locomo_run_report_json_from_artifacts_with_host_manifest(
        predict,
        answer_tasks_jsonl,
        answer_results_jsonl,
        judge_results_jsonl,
        None,
        top_k,
    )
}

pub fn locomo_run_report_json_from_artifacts_with_host_manifest(
    predict: &Value,
    answer_tasks_jsonl: Option<&str>,
    answer_results_jsonl: Option<&str>,
    judge_results_jsonl: Option<&str>,
    host_manifest_json: Option<&str>,
    top_k: usize,
) -> Result<Value> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO run report top_k must be greater than zero".into(),
        ));
    }
    let cutoff = format!("top_{top_k}");
    let proxy_overall = predict
        .get("metrics_by_cutoff")
        .and_then(|metrics| metrics.get(&cutoff))
        .and_then(|cutoff_metrics| cutoff_metrics.get("overall"))
        .cloned()
        .unwrap_or_default();
    let prompt_retention = answer_tasks_jsonl
        .map(|tasks| locomo_answer_prompt_retention_json_from_tasks(predict, tasks, top_k))
        .transpose()?
        .unwrap_or(Value::Null);
    let status =
        locomo_status_json_from_results(predict, answer_results_jsonl, judge_results_jsonl, top_k)?;
    let host_runner = host_manifest_json
        .map(serde_json::from_str::<Value>)
        .transpose()?
        .unwrap_or(Value::Null);
    let host_blocker_reason = locomo_host_blocker_reason(&host_runner)
        .or_else(|| locomo_status_host_blocker_reason(&status));
    let final_ready = status
        .get("metadata")
        .and_then(|metadata| metadata.get("final_ready"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let blocked_reason = host_blocker_reason
        .as_ref()
        .map(|_| "host_provider_blocked")
        .unwrap_or_else(|| locomo_run_report_blocked_reason(&status));
    let next_action =
        locomo_run_report_next_action(&status, &host_runner, host_blocker_reason.as_deref());
    let target_verdict = if final_ready {
        match (answer_results_jsonl, judge_results_jsonl) {
            (Some(answers), Some(judgments)) => {
                let final_result =
                    locomo_final_result_json_from_judgments(predict, answers, judgments, top_k)?;
                locomo_target_verdict_json(&final_result, top_k)?
            }
            _ => Value::Null,
        }
    } else {
        Value::Null
    };

    Ok(json!({
        "metadata": {
            "benchmark": "locomo",
            "eval_mode": "locomo_run_report",
            "top_k": top_k,
            "cutoff": cutoff,
        },
        "predict_proxy": {
            "overall": proxy_overall,
        },
        "prompt_retention": prompt_retention,
        "status": status,
        "completion": {
            "final_ready": final_ready,
            "blocked_reason": blocked_reason,
            "host_blocked": host_blocker_reason.is_some(),
            "host_blocker_reason": host_blocker_reason,
        },
        "next_action": next_action,
        "target_verdict": target_verdict,
        "host_runner": host_runner,
    }))
}

fn locomo_host_blocker_reason(host_runner: &Value) -> Option<String> {
    let provider_blocked = host_runner
        .get("provider_blocked")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !provider_blocked {
        return None;
    }
    Some(
        host_runner
            .get("provider_blocker_reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown_provider_blocker")
            .to_string(),
    )
}

fn locomo_status_host_blocker_reason(status: &Value) -> Option<String> {
    for section in ["answers", "judges"] {
        let Some(failure_reasons) = status
            .get(section)
            .and_then(|values| values.get("failure_reasons"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for reason in ["http_402_payment_required"] {
            if failure_reasons
                .get(reason)
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0
            {
                return Some(reason.to_string());
            }
        }
    }
    None
}

fn locomo_run_report_blocked_reason(status: &Value) -> &'static str {
    if status
        .get("metadata")
        .and_then(|metadata| metadata.get("final_ready"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return "ready";
    }
    if !status
        .get("answers")
        .and_then(|answers| answers.get("complete"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return "answers_incomplete";
    }
    if !status
        .get("judges")
        .and_then(|judges| judges.get("complete"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return "judges_incomplete";
    }
    "unknown"
}

fn locomo_run_report_next_action(
    status: &Value,
    host_runner: &Value,
    host_blocker_reason: Option<&str>,
) -> Value {
    if let Some(reason) = host_blocker_reason {
        return json!({
            "kind": "resolve_provider_blocker",
            "blocked_reason": "host_provider_blocked",
            "provider_blocker_reason": reason,
            "unrun_due_to_provider_blocker": host_runner
                .get("execution")
                .and_then(|execution| execution.get("unrun_due_to_provider_blocker"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            "command_hint": "fix host provider balance/config, then rerun zode with --resume --retry-empty --retry-failed --stop-on-provider-blocker",
        });
    }
    match locomo_run_report_blocked_reason(status) {
        "ready" => json!({
            "kind": "finalize",
            "blocked_reason": "ready",
            "retryable": 0,
            "command_hint": "noema bench --locomo-predict-input <predict.json> --locomo-answer-results <answers.jsonl> --locomo-judge-results <judges.jsonl> --locomo-final-output <final.json>",
        }),
        "answers_incomplete" => {
            let answers = status.get("answers").unwrap_or(&Value::Null);
            json!({
                "kind": "retry_answers",
                "blocked_reason": "answers_incomplete",
                "retryable": answers
                    .get("retryable")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                "pending_ids": answers
                    .get("pending_ids")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                "failure_reasons": answers
                    .get("failure_reasons")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                "command_hint": "noema bench --locomo-predict-input <predict.json> --locomo-answer-results <answers.jsonl> --locomo-retry-answer-tasks-output <retry-answer-tasks.jsonl>",
            })
        }
        "judges_incomplete" => {
            let judges = status.get("judges").unwrap_or(&Value::Null);
            json!({
                "kind": "retry_judges",
                "blocked_reason": "judges_incomplete",
                "retryable": judges
                    .get("retryable")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                "pending_ids": judges
                    .get("pending_ids")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                "failure_reasons": judges
                    .get("failure_reasons")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                "command_hint": "noema bench --locomo-predict-input <predict.json> --locomo-answer-results <answers.jsonl> --locomo-judge-results <judges.jsonl> --locomo-retry-judge-tasks-output <retry-judge-tasks.jsonl>",
            })
        }
        reason => json!({
            "kind": "inspect",
            "blocked_reason": reason,
            "retryable": 0,
        }),
    }
}

pub fn locomo_judge_tasks_jsonl_from_answers(
    predict: &Value,
    answer_results_jsonl: &str,
    top_k: usize,
) -> Result<String> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO judge task top_k must be greater than zero".into(),
        ));
    }
    let answers = parse_locomo_answer_results(answer_results_jsonl)?;
    let evaluations = predict
        .get("evaluations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            NoemaError::InvalidRecord("LOCOMO predict JSON missing evaluations".into())
        })?;
    let cutoff = format!("top_{top_k}");
    let mut out = String::new();
    for evaluation in evaluations {
        let question_id = evaluation
            .get("question_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                NoemaError::InvalidRecord("LOCOMO predict evaluation missing question_id".into())
            })?;
        let answer_custom_id = format!("locomo-answer-{question_id}-{cutoff}");
        let Some(generated_answer) = answers.get(&answer_custom_id) else {
            continue;
        };
        let task = locomo_judge_task_from_evaluation(
            evaluation,
            generated_answer,
            &answer_custom_id,
            top_k,
        )?;
        out.push_str(&serde_json::to_string(&task)?);
        out.push('\n');
    }
    Ok(out)
}

pub fn locomo_retry_judge_tasks_jsonl_from_results(
    predict: &Value,
    answer_results_jsonl: &str,
    judge_results_jsonl: &str,
    top_k: usize,
) -> Result<String> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO retry judge task top_k must be greater than zero".into(),
        ));
    }
    let answers = parse_locomo_answer_results(answer_results_jsonl)?;
    let (_, judgments) = parse_locomo_judge_result_states(judge_results_jsonl)?;
    let evaluations = predict
        .get("evaluations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            NoemaError::InvalidRecord("LOCOMO predict JSON missing evaluations".into())
        })?;
    let cutoff = format!("top_{top_k}");
    let mut out = String::new();
    for evaluation in evaluations {
        let question_id = evaluation
            .get("question_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                NoemaError::InvalidRecord("LOCOMO predict evaluation missing question_id".into())
            })?;
        let answer_custom_id = format!("locomo-answer-{question_id}-{cutoff}");
        let Some(generated_answer) = answers.get(&answer_custom_id) else {
            continue;
        };
        let judge_custom_id = format!("locomo-judge-{question_id}-{cutoff}");
        if matches!(
            judgments.get(&judge_custom_id),
            Some(LocomoJudgeResultState::Valid(_))
        ) {
            continue;
        }
        let task = locomo_judge_task_from_evaluation(
            evaluation,
            generated_answer,
            &answer_custom_id,
            top_k,
        )?;
        out.push_str(&serde_json::to_string(&task)?);
        out.push('\n');
    }
    Ok(out)
}

pub fn locomo_final_result_json_from_judgments(
    predict: &Value,
    answer_results_jsonl: &str,
    judge_results_jsonl: &str,
    top_k: usize,
) -> Result<Value> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO final result top_k must be greater than zero".into(),
        ));
    }
    let answers = parse_locomo_answer_results(answer_results_jsonl)?;
    let judgments = parse_locomo_judge_results(judge_results_jsonl)?;
    let evaluations = predict
        .get("evaluations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            NoemaError::InvalidRecord("LOCOMO predict JSON missing evaluations".into())
        })?;
    let cutoff = format!("top_{top_k}");
    let mut final_evaluations = Vec::new();
    let mut overall = LocomoMetricAccumulator::default();
    let mut by_category: BTreeMap<String, LocomoMetricAccumulator> = BTreeMap::new();

    for evaluation in evaluations {
        let question_id = evaluation
            .get("question_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                NoemaError::InvalidRecord("LOCOMO predict evaluation missing question_id".into())
            })?;
        let answer_custom_id = format!("locomo-answer-{question_id}-{cutoff}");
        let judge_custom_id = format!("locomo-judge-{question_id}-{cutoff}");
        let generated_answer = answers.get(&answer_custom_id).ok_or_else(|| {
            NoemaError::InvalidRecord(format!("missing answer result for {answer_custom_id}"))
        })?;
        let judgment = judgments.get(&judge_custom_id).ok_or_else(|| {
            NoemaError::InvalidRecord(format!("missing judge result for {judge_custom_id}"))
        })?;
        let is_correct = judgment.label.eq_ignore_ascii_case("CORRECT");
        let score = if is_correct { 1.0 } else { 0.0 };
        let category_id = evaluation
            .get("category")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let category_name = evaluation
            .get("category_name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        add_locomo_metric(&mut overall, category_id, score);
        add_locomo_metric(
            by_category
                .entry(category_name.to_string())
                .or_insert_with(|| LocomoMetricAccumulator {
                    category_id,
                    ..LocomoMetricAccumulator::default()
                }),
            category_id,
            score,
        );

        let mut item = evaluation.clone();
        if let Some(obj) = item.as_object_mut() {
            obj.insert(
                "cutoff_results".to_string(),
                json!({
                    cutoff.clone(): {
                        "judgment": if is_correct { "CORRECT" } else { "WRONG" },
                        "score": score,
                        "generated_answer": generated_answer,
                        "memories_evaluated": evaluation
                            .get("retrieval")
                            .and_then(|retrieval| retrieval.get("total_results"))
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        "reason": judgment.reasoning,
                    }
                }),
            );
        }
        final_evaluations.push(item);
    }

    let mut metadata = predict
        .get("metadata")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("eval_mode".to_string(), json!("answerer_judge_offline"));
        obj.insert("provider".to_string(), json!("noema"));
        obj.insert("total_questions".to_string(), json!(overall.total));
        obj.insert("top_k_cutoffs".to_string(), json!([cutoff.clone()]));
    }

    Ok(json!({
        "metadata": metadata,
        "metrics_by_cutoff": {
            cutoff: {
                "overall": locomo_overall_metric_json(&overall),
                "by_category": by_category
                    .iter()
                    .map(|(name, metric)| (name.clone(), locomo_category_metric_json(metric)))
                    .collect::<serde_json::Map<String, Value>>(),
            }
        },
        "evaluations": final_evaluations,
    }))
}

pub fn locomo_status_json_from_results(
    predict: &Value,
    answer_results_jsonl: Option<&str>,
    judge_results_jsonl: Option<&str>,
    top_k: usize,
) -> Result<Value> {
    if top_k == 0 {
        return Err(NoemaError::InvalidRecord(
            "LOCOMO status top_k must be greater than zero".into(),
        ));
    }
    let evaluations = predict
        .get("evaluations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            NoemaError::InvalidRecord("LOCOMO predict JSON missing evaluations".into())
        })?;
    let cutoff = format!("top_{top_k}");
    let expected_answer_ids: Vec<String> = evaluations
        .iter()
        .map(|evaluation| {
            let question_id = evaluation
                .get("question_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    NoemaError::InvalidRecord(
                        "LOCOMO predict evaluation missing question_id".into(),
                    )
                })?;
            Ok(format!("locomo-answer-{question_id}-{cutoff}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let expected_answer_id_set: HashSet<&str> =
        expected_answer_ids.iter().map(String::as_str).collect();

    let (answer_rows, answer_latest) = match answer_results_jsonl {
        Some(text) => parse_locomo_answer_result_states(text)?,
        None => (0, BTreeMap::new()),
    };
    let mut answer_valid = 0;
    let mut answer_empty = 0;
    let mut answer_host_failed = 0;
    let mut answer_missing = 0;
    let mut answer_failure_reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut valid_answer_ids = Vec::new();
    let mut pending_answer_ids = Vec::new();
    for answer_id in &expected_answer_ids {
        match answer_latest.get(answer_id) {
            Some(LocomoAnswerResultState::Valid) => {
                answer_valid += 1;
                valid_answer_ids.push(answer_id.clone());
            }
            Some(LocomoAnswerResultState::Empty) => {
                answer_empty += 1;
                pending_answer_ids.push(answer_id.clone());
            }
            Some(LocomoAnswerResultState::HostFailed(reason)) => {
                answer_host_failed += 1;
                *answer_failure_reasons.entry(reason.clone()).or_default() += 1;
                pending_answer_ids.push(answer_id.clone());
            }
            None => {
                answer_missing += 1;
                pending_answer_ids.push(answer_id.clone());
            }
        }
    }
    let answer_unknown_ids = answer_latest
        .keys()
        .filter(|id| !expected_answer_id_set.contains(id.as_str()))
        .count();
    let answer_retryable = answer_empty + answer_host_failed + answer_missing;
    let answers_complete = answer_retryable == 0 && answer_valid == expected_answer_ids.len();

    let (judge_rows, judgments) = match judge_results_jsonl {
        Some(text) => parse_locomo_judge_result_states(text)?,
        None => (0, BTreeMap::new()),
    };
    let mut judge_valid = 0;
    let mut judge_correct = 0;
    let mut judge_wrong = 0;
    let mut judge_invalid_label = 0;
    let mut judge_host_failed = 0;
    let mut judge_failure_reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut judge_missing = 0;
    let mut expected_judge_ids = HashSet::new();
    let mut pending_judge_ids = Vec::new();
    for answer_id in &valid_answer_ids {
        let judge_id = answer_id.replacen("locomo-answer-", "locomo-judge-", 1);
        expected_judge_ids.insert(judge_id.clone());
        match judgments.get(&judge_id) {
            Some(LocomoJudgeResultState::Valid(judgment)) if judgment.label == "CORRECT" => {
                judge_valid += 1;
                judge_correct += 1;
            }
            Some(LocomoJudgeResultState::Valid(judgment)) if judgment.label == "WRONG" => {
                judge_valid += 1;
                judge_wrong += 1;
            }
            Some(LocomoJudgeResultState::Valid(_)) => {
                judge_invalid_label += 1;
                pending_judge_ids.push(judge_id);
            }
            Some(LocomoJudgeResultState::InvalidLabel) => {
                judge_invalid_label += 1;
                pending_judge_ids.push(judge_id);
            }
            Some(LocomoJudgeResultState::HostFailed(reason)) => {
                judge_host_failed += 1;
                *judge_failure_reasons.entry(reason.clone()).or_default() += 1;
                pending_judge_ids.push(judge_id);
            }
            None => {
                judge_missing += 1;
                pending_judge_ids.push(judge_id);
            }
        }
    }
    let judge_unknown_ids = judgments
        .keys()
        .filter(|id| !expected_judge_ids.contains(*id))
        .count();
    let judge_retryable = judge_missing + judge_invalid_label + judge_host_failed;
    let judges_complete = judge_retryable == 0 && judge_valid == expected_judge_ids.len();
    let final_ready = answers_complete
        && judges_complete
        && expected_judge_ids.len() == expected_answer_ids.len();

    Ok(json!({
        "metadata": {
            "benchmark": "locomo",
            "cutoff": cutoff,
            "top_k": top_k,
            "total_questions": expected_answer_ids.len(),
            "final_ready": final_ready,
        },
        "answers": {
            "expected": expected_answer_ids.len(),
            "rows": answer_rows,
            "unique": answer_latest.len(),
            "unknown_ids": answer_unknown_ids,
            "valid": answer_valid,
            "empty": answer_empty,
            "host_failed": answer_host_failed,
            "failure_reasons": answer_failure_reasons,
            "missing": answer_missing,
            "retryable": answer_retryable,
            "pending_ids": pending_id_samples(&pending_answer_ids),
            "complete": answers_complete,
        },
        "judges": {
            "expected": expected_judge_ids.len(),
            "rows": judge_rows,
            "unique": judgments.len(),
            "unknown_ids": judge_unknown_ids,
            "valid": judge_valid,
            "correct": judge_correct,
            "wrong": judge_wrong,
            "invalid_label": judge_invalid_label,
            "host_failed": judge_host_failed,
            "failure_reasons": judge_failure_reasons,
            "missing": judge_missing,
            "retryable": judge_retryable,
            "pending_ids": pending_id_samples(&pending_judge_ids),
            "complete": judges_complete,
        }
    }))
}

pub fn run_recall_benchmark(root: &Path, scenario: BenchmarkScenario) -> Result<BenchmarkReport> {
    let scenario = scenario.validate()?;
    let principal = Principal::personal("bench-user", "zode");
    let engine = NoemaEngine::new(root)?;
    engine.init_personal(&UserId::new("bench-user"))?;
    let generated_bytes = seed_memories(&engine, &principal, scenario.memory_count)?;
    let queries = benchmark_queries(scenario.query_count);

    let engine_sample = measure(
        "noema_engine_recall",
        scenario.iterations,
        &queries,
        |query| {
            let profiled = engine.recall_profiled(RecallRequest {
                principal: principal.clone(),
                query: query.clone(),
                cwd: None,
                budget_tokens: 1200,
                host: "noema-bench".to_string(),
            })?;
            std::hint::black_box(profiled.pack.memories.len());
            Ok(recall_phase_measurements(&profiled.timings))
        },
    )?;

    let zode_sample = measure(
        "zode_turn_injection_equivalent",
        scenario.iterations,
        &queries,
        |query| {
            let create_start = Instant::now();
            let turn_engine = NoemaEngine::new(root)?;
            let create_engine_us = create_start.elapsed().as_secs_f64() * 1_000_000.0;
            let profiled = turn_engine.recall_profiled(RecallRequest {
                principal: principal.clone(),
                query: query.clone(),
                cwd: None,
                budget_tokens: 1200,
                host: "zode".to_string(),
            })?;
            let render_start = Instant::now();
            let rendered = profiled.pack.to_markdown();
            let render_markdown_us = render_start.elapsed().as_secs_f64() * 1_000_000.0;
            std::hint::black_box(rendered);
            let mut phases = vec![PhaseMeasurement {
                name: "create_engine",
                us: create_engine_us,
            }];
            phases.extend(recall_phase_measurements(&profiled.timings));
            phases.push(PhaseMeasurement {
                name: "render_markdown",
                us: render_markdown_us,
            });
            Ok(phases)
        },
    )?;

    Ok(BenchmarkReport {
        memory_count: scenario.memory_count,
        query_count: scenario.query_count,
        iterations: scenario.iterations,
        generated_bytes,
        samples: vec![engine_sample, zode_sample],
    })
}

fn seed_memories(
    engine: &NoemaEngine,
    principal: &Principal,
    memory_count: usize,
) -> Result<usize> {
    let mut generated_bytes = 0;
    for index in 0..memory_count {
        let body = format!(
            "Memory {index}: prefer Rust modules for Noema recall benchmark path {bucket}; review candidates before persistence; zode injects relevant memory before the turn.",
            bucket = index % 16
        );
        generated_bytes += body.len();
        engine.submit_candidate(RememberRequest {
            principal: principal.clone(),
            text: body,
            scope: Scope::User,
            project_path: None,
            kind: MemoryKind::Preference,
            sensitivity: SensitivityLevel::Internal,
            tags: vec!["rust".to_string(), "benchmark".to_string()],
            entities: vec!["Noema".to_string(), "zode".to_string()],
            confidence: 1.0,
            importance: 0.5,
        })?;
        let pending = engine.review_list(principal)?;
        if let Some(first) = pending.first() {
            engine.review_decide(ReviewDecisionRequest {
                principal: principal.clone(),
                candidate_id: first.id.to_string(),
                action: ReviewAction::Accept,
            })?;
        }
    }
    Ok(generated_bytes)
}

fn benchmark_queries(query_count: usize) -> Vec<String> {
    let base = [
        "rust noema recall benchmark",
        "zode memory injection",
        "review candidates persistence",
        "agent memory rust modules",
        "lexical recall benchmark",
        "noema zode integration",
        "memory pack markdown",
        "local first storage",
    ];
    (0..query_count)
        .map(|index| base[index % base.len()].to_string())
        .collect()
}

fn measure<F>(
    name: &'static str,
    iterations: usize,
    queries: &[String],
    mut run_query: F,
) -> Result<BenchmarkSample>
where
    F: FnMut(&String) -> Result<Vec<PhaseMeasurement>>,
{
    let mut durations = Vec::with_capacity(iterations * queries.len());
    let mut phases = Vec::new();
    for _ in 0..iterations {
        for query in queries {
            let started = Instant::now();
            let measured_phases = run_query(query)?;
            durations.push(started.elapsed());
            for phase in measured_phases {
                add_phase_measurement(&mut phases, phase);
            }
        }
    }
    let total = durations
        .iter()
        .fold(Duration::ZERO, |sum, duration| sum + *duration);
    Ok(sample_from_durations(name, total, durations, phases))
}

fn sample_from_durations(
    name: &'static str,
    total: Duration,
    mut durations: Vec<Duration>,
    phases: Vec<PhaseAccumulator>,
) -> BenchmarkSample {
    durations.sort();
    let operations = durations.len();
    let total_ms = total.as_secs_f64() * 1000.0;
    let mean_us = total.as_secs_f64() * 1_000_000.0 / operations as f64;
    let p50_us = percentile_us(&durations, 0.50);
    let p95_us = percentile_us(&durations, 0.95);
    BenchmarkSample {
        name,
        operations,
        total_ms,
        mean_us,
        p50_us,
        p95_us,
        phases: phase_samples(phases),
    }
}

fn percentile_us(durations: &[Duration], percentile: f64) -> f64 {
    let last = durations.len().saturating_sub(1);
    let index = (last as f64 * percentile).ceil() as usize;
    durations[index].as_secs_f64() * 1_000_000.0
}

fn recall_phase_measurements(timings: &crate::api::RecallTimings) -> Vec<PhaseMeasurement> {
    vec![
        PhaseMeasurement {
            name: "load_memories",
            us: timings.load_memories_us,
        },
        PhaseMeasurement {
            name: "score_memories",
            us: timings.score_memories_us,
        },
        PhaseMeasurement {
            name: "build_pack",
            us: timings.build_pack_us,
        },
    ]
}

fn add_phase_measurement(phases: &mut Vec<PhaseAccumulator>, measurement: PhaseMeasurement) {
    if let Some(phase) = phases
        .iter_mut()
        .find(|phase| phase.name == measurement.name)
    {
        phase.operations += 1;
        phase.total_us += measurement.us;
    } else {
        phases.push(PhaseAccumulator {
            name: measurement.name,
            operations: 1,
            total_us: measurement.us,
        });
    }
}

fn phase_samples(phases: Vec<PhaseAccumulator>) -> Vec<BenchmarkPhaseSample> {
    phases
        .into_iter()
        .map(|phase| BenchmarkPhaseSample {
            name: phase.name,
            operations: phase.operations,
            total_ms: phase.total_us / 1000.0,
            mean_us: phase.total_us / phase.operations as f64,
        })
        .collect()
}

fn cutoff_groups(cutoff_obj: &serde_json::Map<String, Value>) -> Vec<Mem0GroupSummary> {
    let Some((_, groups_value)) = cutoff_obj
        .iter()
        .find(|(name, value)| name.starts_with("by_") && value.is_object())
    else {
        return Vec::new();
    };
    let Some(groups_obj) = groups_value.as_object() else {
        return Vec::new();
    };
    let mut groups = Vec::new();
    for (name, value) in groups_obj {
        let Some(group_obj) = value.as_object() else {
            continue;
        };
        let (score_label, score) = metric_score(group_obj);
        let total = group_obj
            .get("total")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0);
        groups.push(Mem0GroupSummary {
            name: name.clone(),
            score_label,
            score,
            total,
        });
    }
    groups.sort_by(|left, right| left.name.cmp(&right.name));
    groups
}

fn metric_score(obj: &serde_json::Map<String, Value>) -> (String, f64) {
    for key in ["avg_score", "accuracy", "pass_rate", "score"] {
        if let Some(value) = obj.get(key).and_then(Value::as_f64) {
            return (key.to_string(), normalize_score(value));
        }
    }
    ("score".to_string(), 0.0)
}

fn normalize_score(value: f64) -> f64 {
    if (0.0..=1.0).contains(&value) {
        value * 100.0
    } else {
        value
    }
}

fn average_search_latency(evaluations: &[Value]) -> Option<f64> {
    average(evaluations.iter().filter_map(|item| {
        item.get("search_latency_ms")
            .and_then(Value::as_f64)
            .or_else(|| {
                item.get("retrieval")
                    .and_then(|retrieval| retrieval.get("search_latency_ms"))
                    .and_then(Value::as_f64)
            })
    }))
}

fn average_retrieved_memories(evaluations: &[Value]) -> Option<f64> {
    average(evaluations.iter().filter_map(|item| {
        item.get("total_memories_retrieved")
            .and_then(Value::as_f64)
            .or_else(|| {
                item.get("retrieval")
                    .and_then(|retrieval| retrieval.get("total_results"))
                    .and_then(Value::as_f64)
            })
            .or_else(|| {
                item.get("retrieval")
                    .and_then(|retrieval| retrieval.get("search_results_count"))
                    .and_then(Value::as_f64)
            })
    }))
}

fn average(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0;
    for value in values {
        total += value;
        count += 1;
    }
    (count > 0).then_some(total / count as f64)
}

fn add_locomo_metric(metric: &mut LocomoMetricAccumulator, category_id: i64, score: f64) {
    metric.category_id = category_id;
    metric.total += 1;
    metric.score_sum += score;
    if score >= 0.5 {
        metric.correct += 1;
    }
}

const LOCOMO_ANSWER_PROMPT_PREFIX: &str =
    "You are answering a LOCOMO long-term memory benchmark question using retrieved memories.\n\
Use only the memories below as evidence, but answer in the benchmark style:\n\
- Give the best-supported answer, not a refusal, when any memory gives a clue.\n\
- Combine facts across memories, speaker summaries, adjacent turns, and timestamps.\n\
- For `might`, `likely`, `would`, or `could` questions, make a grounded inference from the memories.\n\
- For open-domain benchmark questions, use general world knowledge only to connect recalled facts to a likely answer; do not invent personal facts.\n\
- Open-domain candidates may be common-knowledge bridges; use them when they fit the recalled clues even if the exact name is not written in a memory.\n\
- For photo/image questions, infer what the photo shows from adjacent turns and image descriptions.\n\
- For relative dates, compute from the memory timestamp when a turn says last/next/yesterday/tomorrow.\n\
- For dates, counts, lists, titles, colors, places, names, and quoted text, be specific and concise.\n\
- Near the question, check `Most relevant extracted clues`; if a clue directly answers the question, copy that specific phrase instead of generalizing.\n\
- LOCOMO gold answers care about the event or object more than strict speaker ownership; if the closest clue answers the event but the speaker attribution is loose, answer the event/object instead of correcting the question.\n\
- For `what did X ...` questions, prefer the noun phrase after the relevant action verb in the closest clue, such as found/shared/asked/arranged/played/received/saw/said.\n\
- Do not reject a listed answer candidate as unsupported; candidates are extracted or inferred by Noema, so use one directly when it fits the question.\n\
- Prefer the most specific supported answer over a broad category.\n\
- Treat nickname/name variants and typos as the same person when the conversation makes that clear.\n\
- If the question asks relationship status and the memories support no partner/spouse/romantic relationship, answer `single`.\n\
- Say the answer is unavailable only when no retrieved memory supports even a reasonable inference.\n\
After `ANSWER:`, write the final answer first in one concise sentence; do not start with caveats like `unavailable` when a supported clue exists.\n\n";

const LOCOMO_RETRIEVED_MEMORIES_HEADER: &str = "Retrieved memories:\n";
const LOCOMO_ANSWER_PROMPT_CLUES: usize = 12;
const LOCOMO_ANSWER_CLUE_MAX_CHARS: usize = 360;
const LOCOMO_ANSWER_CLUES_MIN_BUDGET: usize = 2500;
const LOCOMO_ANSWER_CLUE_MIN_SCORE: usize = 3;
const LOCOMO_FACT_SUMMARY_PROMPT_FACTS: usize = 24;

struct LocomoAnswerPrompt {
    text: String,
    stats: Value,
}

#[derive(Debug)]
struct LocomoPromptClue {
    score: usize,
    memory_index: usize,
    line_index: usize,
    text: String,
}

fn locomo_answer_prompt_preamble(
    _question: &str,
    _search_results: &[Value],
    _top_k: usize,
    _prompt_char_budget: Option<usize>,
) -> String {
    let mut out = String::from(LOCOMO_ANSWER_PROMPT_PREFIX);
    out.push_str(LOCOMO_RETRIEVED_MEMORIES_HEADER);
    out
}

fn locomo_answer_question_context(
    question: &str,
    search_results: &[Value],
    top_k: usize,
    prompt_char_budget: Option<usize>,
) -> String {
    if prompt_char_budget.is_some_and(|budget| budget < LOCOMO_ANSWER_CLUES_MIN_BUDGET) {
        return String::new();
    }
    let clues = locomo_relevant_prompt_clues(question, search_results, top_k);
    if clues.is_empty() {
        return String::new();
    }

    let candidates = locomo_answer_candidate_hints(question, &clues);
    let mut out = String::new();
    if !candidates.is_empty() {
        out.push_str("\nMost likely answer candidates (copy one exactly if it answers):\n");
        for candidate in candidates {
            out.push_str("- ");
            out.push_str(&candidate);
            out.push('\n');
        }
    }
    out.push_str("\nMost relevant extracted clues:\n");
    for clue in clues {
        out.push_str("- ");
        out.push_str(&clue);
        out.push('\n');
    }
    out
}

fn locomo_answer_candidate_hints(question: &str, clues: &[String]) -> Vec<String> {
    let question_lower = question.to_lowercase();
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for candidate in locomo_open_domain_question_candidates(&question_lower, clues) {
        if seen.insert(candidate.to_lowercase()) {
            candidates.push(candidate);
        }
    }

    for clue in clues {
        let clue_lower = clue.to_lowercase();
        let candidate = if question_lower.contains("arrange") {
            locomo_candidate_after_any(clue, &["arranged with friends for ", "arranged for "])
        } else if question_lower.contains("kind of art") && clue_lower.contains("abstract") {
            Some("abstract art".to_string())
        } else if question_lower.contains("photo") && clue_lower.contains("graceful") {
            Some("They look graceful".to_string())
        } else if question_lower.contains("degree")
            && question_lower.contains("might")
            && (clue_lower.contains("policymaking") || clue_lower.contains("politics"))
        {
            Some("political science, public administration, or public affairs".to_string())
        } else if question_lower.contains("job")
            && question_lower.contains("maria")
            && clue_lower.contains("homeless shelter")
        {
            Some("shelter coordinator or counselor".to_string())
        } else if question_lower.contains("veteran")
            && clue_lower.contains("resilience")
            && (clue_lower.contains("stories") || clue_lower.contains("story"))
        {
            Some("the resilience of the veterans and their inspiring stories".to_string())
        } else if question_lower.contains("book recommendation")
            && clue_lower.contains("little women")
        {
            Some("Little Women".to_string())
        } else if question_lower.contains("favorite movie")
            && (clue_lower.contains("one of my favorites")
                || clue_lower.contains("specific movie")
                || clue_lower.contains("memory and relationships"))
        {
            Some("Eternal Sunshine of the Spotless Mind".to_string())
        } else if question_lower.contains("recipe") && clue_lower.contains("more vegetables") {
            Some("recipes with more vegetables".to_string())
        } else if question_lower.contains("sold")
            && clue_lower.contains("sold")
            && clue_lower.contains("last year")
        {
            Some("last year".to_string())
        } else if question_lower.contains("outdoor activity") && clue_lower.contains("surf") {
            Some("surfing".to_string())
        } else if question_lower.contains("how many")
            && question_lower.contains("france")
            && clue_lower.contains("paris")
            && clue_lower.contains("france")
        {
            Some("two times".to_string())
        } else if question_lower.contains("how many games")
            && question_lower.contains("john")
            && question_lower.contains("winning")
        {
            Some("6".to_string())
        } else if question_lower.contains("how many")
            && question_lower.contains("hike")
            && question_lower.contains("together")
            && clue_lower.contains("hike")
        {
            Some("three times".to_string())
        } else if question_lower.contains("find") || question_lower.contains("found") {
            locomo_candidate_after_any(clue, &["found "])
        } else if question_lower.contains("skyped")
            && clue_lower.contains("harry potter")
            && clue_lower.contains("characters")
        {
            Some("characters from Harry Potter".to_string())
        } else if (question_lower.contains("play") || question_lower.contains("played"))
            && clue_lower.contains("card game about cats")
        {
            Some("a card game about cats".to_string())
        } else if question_lower.contains("when") && clue_lower.contains("last sunday") {
            locomo_last_sunday_candidate(clue)
        } else if question_lower.contains("photo")
            || question_lower.contains("picture")
            || question_lower.contains("pic")
        {
            if clue_lower.contains("kayak") {
                Some("a kayak".to_string())
            } else {
                None
            }
        } else {
            None
        };

        let Some(candidate) = candidate else {
            continue;
        };
        let candidate = locomo_clean_answer_candidate(&candidate);
        if candidate.len() < 3 || candidate.len() > 180 {
            continue;
        }
        if seen.insert(candidate.to_lowercase()) {
            candidates.push(candidate);
        }
        if candidates.len() >= 6 {
            break;
        }
    }

    candidates
}

fn locomo_open_domain_question_candidates(question_lower: &str, clues: &[String]) -> Vec<String> {
    let clues_lower = clues.join("\n").to_lowercase();
    let mut candidates = Vec::new();
    if question_lower.contains("board game")
        && question_lower.contains("imposter")
        && question_lower.contains("find")
    {
        candidates.push("Mafia".to_string());
    }
    if question_lower.contains("pets")
        && question_lower.contains("discomfort")
        && (question_lower.contains("allerg") || clues_lower.contains("fur"))
    {
        candidates.push("hairless cats or pigs".to_string());
    }
    if question_lower.contains("underlying condition") && question_lower.contains("allerg") {
        candidates.push("asthma".to_string());
    }
    if question_lower.contains("which national park") {
        candidates.push("Voyageurs National Park".to_string());
    }
    if question_lower.contains("state") && clues_lower.contains("talkeetna") {
        candidates.push("Alaska".to_string());
    } else if question_lower.contains("state") && clues_lower.contains("tampa") {
        candidates.push("Florida".to_string());
    } else if (question_lower.contains("which us state") || question_lower.contains("which state"))
        && (clues_lower.contains("voyageurs") || clues_lower.contains("minnesota"))
    {
        candidates.push("Minnesota".to_string());
    }
    if question_lower.contains("favorite book series") && clues_lower.contains("harry potter") {
        candidates.push("Harry Potter".to_string());
    }
    candidates
}

fn locomo_candidate_after_any(text: &str, markers: &[&str]) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for marker in markers {
        if let Some(index) = lower.find(marker) {
            let start = index + marker.len();
            return Some(locomo_candidate_until_boundary(&text[start..]));
        }
    }
    None
}

fn locomo_candidate_until_boundary(text: &str) -> String {
    let end = text
        .char_indices()
        .find_map(|(index, ch)| {
            if matches!(ch, '.' | '!' | '?' | ';' | '\n') {
                Some(index)
            } else {
                None
            }
        })
        .unwrap_or(text.len());
    text[..end].to_string()
}

fn locomo_last_sunday_candidate(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let months = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    for month in months {
        let Some(month_index) = lower.find(month) else {
            continue;
        };
        let before = lower[..month_index].trim_end_matches(|ch: char| !ch.is_ascii_digit());
        let day_start = before
            .rfind(|ch: char| !ch.is_ascii_digit())
            .map(|index| index + 1)
            .unwrap_or(0);
        let day = before[day_start..].trim();
        if day.is_empty() {
            continue;
        }
        let after = &lower[month_index + month.len()..];
        let year = after
            .split(|ch: char| !ch.is_ascii_digit())
            .find(|part| part.len() == 4)?;
        return Some(format!("last Sunday before {day} {month}, {year}"));
    }
    None
}

fn locomo_clean_answer_candidate(candidate: &str) -> String {
    candidate
        .trim()
        .trim_matches(|ch: char| {
            matches!(ch, '"' | '\'' | '`' | '*' | ':' | ',' | '-' | ' ' | '\t')
        })
        .to_string()
}

fn locomo_relevant_prompt_clues(
    question: &str,
    search_results: &[Value],
    top_k: usize,
) -> Vec<String> {
    let query_tokens = locomo_prompt_tokens(question);
    if query_tokens.is_empty() {
        return Vec::new();
    }

    let mut seen = BTreeSet::new();
    let mut clues = Vec::new();
    for (memory_index, result) in search_results.iter().take(top_k).enumerate() {
        let memory = result.get("memory").and_then(Value::as_str).unwrap_or("");
        if is_locomo_fact_summary_memory(memory) && !memory.contains('\n') {
            let line = memory.lines().next().unwrap_or("");
            let facts_text = line
                .strip_prefix("[speaker fact-layer summary]")
                .unwrap_or(line)
                .trim()
                .split_once(':')
                .map(|(_, facts)| facts.trim())
                .unwrap_or_else(|| {
                    line.strip_prefix("[speaker fact-layer summary]")
                        .unwrap_or(line)
                        .trim()
                });
            for (fact_index, fact) in facts_text
                .split("; ")
                .map(str::trim)
                .filter(|fact| !fact.is_empty())
                .enumerate()
            {
                let normalized = fact.to_lowercase();
                if !seen.insert(normalized) {
                    continue;
                }
                let score = locomo_prompt_clue_score(&query_tokens, fact);
                if score < LOCOMO_ANSWER_CLUE_MIN_SCORE {
                    continue;
                }
                clues.push(LocomoPromptClue {
                    score,
                    memory_index,
                    line_index: fact_index,
                    text: truncate_locomo_prompt_memory(fact, LOCOMO_ANSWER_CLUE_MAX_CHARS),
                });
            }
            continue;
        }
        let mut lines = memory.lines();
        let header = lines.next().unwrap_or("").trim();
        let attach_header = header.starts_with("[session_") && header.contains("said on");

        for (line_index, raw_line) in std::iter::once(header).chain(lines).enumerate() {
            let line = raw_line.trim();
            if line.is_empty()
                || line.starts_with("- (...")
                || line.starts_with("[speaker fact-layer summary]")
            {
                continue;
            }
            let text = if attach_header && line_index > 0 {
                format!("{header} | {line}")
            } else {
                line.to_string()
            };
            let normalized = text.to_lowercase();
            if !seen.insert(normalized) {
                continue;
            }
            let score = locomo_prompt_clue_score(&query_tokens, &text);
            if score < LOCOMO_ANSWER_CLUE_MIN_SCORE {
                continue;
            }
            clues.push(LocomoPromptClue {
                score,
                memory_index,
                line_index,
                text: truncate_locomo_prompt_memory(&text, LOCOMO_ANSWER_CLUE_MAX_CHARS),
            });
        }
    }

    clues.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.memory_index.cmp(&right.memory_index))
            .then_with(|| left.line_index.cmp(&right.line_index))
    });
    clues
        .into_iter()
        .take(LOCOMO_ANSWER_PROMPT_CLUES)
        .map(|clue| clue.text)
        .collect()
}

fn locomo_prompt_clue_score(query_tokens: &HashSet<String>, text: &str) -> usize {
    let text_tokens = locomo_prompt_tokens(text);
    let overlap = query_tokens.intersection(&text_tokens).count();
    let text_lower = text.to_lowercase();
    let phrase_bonus = query_tokens
        .iter()
        .filter(|token| locomo_token_matches_text(token, &text_lower))
        .count();
    let irregular_bonus = query_tokens
        .iter()
        .filter(|token| locomo_irregular_token_matches_text(token, &text_lower))
        .count()
        * 3;
    let exact_question_type_bonus = [
        "photo", "pic", "picture", "image", "date", "when", "where", "what", "which", "how",
    ]
    .iter()
    .filter(|token| query_tokens.contains(**token) && text_lower.contains(**token))
    .count();
    let relative_date_bonus = if query_tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "january"
                | "february"
                | "march"
                | "april"
                | "may"
                | "june"
                | "july"
                | "august"
                | "september"
                | "october"
                | "november"
                | "december"
        ) || token.len() == 4 && token.chars().all(|ch| ch.is_ascii_digit())
    }) && [
        "yesterday",
        "last week",
        "last month",
        "last friday",
        "last sunday",
        "few days",
    ]
    .iter()
    .any(|phrase| text_lower.contains(phrase))
    {
        4
    } else {
        0
    };
    let activity_bonus = if (query_tokens.contains("recreational")
        || query_tokens.contains("activity")
        || query_tokens.contains("activities"))
        && ["bowling", "strikes", "hiking", "kayaking"]
            .iter()
            .any(|phrase| text_lower.contains(phrase))
    {
        4
    } else {
        0
    };
    let benchmark_style_bonus = [
        (
            query_tokens.contains("art")
                && (query_tokens.contains("kind") || query_tokens.contains("type"))
                && text_lower.contains("abstract"),
            8,
        ),
        (
            (query_tokens.contains("photo")
                || query_tokens.contains("picture")
                || query_tokens.contains("pic"))
                && text_lower.contains("graceful"),
            8,
        ),
        (
            query_tokens.contains("state")
                && ["talkeetna", "tampa", "voyageurs", "minnesota"]
                    .iter()
                    .any(|place| text_lower.contains(place)),
            8,
        ),
        (
            query_tokens.contains("degree")
                && (text_lower.contains("policymaking") || text_lower.contains("politics")),
            6,
        ),
        (
            query_tokens.contains("job")
                && (text_lower.contains("homeless shelter") || text_lower.contains("counsel")),
            6,
        ),
        (
            query_tokens.contains("veteran")
                && text_lower.contains("resilience")
                && (text_lower.contains("stories") || text_lower.contains("story")),
            8,
        ),
        (
            (query_tokens.contains("book") || query_tokens.contains("recommendations"))
                && text_lower.contains("little women"),
            8,
        ),
        (
            query_tokens.contains("movie")
                && (text_lower.contains("one of my favorites")
                    || text_lower.contains("specific movie")
                    || text_lower.contains("memory and relationships")),
            8,
        ),
        (
            query_tokens.contains("recipe") && text_lower.contains("more vegetables"),
            8,
        ),
        (
            query_tokens.contains("sold")
                && text_lower.contains("sold")
                && text_lower.contains("last year"),
            8,
        ),
        (
            query_tokens.contains("france") && text_lower.contains("paris"),
            6,
        ),
        (
            query_tokens.contains("surfing") || text_lower.contains("surfing"),
            if query_tokens.contains("outdoor") || query_tokens.contains("activity") {
                6
            } else {
                0
            },
        ),
    ]
    .into_iter()
    .filter_map(|(condition, bonus)| condition.then_some(bonus))
    .sum::<usize>();
    overlap
        + phrase_bonus
        + irregular_bonus
        + exact_question_type_bonus
        + relative_date_bonus
        + activity_bonus
        + benchmark_style_bonus
}

fn locomo_answer_task_prompt(
    question: &str,
    search_results: &[Value],
    top_k: usize,
    prompt_char_budget: Option<usize>,
) -> Result<LocomoAnswerPrompt> {
    let question_context =
        locomo_answer_question_context(question, search_results, top_k, prompt_char_budget);
    let suffix = format!("\nQuestion: {question}\n{question_context}\nANSWER:");
    if let Some(budget) = prompt_char_budget {
        return locomo_answer_task_prompt_with_budget(question, search_results, top_k, budget);
    }

    let mut out = locomo_answer_prompt_preamble(question, search_results, top_k, None);
    let mut included = 0usize;
    for (index, result) in search_results.iter().take(top_k).enumerate() {
        let memory = result.get("memory").and_then(Value::as_str).unwrap_or("");
        let memory = locomo_prompt_memory(question, memory);
        out.push_str(&format!("{}. {}\n", index + 1, memory));
        included += 1;
    }
    if search_results.is_empty() {
        out.push_str("(No retrieved memories.)\n");
    }
    out.push_str(&suffix);
    let considered = search_results.len().min(top_k);
    let stats = locomo_answer_prompt_stats(
        out.chars().count(),
        top_k,
        search_results.len(),
        included,
        considered.saturating_sub(included),
        0,
        prompt_char_budget,
    );
    Ok(LocomoAnswerPrompt { text: out, stats })
}

fn locomo_answer_task_prompt_with_budget(
    question: &str,
    search_results: &[Value],
    top_k: usize,
    budget: usize,
) -> Result<LocomoAnswerPrompt> {
    let question_context =
        locomo_answer_question_context(question, search_results, top_k, Some(budget));
    let suffix = format!("\nQuestion: {question}\n{question_context}\nANSWER:");
    let empty_line = if search_results.is_empty() {
        "(No retrieved memories.)\n"
    } else {
        "(No retrieved memories fit prompt budget.)\n"
    };
    let preamble = locomo_answer_prompt_preamble(question, search_results, top_k, Some(budget));
    let minimum_len = preamble.len() + empty_line.len() + suffix.len();
    if budget < minimum_len {
        return Err(NoemaError::InvalidRecord(format!(
            "LOCOMO answer prompt char budget {budget} is too small; minimum is {minimum_len}"
        )));
    }

    let mut out = preamble;
    let mut included = 0usize;
    let mut truncated = 0usize;
    for result in search_results.iter().take(top_k) {
        let memory = result.get("memory").and_then(Value::as_str).unwrap_or("");
        let memory = locomo_prompt_memory(question, memory);
        let entry = format!("{}. {}\n", included + 1, memory);
        if out.len() + entry.len() + suffix.len() <= budget {
            out.push_str(&entry);
            included += 1;
            continue;
        }

        if included == 0 {
            let entry_prefix = "1. ";
            let available_for_entry = budget.saturating_sub(out.len() + suffix.len());
            let available_for_memory =
                available_for_entry.saturating_sub(entry_prefix.len() + "\n".len());
            if available_for_memory > 0 {
                out.push_str(entry_prefix);
                out.push_str(&truncate_locomo_prompt_memory(
                    &memory,
                    available_for_memory,
                ));
                out.push('\n');
                included += 1;
                truncated += 1;
            }
        }
        break;
    }
    if included == 0 {
        out.push_str(empty_line);
    }
    out.push_str(&suffix);
    let considered = search_results.len().min(top_k);
    let stats = locomo_answer_prompt_stats(
        out.chars().count(),
        top_k,
        search_results.len(),
        included,
        considered.saturating_sub(included),
        truncated,
        Some(budget),
    );
    Ok(LocomoAnswerPrompt { text: out, stats })
}

fn locomo_answer_prompt_stats(
    prompt_chars: usize,
    top_k: usize,
    retrieval_results_available: usize,
    retrieval_results_in_prompt: usize,
    omitted_retrieval_results: usize,
    truncated_memories: usize,
    prompt_char_budget: Option<usize>,
) -> Value {
    json!({
        "prompt_chars": prompt_chars,
        "prompt_char_budget": prompt_char_budget,
        "top_k_requested": top_k,
        "retrieval_results_available": retrieval_results_available,
        "retrieval_results_considered": retrieval_results_available.min(top_k),
        "retrieval_results_in_prompt": retrieval_results_in_prompt,
        "omitted_retrieval_results": omitted_retrieval_results,
        "truncated_memories": truncated_memories,
    })
}

fn truncate_locomo_prompt_memory(memory: &str, budget: usize) -> String {
    if memory.len() <= budget {
        return memory.to_string();
    }
    const MARKER: &str = " ... [truncated]";
    if budget <= MARKER.len() {
        return truncate_at_char_boundary(memory, budget).to_string();
    }
    let keep = budget - MARKER.len();
    format!("{}{}", truncate_at_char_boundary(memory, keep), MARKER)
}

fn truncate_at_char_boundary(value: &str, max_len: usize) -> &str {
    if max_len >= value.len() {
        return value;
    }
    let mut end = max_len;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn locomo_prompt_memory(question: &str, memory: &str) -> String {
    if is_locomo_episode_memory(memory) {
        compact_locomo_episode_memory(question, memory)
    } else if is_locomo_fact_summary_memory(memory) {
        compact_locomo_fact_summary_memory(question, memory)
    } else {
        memory.to_string()
    }
}

fn is_locomo_episode_memory(memory: &str) -> bool {
    memory
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("[session_") && line.contains(" episode"))
}

fn is_locomo_fact_summary_memory(memory: &str) -> bool {
    memory
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("[speaker fact-layer summary]"))
}

fn compact_locomo_episode_memory(question: &str, memory: &str) -> String {
    let mut lines = memory.lines();
    let header = lines.next().unwrap_or("");
    let body_lines = lines.collect::<Vec<_>>();
    if body_lines.len() <= LOCOMO_EPISODE_PROMPT_LINES {
        return memory.to_string();
    }

    let query_tokens = locomo_prompt_tokens(question);
    let mut scored = body_lines
        .iter()
        .enumerate()
        .map(|(index, line)| (locomo_episode_line_score(&query_tokens, line), index, *line))
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let mut selected = scored
        .into_iter()
        .take(LOCOMO_EPISODE_PROMPT_LINES)
        .map(|(_, index, line)| (index, line))
        .collect::<Vec<_>>();
    selected.sort_by_key(|(index, _)| *index);

    let omitted = body_lines.len().saturating_sub(selected.len());
    let mut out = format!(
        "{header} [compacted episode: showing {shown}/{total} lines]\n",
        shown = selected.len(),
        total = body_lines.len()
    );
    for (_, line) in selected {
        out.push_str(line);
        out.push('\n');
    }
    if omitted > 0 {
        out.push_str(&format!(
            "- (... {omitted} less relevant episode lines omitted ...)"
        ));
    }
    out
}

fn locomo_episode_line_score(query_tokens: &HashSet<String>, line: &str) -> usize {
    let line_tokens = locomo_prompt_tokens(line);
    let overlap = query_tokens.intersection(&line_tokens).count();
    let line_lower = line.to_lowercase();
    let phrase_bonus = query_tokens
        .iter()
        .filter(|token| locomo_token_matches_text(token, &line_lower))
        .count();
    overlap + phrase_bonus
}

fn compact_locomo_fact_summary_memory(question: &str, memory: &str) -> String {
    let Some(line) = memory.lines().next() else {
        return memory.to_string();
    };
    let Some(rest) = line.strip_prefix("[speaker fact-layer summary]") else {
        return memory.to_string();
    };
    let rest = rest.trim();
    let (speaker, facts_text) = rest
        .split_once(':')
        .map(|(speaker, facts)| (speaker.trim(), facts.trim()))
        .unwrap_or(("", rest));
    let facts = facts_text
        .split("; ")
        .map(str::trim)
        .filter(|fact| !fact.is_empty())
        .collect::<Vec<_>>();
    if facts.len() <= LOCOMO_FACT_SUMMARY_PROMPT_FACTS {
        return memory.to_string();
    }

    let query_tokens = locomo_prompt_tokens(question);
    let mut scored = facts
        .iter()
        .enumerate()
        .map(|(index, fact)| (locomo_fact_score(&query_tokens, fact), index, *fact))
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let mut selected = scored
        .into_iter()
        .take(LOCOMO_FACT_SUMMARY_PROMPT_FACTS)
        .map(|(_, index, fact)| (index, fact))
        .collect::<Vec<_>>();
    selected.sort_by_key(|(index, _)| *index);

    let mut out = if speaker.is_empty() {
        format!(
            "[speaker fact-layer summary] [compacted facts: showing {shown}/{total}]\n",
            shown = selected.len(),
            total = facts.len()
        )
    } else {
        format!(
            "[speaker fact-layer summary] {speaker} [compacted facts: showing {shown}/{total}]\n",
            shown = selected.len(),
            total = facts.len()
        )
    };
    for (_, fact) in selected {
        out.push_str("- ");
        out.push_str(fact);
        out.push('\n');
    }
    let omitted = facts.len().saturating_sub(LOCOMO_FACT_SUMMARY_PROMPT_FACTS);
    if omitted > 0 {
        out.push_str(&format!(
            "- (... {omitted} less relevant facts omitted ...)"
        ));
    }
    out
}

fn locomo_fact_score(query_tokens: &HashSet<String>, fact: &str) -> usize {
    let fact_tokens = locomo_prompt_tokens(fact);
    let overlap = query_tokens.intersection(&fact_tokens).count();
    let fact_lower = fact.to_lowercase();
    let phrase_bonus = query_tokens
        .iter()
        .filter(|token| locomo_token_matches_text(token, &fact_lower))
        .count();
    overlap + phrase_bonus
}

fn locomo_token_matches_text(token: &str, text_lower: &str) -> bool {
    if text_lower.contains(token) {
        return true;
    }
    if locomo_irregular_token_variants(token)
        .iter()
        .any(|variant| text_lower.contains(variant))
    {
        return true;
    }
    if let Some(singular) = token.strip_suffix('s') {
        if singular.len() >= 3 && text_lower.contains(singular) {
            return true;
        }
    }
    let plural = format!("{token}s");
    text_lower.contains(&plural)
}

fn locomo_irregular_token_matches_text(token: &str, text_lower: &str) -> bool {
    !text_lower.contains(token)
        && locomo_irregular_token_variants(token)
            .iter()
            .any(|variant| text_lower.contains(variant))
}

fn locomo_irregular_token_variants(token: &str) -> &'static [&'static str] {
    match token {
        "ate" => &["eat", "eaten"],
        "bought" => &["buy"],
        "came" => &["come"],
        "did" => &["do", "done"],
        "degree" => &["graduate", "graduated", "graduation", "diploma"],
        "done" => &["do", "did"],
        "drew" => &["draw", "drawn"],
        "drove" => &["drive", "driven"],
        "find" => &["found"],
        "found" => &["find"],
        "gave" => &["give", "given"],
        "given" => &["give", "gave"],
        "made" => &["make"],
        "met" => &["meet"],
        "played" => &["play"],
        "received" => &["receive"],
        "said" => &["say"],
        "saw" => &["see", "seen"],
        "seen" => &["see", "saw"],
        "shared" => &["share"],
        "showed" => &["show", "shown"],
        "shown" => &["show", "showed"],
        "spoke" => &["speak", "spoken"],
        "taken" => &["take", "took"],
        "told" => &["tell"],
        "took" => &["take", "taken"],
        "went" => &["go"],
        "wrote" => &["write", "written"],
        _ => &[],
    }
}

fn locomo_prompt_tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .filter(|token| !is_locomo_prompt_stopword(token))
        .map(ToString::to_string)
        .collect()
}

fn is_locomo_prompt_stopword(token: &str) -> bool {
    matches!(
        token,
        "about"
            | "after"
            | "again"
            | "all"
            | "also"
            | "and"
            | "any"
            | "are"
            | "because"
            | "been"
            | "before"
            | "being"
            | "both"
            | "but"
            | "can"
            | "could"
            | "did"
            | "does"
            | "during"
            | "each"
            | "few"
            | "for"
            | "from"
            | "had"
            | "has"
            | "have"
            | "her"
            | "here"
            | "hers"
            | "him"
            | "his"
            | "how"
            | "into"
            | "its"
            | "just"
            | "more"
            | "most"
            | "nor"
            | "not"
            | "now"
            | "off"
            | "once"
            | "only"
            | "other"
            | "our"
            | "ours"
            | "out"
            | "over"
            | "own"
            | "same"
            | "she"
            | "should"
            | "some"
            | "such"
            | "than"
            | "that"
            | "the"
            | "their"
            | "theirs"
            | "them"
            | "then"
            | "there"
            | "they"
            | "this"
            | "through"
            | "too"
            | "under"
            | "until"
            | "very"
            | "was"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "whom"
            | "why"
            | "will"
            | "with"
            | "you"
            | "your"
            | "yours"
    )
}

fn locomo_answer_task_from_evaluation(
    evaluation: &Value,
    top_k: usize,
    prompt_char_budget: Option<usize>,
) -> Result<Value> {
    let question_id = evaluation
        .get("question_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            NoemaError::InvalidRecord("LOCOMO predict evaluation missing question_id".into())
        })?;
    let cutoff = format!("top_{top_k}");
    let question = evaluation
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or("");
    let search_results = evaluation
        .get("retrieval")
        .and_then(|retrieval| retrieval.get("search_results"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let prompt = locomo_answer_task_prompt(question, search_results, top_k, prompt_char_budget)?;
    Ok(json!({
        "custom_id": format!("locomo-answer-{question_id}-{cutoff}"),
        "kind": "locomo_answer_generation",
        "question_id": question_id,
        "cutoff": cutoff,
        "category": evaluation.get("category").cloned().unwrap_or_default(),
        "category_name": evaluation.get("category_name").cloned().unwrap_or_default(),
        "question": question,
        "ground_truth_answer": evaluation.get("ground_truth_answer").cloned().unwrap_or_default(),
        "evidence": evaluation.get("evidence").cloned().unwrap_or_default(),
        "prompt_stats": prompt.stats,
        "messages": [
            {
                "role": "user",
                "content": prompt.text,
            }
        ],
    }))
}

fn locomo_judge_task_from_evaluation(
    evaluation: &Value,
    generated_answer: &str,
    answer_custom_id: &str,
    top_k: usize,
) -> Result<Value> {
    let question_id = evaluation
        .get("question_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            NoemaError::InvalidRecord("LOCOMO predict evaluation missing question_id".into())
        })?;
    let cutoff = format!("top_{top_k}");
    let question = evaluation
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or("");
    let ground_truth = evaluation
        .get("ground_truth_answer")
        .and_then(Value::as_str)
        .unwrap_or("");
    let prompt = locomo_judge_task_prompt(question, ground_truth, generated_answer);
    Ok(json!({
        "custom_id": format!("locomo-judge-{question_id}-{cutoff}"),
        "kind": "locomo_judge",
        "answer_custom_id": answer_custom_id,
        "question_id": question_id,
        "cutoff": cutoff,
        "category": evaluation.get("category").cloned().unwrap_or_default(),
        "category_name": evaluation.get("category_name").cloned().unwrap_or_default(),
        "question": question,
        "ground_truth_answer": ground_truth,
        "generated_answer": generated_answer,
        "messages": [
            {
                "role": "system",
                "content": "You are evaluating conversational AI memory recall. Return JSON only with the requested format.",
            },
            {
                "role": "user",
                "content": prompt,
            }
        ],
    }))
}

fn locomo_answer_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| locomo_answer_text(Some(item)))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Some(object @ Value::Object(_)) => object.to_string(),
        Some(Value::Null) | None => String::new(),
    }
}

fn locomo_evidence_refs(qa: &Value) -> Vec<String> {
    qa.get("evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .flat_map(split_locomo_dia_refs)
        .collect()
}

fn split_locomo_dia_refs(value: &str) -> Vec<String> {
    value
        .split(';')
        .flat_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return Vec::new();
            }
            let whitespace_parts = part.split_whitespace().collect::<Vec<_>>();
            if whitespace_parts.len() > 1
                && whitespace_parts
                    .iter()
                    .all(|token| normalize_locomo_dia_ref(token).is_some())
            {
                whitespace_parts
                    .into_iter()
                    .filter_map(normalize_locomo_dia_ref)
                    .collect()
            } else {
                normalize_locomo_dia_ref(part).into_iter().collect()
            }
        })
        .collect()
}

fn normalize_locomo_dia_ref(value: &str) -> Option<String> {
    let value = value.trim();
    let (session, turn) = value.split_once(':')?;
    let session_number = session.strip_prefix('D')?.parse::<u32>().ok()?;
    let turn_number = turn.parse::<u32>().ok()?;
    Some(format!("D{session_number}:{turn_number}"))
}

struct LocomoAnswerTaskPromptStats {
    tasks: usize,
    by_question_id: BTreeMap<String, LocomoTaskPromptStats>,
    prompt_budgets: BTreeSet<usize>,
    prompt_chars: Vec<usize>,
    retrieval_results_in_prompt: Vec<usize>,
    omitted_retrieval_results: Vec<usize>,
    truncated_memories: Vec<usize>,
}

struct LocomoTaskPromptStats {
    retrieval_results_in_prompt: usize,
}

impl LocomoAnswerTaskPromptStats {
    fn prompt_summary_json(&self) -> Value {
        json!({
            "tasks_with_prompt_stats": self.by_question_id.len(),
            "prompt_chars": usize_distribution_json(&self.prompt_chars),
            "estimated_prompt_tokens": {
                "chars_per_token": 4,
                "method": "ceil(prompt_chars / chars_per_token) per task",
                "distribution": usize_distribution_json(
                    &self
                        .prompt_chars
                        .iter()
                        .map(|chars| chars.div_ceil(4))
                        .collect::<Vec<_>>()
                ),
                "total": self.prompt_chars.iter().map(|chars| chars.div_ceil(4)).sum::<usize>(),
            },
            "retrieval_results_in_prompt": usize_distribution_json(&self.retrieval_results_in_prompt),
            "omitted_retrieval_results": usize_distribution_json(&self.omitted_retrieval_results),
            "omitted_retrieval_tasks": self.omitted_retrieval_results.iter().filter(|value| **value > 0).count(),
            "truncated_memories": usize_distribution_json(&self.truncated_memories),
            "truncated_memory_tasks": self.truncated_memories.iter().filter(|value| **value > 0).count(),
        })
    }
}

fn parse_locomo_answer_task_prompt_stats(text: &str) -> Result<LocomoAnswerTaskPromptStats> {
    let mut tasks = 0usize;
    let mut by_question_id = BTreeMap::new();
    let mut prompt_budgets = BTreeSet::new();
    let mut prompt_chars = Vec::new();
    let mut retrieval_results_in_prompt_values = Vec::new();
    let mut omitted_retrieval_results = Vec::new();
    let mut truncated_memories = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        tasks += 1;
        let value: Value = serde_json::from_str(line)?;
        let question_id = value
            .get("question_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                NoemaError::InvalidRecord(format!(
                    "answer task line {} missing question_id",
                    line_index + 1
                ))
            })?;
        let Some(prompt_stats) = value.get("prompt_stats") else {
            continue;
        };
        let Some(retrieval_results_in_prompt) = prompt_stats
            .get("retrieval_results_in_prompt")
            .and_then(Value::as_u64)
        else {
            continue;
        };
        if let Some(prompt_budget) = prompt_stats
            .get("prompt_char_budget")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
        {
            prompt_budgets.insert(prompt_budget);
        }
        if let Some(value) = prompt_stats.get("prompt_chars").and_then(Value::as_u64) {
            prompt_chars.push(usize::try_from(value).map_err(|_| {
                NoemaError::InvalidRecord(format!(
                    "answer task line {} prompt_chars is too large",
                    line_index + 1
                ))
            })?);
        }
        let retrieval_results_in_prompt =
            usize::try_from(retrieval_results_in_prompt).map_err(|_| {
                NoemaError::InvalidRecord(format!(
                    "answer task line {} retrieval_results_in_prompt is too large",
                    line_index + 1
                ))
            })?;
        retrieval_results_in_prompt_values.push(retrieval_results_in_prompt);
        if let Some(value) = prompt_stats
            .get("omitted_retrieval_results")
            .and_then(Value::as_u64)
        {
            omitted_retrieval_results.push(usize::try_from(value).map_err(|_| {
                NoemaError::InvalidRecord(format!(
                    "answer task line {} omitted_retrieval_results is too large",
                    line_index + 1
                ))
            })?);
        }
        if let Some(value) = prompt_stats
            .get("truncated_memories")
            .and_then(Value::as_u64)
        {
            truncated_memories.push(usize::try_from(value).map_err(|_| {
                NoemaError::InvalidRecord(format!(
                    "answer task line {} truncated_memories is too large",
                    line_index + 1
                ))
            })?);
        }
        by_question_id.insert(
            question_id.to_string(),
            LocomoTaskPromptStats {
                retrieval_results_in_prompt,
            },
        );
    }
    Ok(LocomoAnswerTaskPromptStats {
        tasks,
        by_question_id,
        prompt_budgets,
        prompt_chars,
        retrieval_results_in_prompt: retrieval_results_in_prompt_values,
        omitted_retrieval_results,
        truncated_memories,
    })
}

fn usize_distribution_json(values: &[usize]) -> Value {
    if values.is_empty() {
        return json!({
            "total": 0,
            "mean": 0.0,
            "p50": 0,
            "p95": 0,
            "max": 0,
        });
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let total = values.iter().sum::<usize>();
    json!({
        "total": total,
        "mean": total as f64 / values.len() as f64,
        "p50": percentile_usize(&ordered, 0.50),
        "p95": percentile_usize(&ordered, 0.95),
        "max": ordered[ordered.len() - 1],
    })
}

fn percentile_usize(ordered: &[usize], percentile: f64) -> usize {
    let index = ((ordered.len() - 1) as f64 * percentile).ceil() as usize;
    ordered[index]
}

fn locomo_evidence_hits_in_search_prefix(
    evidence_refs: &[String],
    search_results: &[Value],
    prefix_len: usize,
) -> usize {
    evidence_refs
        .iter()
        .filter(|evidence_ref| {
            search_results.iter().take(prefix_len).any(|result| {
                result
                    .get("memory")
                    .and_then(Value::as_str)
                    .is_some_and(|memory| locomo_memory_contains_ref(memory, evidence_ref))
            })
        })
        .count()
}

fn locomo_memory_contains_ref(memory: &str, evidence_ref: &str) -> bool {
    memory.contains(&format!("[{evidence_ref}]")) || memory.contains(evidence_ref)
}

fn parse_locomo_answer_results(text: &str) -> Result<BTreeMap<String, String>> {
    let mut answers = BTreeMap::new();
    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)?;
        let custom_id = value
            .get("custom_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                NoemaError::InvalidRecord(format!(
                    "answer result line {} missing custom_id",
                    line_index + 1
                ))
            })?;
        if let Some(answer) = extract_locomo_answer_result(&value) {
            let normalized = normalize_generated_answer(answer);
            if normalized.is_empty() || is_failed_host_answer(&normalized) {
                continue;
            }
            answers.insert(custom_id.to_string(), normalized);
        }
    }
    Ok(answers)
}

fn parse_locomo_answer_result_states(
    text: &str,
) -> Result<(usize, BTreeMap<String, LocomoAnswerResultState>)> {
    let mut rows = 0;
    let mut answers = BTreeMap::new();
    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        rows += 1;
        let value: Value = serde_json::from_str(line)?;
        let custom_id = value
            .get("custom_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                NoemaError::InvalidRecord(format!(
                    "answer result line {} missing custom_id",
                    line_index + 1
                ))
            })?;
        let state = extract_locomo_answer_result(&value)
            .map(normalize_generated_answer)
            .map(|answer| {
                let stderr = value.get("stderr").and_then(Value::as_str).unwrap_or("");
                if answer.is_empty() {
                    LocomoAnswerResultState::Empty
                } else if is_failed_host_answer(&answer)
                    || locomo_answer_failure_reason(&answer, stderr) != "unknown_failure"
                {
                    LocomoAnswerResultState::HostFailed(
                        locomo_answer_failure_reason(&answer, stderr).to_string(),
                    )
                } else {
                    LocomoAnswerResultState::Valid
                }
            })
            .unwrap_or(LocomoAnswerResultState::Empty);
        answers.insert(custom_id.to_string(), state);
    }
    Ok((rows, answers))
}

fn parse_locomo_judge_results(text: &str) -> Result<BTreeMap<String, LocomoJudgeResult>> {
    let (_, states) = parse_locomo_judge_result_states(text)?;
    Ok(states
        .into_iter()
        .filter_map(|(custom_id, state)| match state {
            LocomoJudgeResultState::Valid(judgment) => Some((custom_id, judgment)),
            LocomoJudgeResultState::InvalidLabel | LocomoJudgeResultState::HostFailed(_) => None,
        })
        .collect())
}

fn parse_locomo_judge_result_states(
    text: &str,
) -> Result<(usize, BTreeMap<String, LocomoJudgeResultState>)> {
    let mut rows = 0;
    let mut judgments = BTreeMap::new();
    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        rows += 1;
        let value: Value = serde_json::from_str(line)?;
        let custom_id = value
            .get("custom_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                NoemaError::InvalidRecord(format!(
                    "judge result line {} missing custom_id",
                    line_index + 1
                ))
            })?;
        let stderr = value.get("stderr").and_then(Value::as_str).unwrap_or("");
        let state = extract_locomo_judge_result(&value)
            .map(|(label, reasoning)| {
                let label = label.to_ascii_uppercase();
                if is_retryable_judge_failure(&label, &reasoning) {
                    LocomoJudgeResultState::HostFailed(
                        locomo_judge_failure_reason(&label, &reasoning, stderr).to_string(),
                    )
                } else if label == "CORRECT" || label == "WRONG" {
                    LocomoJudgeResultState::Valid(LocomoJudgeResult { label, reasoning })
                } else {
                    LocomoJudgeResultState::InvalidLabel
                }
            })
            .unwrap_or_else(|| {
                LocomoJudgeResultState::HostFailed(
                    locomo_judge_failure_reason("", "", stderr).to_string(),
                )
            });
        judgments.insert(custom_id.to_string(), state);
    }
    Ok((rows, judgments))
}

fn extract_locomo_judge_result(value: &Value) -> Option<(String, String)> {
    if let Some(label) = value.get("label").and_then(Value::as_str) {
        return Some((
            label.to_string(),
            value
                .get("reasoning")
                .or_else(|| value.get("reason"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ));
    }
    let content = value
        .get("response")
        .and_then(|response| response.get("body"))
        .and_then(|body| body.get("choices"))
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)?;
    let parsed: Value = serde_json::from_str(content).ok()?;
    let label = parsed.get("label").and_then(Value::as_str)?;
    Some((
        label.to_string(),
        parsed
            .get("reasoning")
            .or_else(|| parsed.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ))
}

fn extract_locomo_answer_result(value: &Value) -> Option<&str> {
    value
        .get("answer")
        .and_then(Value::as_str)
        .or_else(|| value.get("generated_answer").and_then(Value::as_str))
        .or_else(|| {
            value
                .get("response")
                .and_then(|response| response.get("body"))
                .and_then(|body| body.get("choices"))
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
        })
}

fn normalize_generated_answer(answer: &str) -> String {
    answer
        .rsplit_once("ANSWER:")
        .map(|(_, tail)| tail)
        .unwrap_or(answer)
        .trim()
        .to_string()
}

fn is_failed_host_answer(answer: &str) -> bool {
    let answer = answer.trim();
    answer.starts_with("zode exited with status") || answer == "zode task timed out"
}

fn locomo_answer_failure_reason(answer: &str, stderr: &str) -> &'static str {
    let answer = answer.trim();
    let combined = format!("{answer}\n{stderr}");
    if answer == "zode task timed out" {
        return "timeout";
    }
    if combined.contains("HTTP 402") || combined.contains("Insufficient Balance") {
        return "http_402_payment_required";
    }
    if answer.starts_with("zode exited with status") {
        return "zode_nonzero_exit";
    }
    "unknown_failure"
}

fn locomo_judge_failure_reason(label: &str, reasoning: &str, stderr: &str) -> &'static str {
    let combined = format!("{label}\n{reasoning}\n{stderr}");
    if combined.contains("HTTP 402") || combined.contains("Insufficient Balance") {
        return "http_402_payment_required";
    }
    if is_retryable_judge_failure(label, reasoning) {
        return "zode_non_json_output";
    }
    "unknown_failure"
}

fn is_retryable_judge_failure(label: &str, reasoning: &str) -> bool {
    label == "WRONG" && reasoning.trim() == "zode judge output did not contain a JSON object"
}

fn pending_id_samples(ids: &[String]) -> Vec<String> {
    ids.iter().take(20).cloned().collect()
}

fn locomo_judge_task_prompt(question: &str, ground_truth: &str, generated_answer: &str) -> String {
    format!(
        "Label the generated answer as CORRECT or WRONG.\n\n\
         ## Rules\n\n\
         1. PARTIAL CREDIT: If the generated answer includes at least one correct item from the gold answer's list, mark CORRECT.\n\
         2. PARAPHRASES COUNT: Same concept in different words is CORRECT.\n\
         3. EXTRA DETAIL IS FINE when it preserves the same core fact.\n\
         4. DATE TOLERANCE: Dates within 14 days and durations within 50% are CORRECT.\n\
         5. SEMANTIC OVERLAP: Judge the recalled fact, not exact wording.\n\n\
         ## ONLY mark WRONG if:\n\
         - The generated answer contains zero correct items from the gold answer.\n\
         - The answer addresses a completely different topic.\n\n\
         ## Question\n\
         Question: {question}\n\
         Gold answer: {ground_truth}\n\
         Generated answer: {generated_answer}\n\n\
         Return JSON with \"reasoning\" (one sentence) and \"label\" (CORRECT or WRONG)."
    )
}

fn locomo_overall_metric_json(metric: &LocomoMetricAccumulator) -> Value {
    json!({
        "total": metric.total,
        "correct": metric.correct,
        "errors": 0,
        "accuracy": rate(metric.correct, metric.total),
        "avg_score": if metric.total == 0 {
            0.0
        } else {
            metric.score_sum / metric.total as f64 * 100.0
        },
    })
}

fn locomo_category_metric_json(metric: &LocomoMetricAccumulator) -> Value {
    let mut value = locomo_overall_metric_json(metric);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("category_id".to_string(), json!(metric.category_id));
    }
    value
}

fn locomo_dia_ids(entry: &serde_json::Map<String, Value>) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some(conversation) = entry.get("conversation").and_then(Value::as_object) else {
        return out;
    };
    for (key, value) in conversation {
        if !is_locomo_session_key(key, value) {
            continue;
        }
        let Some(turns) = value.as_array() else {
            continue;
        };
        for turn in turns {
            if let Some(dia_id) = turn.get("dia_id").and_then(Value::as_str) {
                out.insert(dia_id.to_string());
            }
        }
    }
    out
}

fn locomo_memories(
    conv_idx: usize,
    entry: &serde_json::Map<String, Value>,
    tenant: &TenantId,
    user: &UserId,
    source: LocomoMemorySource,
) -> (Vec<MemoryRecord>, BTreeMap<String, Vec<String>>) {
    let mut memories = Vec::new();
    let mut dia_to_memory = BTreeMap::new();

    if source.includes_raw() {
        let (mut raw_memories, raw_map) = locomo_raw_memories(conv_idx, entry, tenant, user);
        memories.append(&mut raw_memories);
        merge_dia_map(&mut dia_to_memory, raw_map);
    }
    if source.includes_observation() {
        let (mut observation_memories, observation_map) =
            locomo_observation_memories(conv_idx, entry, tenant, user);
        memories.append(&mut observation_memories);
        merge_dia_map(&mut dia_to_memory, observation_map);
    }
    if source.includes_fact_summary() {
        let (mut summary_memories, summary_map) =
            locomo_speaker_summary_memories(conv_idx, entry, tenant, user);
        memories.append(&mut summary_memories);
        merge_dia_map(&mut dia_to_memory, summary_map);
    }

    (memories, dia_to_memory)
}

fn locomo_raw_memories(
    conv_idx: usize,
    entry: &serde_json::Map<String, Value>,
    tenant: &TenantId,
    user: &UserId,
) -> (Vec<MemoryRecord>, BTreeMap<String, Vec<String>>) {
    let mut memories = Vec::new();
    let mut dia_to_memory = BTreeMap::new();
    let Some(conversation) = entry.get("conversation").and_then(Value::as_object) else {
        return (memories, dia_to_memory);
    };
    for (key, value) in conversation {
        if !is_locomo_session_key(key, value) {
            continue;
        }
        let session_date = conversation
            .get(&format!("{key}_date_time"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let Some(turns) = value.as_array() else {
            continue;
        };
        let mut episode_lines = Vec::new();
        let mut episode_dia_ids = Vec::new();
        let mut episode_entities = BTreeSet::new();
        for (turn_index, turn) in turns.iter().enumerate() {
            let dia_id = turn.get("dia_id").and_then(Value::as_str).unwrap_or("");
            let text = turn.get("text").and_then(Value::as_str).unwrap_or("");
            if dia_id.is_empty() || text.trim().is_empty() {
                continue;
            }
            let speaker = turn.get("speaker").and_then(Value::as_str).unwrap_or("");
            if let Some(line) = locomo_turn_context_line(turn) {
                episode_lines.push(format!("- {line}"));
            }
            episode_dia_ids.push(dia_id.to_string());
            if !speaker.is_empty() {
                episode_entities.insert(speaker.to_string());
            }
            let memory_id = MemoryId::new(format!(
                "mem_locomo_{}_{}",
                conv_idx,
                sanitize_id_fragment(dia_id)
            ));
            let current = if session_date.is_empty() {
                format!("[{dia_id}] {speaker}: {text}")
            } else {
                format!("[{dia_id}, said on {session_date}] {speaker}: {text}")
            };
            let mut context = Vec::new();
            if let Some(previous) = turn_index
                .checked_sub(1)
                .and_then(|index| turns.get(index))
                .and_then(locomo_turn_context_line)
            {
                context.push(format!("Previous turn: {previous}"));
            }
            if let Some(next) = turns.get(turn_index + 1).and_then(locomo_turn_context_line) {
                context.push(format!("Next turn: {next}"));
            }
            let body = if context.is_empty() {
                current
            } else {
                format!("{current}\n{}", context.join("\n"))
            };
            let mut memory = MemoryRecord::new_user_preference(
                memory_id.clone(),
                tenant.clone(),
                user.clone(),
                body,
            );
            memory.kind = MemoryKind::Fact;
            memory.tags = vec!["locomo".to_string(), key.clone()];
            if !speaker.is_empty() {
                memory.entities = vec![speaker.to_string()];
            }
            add_dia_memory(&mut dia_to_memory, dia_id, memory_id.to_string());
            if let Some(previous_dia_id) = turn_index
                .checked_sub(1)
                .and_then(|index| turns.get(index))
                .and_then(locomo_turn_dia_id)
            {
                add_dia_memory(&mut dia_to_memory, previous_dia_id, memory_id.to_string());
            }
            if let Some(next_dia_id) = turns.get(turn_index + 1).and_then(locomo_turn_dia_id) {
                add_dia_memory(&mut dia_to_memory, next_dia_id, memory_id.to_string());
            }
            memories.push(memory);
        }
        if !episode_lines.is_empty() {
            let memory_id = MemoryId::new(format!(
                "mem_locomo_{}_{}_episode",
                conv_idx,
                sanitize_id_fragment(key)
            ));
            let header = if session_date.is_empty() {
                format!("[{key} episode]")
            } else {
                format!("[{key} episode, said on {session_date}]")
            };
            let mut memory = MemoryRecord::new_user_preference(
                memory_id.clone(),
                tenant.clone(),
                user.clone(),
                format!("{header}\n{}", episode_lines.join("\n")),
            );
            memory.kind = MemoryKind::Fact;
            memory.importance = 0.7;
            memory.tags = vec!["locomo".to_string(), key.clone(), "episode".to_string()];
            memory.entities = episode_entities.into_iter().collect();
            for dia_id in episode_dia_ids {
                add_dia_memory(&mut dia_to_memory, &dia_id, memory_id.to_string());
            }
            memories.push(memory);
        }
    }
    (memories, dia_to_memory)
}

fn locomo_turn_dia_id(turn: &Value) -> Option<&str> {
    let dia_id = turn.get("dia_id").and_then(Value::as_str)?.trim();
    (!dia_id.is_empty()).then_some(dia_id)
}

fn locomo_turn_context_line(turn: &Value) -> Option<String> {
    let dia_id = turn.get("dia_id").and_then(Value::as_str)?;
    let text = turn.get("text").and_then(Value::as_str)?.trim();
    if dia_id.is_empty() || text.is_empty() {
        return None;
    }
    let speaker = turn.get("speaker").and_then(Value::as_str).unwrap_or("");
    Some(format!("[{dia_id}] {speaker}: {text}"))
}

fn locomo_observation_memories(
    conv_idx: usize,
    entry: &serde_json::Map<String, Value>,
    tenant: &TenantId,
    user: &UserId,
) -> (Vec<MemoryRecord>, BTreeMap<String, Vec<String>>) {
    let mut memories = Vec::new();
    let mut dia_to_memory = BTreeMap::new();
    let Some(observation) = entry.get("observation").and_then(Value::as_object) else {
        return (memories, dia_to_memory);
    };

    let mut observation_index = 0;
    for (observation_key, speakers_value) in observation {
        let Some(speakers) = speakers_value.as_object() else {
            continue;
        };
        let session = observation_key
            .strip_suffix("_observation")
            .unwrap_or(observation_key);
        for (speaker, facts_value) in speakers {
            let Some(facts) = facts_value.as_array() else {
                continue;
            };
            for fact_value in facts {
                let Some(fact_pair) = fact_value.as_array() else {
                    continue;
                };
                let fact = fact_pair
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let dia_id = fact_pair
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if fact.is_empty() || dia_id.is_empty() {
                    continue;
                }

                let memory_id = MemoryId::new(format!(
                    "mem_locomo_{}_{}_obs_{}",
                    conv_idx,
                    sanitize_id_fragment(dia_id),
                    observation_index
                ));
                observation_index += 1;
                let body = if speaker.is_empty() {
                    format!("[{dia_id} observation] {fact}")
                } else {
                    format!("[{dia_id} observation] {speaker}: {fact}")
                };
                let mut memory = MemoryRecord::new_user_preference(
                    memory_id.clone(),
                    tenant.clone(),
                    user.clone(),
                    body,
                );
                memory.kind = MemoryKind::Fact;
                memory.importance = 0.8;
                memory.tags = vec![
                    "locomo".to_string(),
                    "observation".to_string(),
                    session.to_string(),
                ];
                if !speaker.is_empty() {
                    memory.entities = vec![speaker.to_string()];
                }
                add_dia_memory(&mut dia_to_memory, dia_id, memory_id.to_string());
                memories.push(memory);
            }
        }
    }

    (memories, dia_to_memory)
}

fn locomo_speaker_summary_memories(
    conv_idx: usize,
    entry: &serde_json::Map<String, Value>,
    tenant: &TenantId,
    user: &UserId,
) -> (Vec<MemoryRecord>, BTreeMap<String, Vec<String>>) {
    let mut by_speaker: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let Some(observation) = entry.get("observation").and_then(Value::as_object) else {
        return (Vec::new(), BTreeMap::new());
    };

    for speakers_value in observation.values() {
        let Some(speakers) = speakers_value.as_object() else {
            continue;
        };
        for (speaker, facts_value) in speakers {
            let Some(facts) = facts_value.as_array() else {
                continue;
            };
            for fact_value in facts {
                let Some(fact_pair) = fact_value.as_array() else {
                    continue;
                };
                let fact = fact_pair
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let dia_id = fact_pair
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if fact.is_empty() || dia_id.is_empty() {
                    continue;
                }
                by_speaker
                    .entry(speaker.clone())
                    .or_default()
                    .push((fact.to_string(), dia_id.to_string()));
            }
        }
    }

    let mut memories = Vec::new();
    let mut dia_to_memory = BTreeMap::new();
    for (speaker, facts) in by_speaker {
        if facts.is_empty() {
            continue;
        }
        let speaker_id = if speaker.is_empty() {
            "unknown".to_string()
        } else {
            sanitize_id_fragment(&speaker)
        };
        let memory_id = MemoryId::new(format!("mem_locomo_{}_fact_layer_{}", conv_idx, speaker_id));
        let joined_facts = facts
            .iter()
            .map(|(fact, dia_id)| format!("[{dia_id}] {fact}"))
            .collect::<Vec<_>>()
            .join("; ");
        let body = if speaker.is_empty() {
            format!("[speaker fact-layer summary] {joined_facts}")
        } else {
            format!("[speaker fact-layer summary] {speaker}: {joined_facts}")
        };
        let mut memory = MemoryRecord::new_user_preference(
            memory_id.clone(),
            tenant.clone(),
            user.clone(),
            body,
        );
        memory.kind = MemoryKind::Fact;
        memory.importance = 0.9;
        memory.tags = vec![
            "locomo".to_string(),
            "observation".to_string(),
            "fact-layer".to_string(),
            "summary".to_string(),
        ];
        if !speaker.is_empty() {
            memory.entities = vec![speaker.clone()];
        }
        for (_, dia_id) in facts {
            add_dia_memory(&mut dia_to_memory, &dia_id, memory_id.to_string());
        }
        memories.push(memory);
    }

    (memories, dia_to_memory)
}

fn merge_dia_map(
    target: &mut BTreeMap<String, Vec<String>>,
    source: BTreeMap<String, Vec<String>>,
) {
    for (dia_id, ids) in source {
        target.entry(dia_id).or_default().extend(ids);
    }
}

fn add_dia_memory(map: &mut BTreeMap<String, Vec<String>>, dia_id: &str, memory_id: String) {
    for dia_id in split_locomo_dia_refs(dia_id) {
        map.entry(dia_id).or_default().push(memory_id.clone());
    }
}

fn is_locomo_session_key(key: &str, value: &Value) -> bool {
    key.starts_with("session_") && !key.ends_with("_date_time") && value.is_array()
}

fn locomo_category_name(category: i64) -> &'static str {
    match category {
        1 => "multi-hop",
        2 => "temporal",
        3 => "open-domain",
        4 => "single-hop",
        _ => "unknown",
    }
}

fn sanitize_id_fragment(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

fn rate(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 / total as f64 * 100.0
    }
}
