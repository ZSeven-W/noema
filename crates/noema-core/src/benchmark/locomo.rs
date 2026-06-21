#![allow(dead_code)]

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use serde_json::{json, Value};

use crate::error::{NoemaError, Result};
use crate::ids::{TenantId, UserId};
use crate::memory::MemoryRecord;
use crate::recall::recall;
use crate::sensitivity::Principal;

use super::memory::locomo_memories;
use super::prompt::locomo_answer_task_prompt;
use super::score::{
    add_locomo_metric, locomo_category_metric_json, locomo_evidence_hits_in_search_prefix,
    locomo_overall_metric_json, parse_locomo_answer_result_states,
    parse_locomo_answer_task_prompt_stats, rate,
};
use super::{
    LocomoAnswerResultState, LocomoDatasetSummary, LocomoEvidenceReport, LocomoMemorySource,
    LocomoMetricAccumulator,
};

// ---------------------------------------------------------------------------
// Dataset summary
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Evidence retrieval
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Predict export
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Answer task JSONL generation
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Answer prompt retention audit
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Task prompt builder (answer)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Dataset parsing helpers (shared with memory module)
// ---------------------------------------------------------------------------

pub(super) fn is_locomo_session_key(key: &str, value: &Value) -> bool {
    key.starts_with("session_") && !key.ends_with("_date_time") && value.is_array()
}

pub(super) fn locomo_category_name(category: i64) -> &'static str {
    match category {
        1 => "multi-hop",
        2 => "temporal",
        3 => "open-domain",
        4 => "single-hop",
        _ => "unknown",
    }
}

pub(super) fn sanitize_id_fragment(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

pub(super) fn locomo_evidence_refs(qa: &Value) -> Vec<String> {
    qa.get("evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .flat_map(split_locomo_dia_refs)
        .collect()
}

pub(super) fn split_locomo_dia_refs(value: &str) -> Vec<String> {
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

pub(super) fn locomo_answer_text(value: Option<&Value>) -> String {
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
