use anyhow::{anyhow, Result};
use noema_core::benchmark::LocomoMemorySource;
use noema_core::config::NoemaConfig;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, clap::Args)]
pub struct BenchArgs {
    #[arg(long, default_value_t = 1000)]
    pub memories: usize,
    #[arg(long, default_value_t = 8)]
    pub queries: usize,
    #[arg(long, default_value_t = 50)]
    pub iterations: usize,
    #[arg(long)]
    pub mem0_targets: bool,
    #[arg(long)]
    pub mem0_result: Option<PathBuf>,
    #[arg(long)]
    pub locomo_dataset: Option<PathBuf>,
    #[arg(long)]
    pub locomo_evidence: Option<PathBuf>,
    #[arg(long)]
    pub locomo_predict_input: Option<PathBuf>,
    #[arg(long, default_value_t = 50)]
    pub top_k: usize,
    #[arg(long, default_value = "raw")]
    pub locomo_memory_source: String,
    #[arg(long)]
    pub locomo_predict_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_predict_dir: Option<PathBuf>,
    #[arg(long)]
    pub locomo_answer_tasks_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_answer_tasks_input: Option<PathBuf>,
    #[arg(long)]
    pub locomo_answer_prompt_char_budget: Option<usize>,
    #[arg(long)]
    pub locomo_retry_answer_tasks_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_answer_results: Option<PathBuf>,
    #[arg(long)]
    pub locomo_judge_tasks_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_retry_judge_tasks_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_judge_results: Option<PathBuf>,
    #[arg(long)]
    pub locomo_final_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_status_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_retention_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_report_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_host_manifest_input: Option<PathBuf>,
    #[arg(long)]
    pub locomo_target_output: Option<PathBuf>,
    #[arg(long)]
    pub locomo_require_beats_mem0: bool,
    #[arg(long)]
    pub locomo_fail_if_incomplete: bool,
}

pub fn run_bench(args: BenchArgs, _cfg: &NoemaConfig) -> anyhow::Result<()> {
    let BenchArgs {
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
    } = args;

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
    use serde_json::json;
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
