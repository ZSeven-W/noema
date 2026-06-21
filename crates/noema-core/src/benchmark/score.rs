#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use crate::error::{NoemaError, Result};

use super::{
    LocomoAnswerResultState, LocomoJudgeResult, LocomoJudgeResultState, LocomoMetricAccumulator,
};

// ---------------------------------------------------------------------------
// Metric helpers
// ---------------------------------------------------------------------------

pub(super) fn add_locomo_metric(
    metric: &mut LocomoMetricAccumulator,
    category_id: i64,
    score: f64,
) {
    metric.category_id = category_id;
    metric.total += 1;
    metric.score_sum += score;
    if score >= 0.5 {
        metric.correct += 1;
    }
}

pub(super) fn locomo_overall_metric_json(metric: &LocomoMetricAccumulator) -> Value {
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

pub(super) fn locomo_category_metric_json(metric: &LocomoMetricAccumulator) -> Value {
    let mut value = locomo_overall_metric_json(metric);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("category_id".to_string(), json!(metric.category_id));
    }
    value
}

pub(super) fn rate(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 / total as f64 * 100.0
    }
}

// ---------------------------------------------------------------------------
// Mem0 result summary helpers
// ---------------------------------------------------------------------------

pub(super) fn cutoff_groups(
    cutoff_obj: &serde_json::Map<String, Value>,
) -> Vec<super::Mem0GroupSummary> {
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
        groups.push(super::Mem0GroupSummary {
            name: name.clone(),
            score_label,
            score,
            total,
        });
    }
    groups.sort_by(|left, right| left.name.cmp(&right.name));
    groups
}

pub(super) fn metric_score(obj: &serde_json::Map<String, Value>) -> (String, f64) {
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

pub(super) fn average_search_latency(evaluations: &[Value]) -> Option<f64> {
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

pub(super) fn average_retrieved_memories(evaluations: &[Value]) -> Option<f64> {
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

// ---------------------------------------------------------------------------
// Distribution / statistics helpers
// ---------------------------------------------------------------------------

pub(super) fn usize_distribution_json(values: &[usize]) -> Value {
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

// ---------------------------------------------------------------------------
// Answer task prompt stats parsing
// ---------------------------------------------------------------------------

pub(super) struct LocomoAnswerTaskPromptStats {
    pub tasks: usize,
    pub by_question_id: BTreeMap<String, LocomoTaskPromptStats>,
    pub prompt_budgets: BTreeSet<usize>,
    pub prompt_chars: Vec<usize>,
    pub retrieval_results_in_prompt: Vec<usize>,
    pub omitted_retrieval_results: Vec<usize>,
    pub truncated_memories: Vec<usize>,
}

pub(super) struct LocomoTaskPromptStats {
    pub retrieval_results_in_prompt: usize,
}

impl LocomoAnswerTaskPromptStats {
    pub fn prompt_summary_json(&self) -> Value {
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

pub(super) fn parse_locomo_answer_task_prompt_stats(
    text: &str,
) -> Result<LocomoAnswerTaskPromptStats> {
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

// ---------------------------------------------------------------------------
// Answer / judge result parsing
// ---------------------------------------------------------------------------

pub(super) fn parse_locomo_answer_results(text: &str) -> Result<BTreeMap<String, String>> {
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

pub(super) fn parse_locomo_answer_result_states(
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

pub(super) fn parse_locomo_judge_results(
    text: &str,
) -> Result<BTreeMap<String, LocomoJudgeResult>> {
    let (_, states) = parse_locomo_judge_result_states(text)?;
    Ok(states
        .into_iter()
        .filter_map(|(custom_id, state)| match state {
            LocomoJudgeResultState::Valid(judgment) => Some((custom_id, judgment)),
            LocomoJudgeResultState::InvalidLabel | LocomoJudgeResultState::HostFailed(_) => None,
        })
        .collect())
}

pub(super) fn parse_locomo_judge_result_states(
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

// ---------------------------------------------------------------------------
// Extract helpers
// ---------------------------------------------------------------------------

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

pub(super) fn locomo_answer_failure_reason(answer: &str, stderr: &str) -> &'static str {
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

pub(super) fn locomo_judge_failure_reason(
    label: &str,
    reasoning: &str,
    stderr: &str,
) -> &'static str {
    let combined = format!("{label}\n{reasoning}\n{stderr}");
    if combined.contains("HTTP 402") || combined.contains("Insufficient Balance") {
        return "http_402_payment_required";
    }
    if is_retryable_judge_failure(label, reasoning) {
        return "zode_non_json_output";
    }
    "unknown_failure"
}

pub(super) fn is_retryable_judge_failure(label: &str, reasoning: &str) -> bool {
    label == "WRONG" && reasoning.trim() == "zode judge output did not contain a JSON object"
}

pub(super) fn pending_id_samples(ids: &[String]) -> Vec<String> {
    ids.iter().take(20).cloned().collect()
}

// ---------------------------------------------------------------------------
// Evidence hit counting
// ---------------------------------------------------------------------------

pub(super) fn locomo_evidence_hits_in_search_prefix(
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
