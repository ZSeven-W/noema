#![allow(dead_code)]

use std::collections::{BTreeMap, HashSet};

use serde_json::{json, Value};

use crate::error::{NoemaError, Result};

use super::locomo::locomo_answer_prompt_retention_json_from_tasks;
use super::prompt::locomo_judge_task_prompt;
use super::score::{
    add_locomo_metric, locomo_category_metric_json, locomo_overall_metric_json,
    parse_locomo_answer_result_states, parse_locomo_answer_results,
    parse_locomo_judge_result_states, parse_locomo_judge_results, pending_id_samples,
};
use super::{LocomoAnswerResultState, LocomoJudgeResultState, LocomoMetricAccumulator};

// ---------------------------------------------------------------------------
// Run report
// ---------------------------------------------------------------------------

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
                super::locomo_target_verdict_json(&final_result, top_k)?
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

// ---------------------------------------------------------------------------
// Judge task JSONL generation
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Final result from judgments
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Status summary
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Task prompt builder (judge)
// ---------------------------------------------------------------------------

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
