use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn init_creates_layout() {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("noema").unwrap();
    cmd.env("NOEMA_ROOT", dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("initialized"));
    assert!(dir.path().join("tenants/personal/hippocampus").exists());
}

#[test]
fn remember_routes_to_review_queue() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["remember", "Prefer Rust for Noema.", "--tag", "rust"])
        .assert()
        .success()
        .stdout(predicate::str::contains("queued"));
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .assert()
        .success()
        .stdout(predicate::str::contains("Prefer Rust for Noema."));
}

#[test]
fn explicit_remember_accept_and_recall_outputs_memory_pack() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["remember", "老李爱健身", "--accept"])
        .assert()
        .success()
        .stdout(predicate::str::contains("accepted"));
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["recall", "老李爱做什么"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## Relevant Memories"))
        .stdout(predicate::str::contains("老李爱健身"));
}

#[test]
fn personal_mode_rejects_confidential_memory() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args([
            "remember",
            "Confidential launch plan.",
            "--sensitivity",
            "confidential",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "confidential and restricted sensitivity require enterprise mode",
        ));
}

#[test]
fn edit_and_merge_are_event_sourced_review_actions() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["remember", "Prefer Rust for Noema."])
        .assert()
        .success();

    let first = Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .output()
        .unwrap();
    assert!(first.status.success());
    let first_stdout = String::from_utf8(first.stdout).unwrap();
    let first_candidate_id = first_stdout.split_whitespace().next().unwrap().to_string();
    let target_memory_id = first_candidate_id.replacen("cand_", "mem_", 1);

    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["accept", &first_candidate_id])
        .assert()
        .success();

    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["remember", "Prefer Rust for Noema again."])
        .assert()
        .success();

    let second = Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .output()
        .unwrap();
    assert!(second.status.success());
    let second_stdout = String::from_utf8(second.stdout).unwrap();
    let candidate_id = second_stdout.split_whitespace().next().unwrap().to_string();

    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args([
            "edit",
            &candidate_id,
            "--body",
            "Prefer Rust for Noema memory.",
            "--reason",
            "clarify",
        ])
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .assert()
        .success()
        .stdout(predicate::str::contains("Prefer Rust for Noema memory."));

    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args([
            "merge",
            &candidate_id,
            &target_memory_id,
            "--reason",
            "duplicate",
        ])
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn bench_outputs_markdown_table_for_readme() {
    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--memories",
            "8",
            "--queries",
            "2",
            "--iterations",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("| Scenario |"))
        .stdout(predicate::str::contains("| Phase |"))
        .stdout(predicate::str::contains("noema_engine_recall"))
        .stdout(predicate::str::contains("zode_turn_injection_equivalent"));
}

#[test]
fn bench_can_print_mem0_reference_targets() {
    Command::cargo_bin("noema")
        .unwrap()
        .args(["bench", "--mem0-targets"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Mem0 benchmark targets"))
        .stdout(predicate::str::contains("Noema must exceed"))
        .stdout(predicate::str::contains("LoCoMo"))
        .stdout(predicate::str::contains("BEAM 10M"));
}

#[test]
fn bench_can_summarize_mem0_result_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mem0-result.json");
    std::fs::write(
        &path,
        r#"{
          "metadata": {"benchmark": "beam", "total_questions": 1},
          "metrics_by_cutoff": {
            "top_200": {
              "overall": {"total": 1, "avg_score": 0.641}
            }
          },
          "evaluations": [{"search_latency_ms": 42.0}]
        }"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args(["bench", "--mem0-result", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Mem0 result summary"))
        .stdout(predicate::str::contains("beam"))
        .stdout(predicate::str::contains("64.1"));
}

#[test]
fn bench_can_summarize_locomo_dataset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("locomo.json");
    std::fs::write(
        &path,
        r#"[
          {
            "conversation": {
              "speaker_a": "A",
              "speaker_b": "B",
              "session_1_date_time": "1:00 pm on 1 May, 2023",
              "session_1": [
                {"speaker": "A", "dia_id": "D1:1", "text": "I prefer Rust."}
              ]
            },
            "qa": [
              {"question": "What do I prefer?", "answer": "Rust", "evidence": ["D1:1"], "category": 4}
            ]
          }
        ]"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args(["bench", "--locomo-dataset", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("LOCOMO dataset summary"))
        .stdout(predicate::str::contains("conversations=1"))
        .stdout(predicate::str::contains("single-hop"));
}

#[test]
fn bench_can_run_locomo_evidence_retrieval() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("locomo.json");
    std::fs::write(
        &path,
        r#"[
          {
            "conversation": {
              "speaker_a": "A",
              "speaker_b": "B",
              "session_1_date_time": "1:00 pm on 1 May, 2023",
              "session_1": [
                {"speaker": "A", "dia_id": "D1:1", "text": "I prefer Rust modules."}
              ]
            },
            "qa": [
              {"question": "What modules do I prefer?", "answer": "Rust", "evidence": ["D1:1"], "category": 4}
            ]
          }
        ]"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-evidence",
            path.to_str().unwrap(),
            "--top-k",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("LOCOMO evidence retrieval"))
        .stdout(predicate::str::contains("any_evidence_hit"));
}

#[test]
fn bench_can_run_locomo_observation_evidence_retrieval() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("locomo.json");
    std::fs::write(
        &path,
        r#"[
          {
            "conversation": {
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_1_date_time": "1:56 pm on 8 May, 2023",
              "session_1": [
                {"speaker": "Caroline", "dia_id": "D1:1", "text": "I did that yesterday."},
                {"speaker": "Melanie", "dia_id": "D1:2", "text": "The LGBTQ support group was inspiring."}
              ]
            },
            "observation": {
              "session_1_observation": {
                "Caroline": [
                  ["Caroline went to the LGBTQ support group on 7 May 2023.", "D1:1"]
                ]
              }
            },
            "qa": [
              {"question": "When did Caroline go to the LGBTQ support group?", "answer": "7 May 2023", "evidence": ["D1:1"], "category": 2}
            ]
          }
        ]"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-evidence",
            path.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-memory-source",
            "observation",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("memory_source=observation"))
        .stdout(predicate::str::contains(
            "| any_evidence_hit | 1/1 | 100.0 |",
        ));
}

#[test]
fn bench_can_run_locomo_raw_plus_fact_layer_evidence_retrieval() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("locomo.json");
    std::fs::write(
        &path,
        r#"[
          {
            "conversation": {
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_1": [
                {"speaker": "Caroline", "dia_id": "D1:1", "text": "I joined the first one."},
                {"speaker": "Caroline", "dia_id": "D1:2", "text": "The second one was later."}
              ]
            },
            "observation": {
              "session_1_observation": {
                "Caroline": [
                  ["Caroline attended a Pride march.", "D1:1"],
                  ["Caroline attended a trans rights meetup.", "D1:2"]
                ]
              }
            },
            "qa": [
              {"question": "What events has Caroline attended?", "answer": "A Pride march and a trans rights meetup.", "evidence": ["D1:1", "D1:2"], "category": 1}
            ]
          }
        ]"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-evidence",
            path.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-memory-source",
            "raw-plus-fact-layer",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "memory_source=raw-plus-fact-layer",
        ))
        .stdout(predicate::str::contains(
            "| all_evidence_hit | 1/1 | 100.0 |",
        ));
}

#[test]
fn bench_can_write_locomo_predict_json() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("locomo.json");
    let output = dir.path().join("locomo-predict.json");
    std::fs::write(
        &input,
        r#"[
          {
            "conversation": {
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_1": [
                {"speaker": "Caroline", "dia_id": "D1:1", "text": "I joined the first one."},
                {"speaker": "Caroline", "dia_id": "D1:2", "text": "The second one was later."}
              ]
            },
            "observation": {
              "session_1_observation": {
                "Caroline": [
                  ["Caroline attended a Pride march.", "D1:1"],
                  ["Caroline attended a trans rights meetup.", "D1:2"]
                ]
              }
            },
            "qa": [
              {"question": "What events has Caroline attended?", "answer": "A Pride march and a trans rights meetup.", "evidence": ["D1:1", "D1:2"], "category": 1}
            ]
          }
        ]"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-evidence",
            input.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-memory-source",
            "raw-plus-fact-layer",
            "--locomo-predict-output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO predict JSON"));

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    assert_eq!(json["metadata"]["benchmark"], "locomo");
    assert_eq!(json["metadata"]["memory_source"], "raw-plus-fact-layer");
    assert_eq!(
        json["evaluations"][0]["cutoff_results"]["top_1"]["judgment"],
        "EVIDENCE_HIT"
    );
}

#[test]
fn bench_can_write_mem0_compatible_locomo_predict_dir() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("locomo.json");
    let output_dir = dir.path().join("predict");
    std::fs::write(
        &input,
        r#"[
          {
            "conversation": {
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_1": [
                {"speaker": "Caroline", "dia_id": "D1:1", "text": "I joined the first one."},
                {"speaker": "Caroline", "dia_id": "D1:2", "text": "The second one was later."}
              ]
            },
            "observation": {
              "session_1_observation": {
                "Caroline": [
                  ["Caroline attended a Pride march.", "D1:1"],
                  ["Caroline attended a trans rights meetup.", "D1:2"]
                ]
              }
            },
            "qa": [
              {"question": "What events has Caroline attended?", "answer": "A Pride march and a trans rights meetup.", "evidence": ["D1:1", "D1:2"], "category": 1}
            ]
          }
        ]"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-evidence",
            input.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-memory-source",
            "raw-plus-fact-layer",
            "--locomo-predict-dir",
            output_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "wrote mem0-compatible LOCOMO predict dir",
        ));

    let item_path = output_dir.join("conv0_q0.json");
    assert!(item_path.exists());
    let item: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(item_path).unwrap()).unwrap();
    assert_eq!(item["question_id"], "conv0_q0");
    assert_eq!(
        item["retrieval"]["search_results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(item.get("cutoff_results").is_none());
    assert!(output_dir.join("_noema_predict_summary.json").exists());
}

#[test]
fn bench_can_write_locomo_answer_tasks_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("locomo.json");
    let output = dir.path().join("answer-tasks.jsonl");
    std::fs::write(
        &input,
        r#"[
          {
            "conversation": {
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_1": [
                {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."}
              ]
            },
            "qa": [
              {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D1:1"], "category": 4}
            ]
          }
        ]"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-evidence",
            input.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-memory-source",
            "raw-plus-fact-layer",
            "--locomo-answer-tasks-output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO answer tasks"));

    let text = std::fs::read_to_string(output).unwrap();
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first["custom_id"], "locomo-answer-conv0_q0-top_1");
    assert!(first["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Caroline attended a Pride march."));
}

#[test]
fn bench_can_write_locomo_answer_tasks_with_prompt_budget() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("predict.json");
    let output = dir.path().join("answer-tasks.jsonl");
    let first_memory = format!("alpha retained {}", "a".repeat(180));
    let second_memory = format!("beta omitted {}", "b".repeat(180));
    let predict = serde_json::json!({
        "metadata": {"benchmark": "locomo"},
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "Which memory should remain in budget?",
                "ground_truth_answer": "alpha",
                "category": 1,
                "retrieval": {
                    "search_results": [
                        {"memory": first_memory},
                        {"memory": second_memory}
                    ]
                }
            }
        ]
    });
    std::fs::write(&input, serde_json::to_vec(&predict).unwrap()).unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            input.to_str().unwrap(),
            "--top-k",
            "2",
            "--locomo-answer-prompt-char-budget",
            "2700",
            "--locomo-answer-tasks-output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO answer tasks"));

    let text = std::fs::read_to_string(output).unwrap();
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let prompt = first["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.len() <= 2700, "prompt len was {}", prompt.len());
    assert!(prompt.contains("alpha retained"));
    assert!(!prompt.contains("beta omitted"));
}

#[test]
fn bench_can_write_locomo_answer_prompt_retention_json() {
    let dir = tempfile::tempdir().unwrap();
    let predict_path = dir.path().join("predict.json");
    let tasks_path = dir.path().join("answer-tasks.jsonl");
    let output_path = dir.path().join("retention.json");
    let predict = serde_json::json!({
        "metadata": {"benchmark": "locomo"},
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "What stayed in prompt?",
                "evidence": ["D1:1"],
                "retrieval": {
                    "search_results": [
                        {"memory": "[D1:1] retained evidence"},
                        {"memory": "[D2:1] unrelated"}
                    ]
                }
            },
            {
                "question_id": "conv0_q1",
                "question": "What was omitted?",
                "evidence": ["D9:1"],
                "retrieval": {
                    "search_results": [
                        {"memory": "[D4:1] retained unrelated"},
                        {"memory": "[D9:1] evidence only outside budget"}
                    ]
                }
            }
        ]
    });
    std::fs::write(&predict_path, serde_json::to_vec(&predict).unwrap()).unwrap();
    let task_lines = [
        serde_json::json!({
            "custom_id": "locomo-answer-conv0_q0-top_2",
            "question_id": "conv0_q0",
            "prompt_stats": {
                "prompt_char_budget": 96000,
                "retrieval_results_in_prompt": 1
            },
            "messages": [{"role": "user", "content": "prompt"}]
        }),
        serde_json::json!({
            "custom_id": "locomo-answer-conv0_q1-top_2",
            "question_id": "conv0_q1",
            "prompt_stats": {
                "prompt_char_budget": 96000,
                "retrieval_results_in_prompt": 1
            },
            "messages": [{"role": "user", "content": "prompt"}]
        }),
    ]
    .into_iter()
    .map(|task| serde_json::to_string(&task).unwrap())
    .collect::<Vec<_>>()
    .join("\n");
    std::fs::write(&tasks_path, task_lines).unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            predict_path.to_str().unwrap(),
            "--top-k",
            "2",
            "--locomo-answer-tasks-input",
            tasks_path.to_str().unwrap(),
            "--locomo-retention-output",
            output_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "wrote LOCOMO answer prompt retention",
        ));

    let audit: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output_path).unwrap()).unwrap();
    assert_eq!(audit["overall"]["baseline_any_evidence_hits"], 2);
    assert_eq!(audit["overall"]["retained_any_evidence_hits"], 1);
    assert_eq!(audit["overall"]["baseline_any_hits_lost"], 1);
}

#[test]
fn bench_can_write_locomo_run_report_json() {
    let dir = tempfile::tempdir().unwrap();
    let predict_path = dir.path().join("predict.json");
    let tasks_path = dir.path().join("answer-tasks.jsonl");
    let answers_path = dir.path().join("answers.jsonl");
    let manifest_path = dir.path().join("host-run.manifest.json");
    let output_path = dir.path().join("report.json");
    let predict = serde_json::json!({
        "metadata": {"benchmark": "locomo"},
        "metrics_by_cutoff": {
            "top_2": {
                "overall": {
                    "total": 2,
                    "correct": 2,
                    "accuracy": 100.0
                }
            }
        },
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "What stayed in prompt?",
                "evidence": ["D1:1"],
                "retrieval": {
                    "search_results": [
                        {"memory": "[D1:1] retained evidence"},
                        {"memory": "[D2:1] unrelated"}
                    ]
                }
            },
            {
                "question_id": "conv0_q1",
                "question": "What was omitted?",
                "evidence": ["D9:1"],
                "retrieval": {
                    "search_results": [
                        {"memory": "[D4:1] retained unrelated"},
                        {"memory": "[D9:1] evidence only outside budget"}
                    ]
                }
            }
        ]
    });
    std::fs::write(&predict_path, serde_json::to_vec(&predict).unwrap()).unwrap();
    let task_lines = [
        serde_json::json!({
            "custom_id": "locomo-answer-conv0_q0-top_2",
            "question_id": "conv0_q0",
            "prompt_stats": {
                "prompt_char_budget": 96000,
                "prompt_chars": 100,
                "retrieval_results_in_prompt": 1
            },
            "messages": [{"role": "user", "content": "prompt"}]
        }),
        serde_json::json!({
            "custom_id": "locomo-answer-conv0_q1-top_2",
            "question_id": "conv0_q1",
            "prompt_stats": {
                "prompt_char_budget": 96000,
                "prompt_chars": 100,
                "retrieval_results_in_prompt": 1
            },
            "messages": [{"role": "user", "content": "prompt"}]
        }),
    ]
    .into_iter()
    .map(|task| serde_json::to_string(&task).unwrap())
    .collect::<Vec<_>>()
    .join("\n");
    std::fs::write(&tasks_path, task_lines).unwrap();
    std::fs::write(
        &answers_path,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_2","answer":"retained"}
{"custom_id":"locomo-answer-conv0_q1-top_2","answer":"zode exited with status 1"}"#,
    )
    .unwrap();
    std::fs::write(
        &manifest_path,
        r#"{
          "runner": "zode",
          "provider_blocked": true,
          "provider_blocker_reason": "http_402_payment_required",
          "execution": {
            "tasks_total": 2,
            "pending_before_run": 2,
            "run": 1,
            "unrun_due_to_provider_blocker": 1
          }
        }"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            predict_path.to_str().unwrap(),
            "--top-k",
            "2",
            "--locomo-answer-tasks-input",
            tasks_path.to_str().unwrap(),
            "--locomo-answer-results",
            answers_path.to_str().unwrap(),
            "--locomo-host-manifest-input",
            manifest_path.to_str().unwrap(),
            "--locomo-report-output",
            output_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO run report"));

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output_path).unwrap()).unwrap();
    assert_eq!(report["metadata"]["eval_mode"], "locomo_run_report");
    assert_eq!(report["predict_proxy"]["overall"]["accuracy"], 100.0);
    assert_eq!(
        report["prompt_retention"]["overall"]["retained_any_evidence_hits"],
        1
    );
    assert_eq!(report["status"]["answers"]["retryable"], 1);
    assert_eq!(
        report["completion"]["blocked_reason"],
        "host_provider_blocked"
    );
    assert_eq!(report["completion"]["host_blocked"], true);
    assert_eq!(report["next_action"]["kind"], "resolve_provider_blocker");
    assert_eq!(report["host_runner"]["provider_blocked"], true);
    assert_eq!(
        report["host_runner"]["execution"]["unrun_due_to_provider_blocker"],
        1
    );
}

#[test]
fn bench_can_write_locomo_judge_tasks_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("locomo.json");
    let answers = dir.path().join("answers.jsonl");
    let output = dir.path().join("judge-tasks.jsonl");
    std::fs::write(
        &input,
        r#"[
          {
            "conversation": {
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_1": [
                {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."}
              ]
            },
            "qa": [
              {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D1:1"], "category": 4}
            ]
          }
        ]"#,
    )
    .unwrap();
    std::fs::write(
        &answers,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A Pride march."}
"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-evidence",
            input.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-memory-source",
            "raw-plus-fact-layer",
            "--locomo-answer-results",
            answers.to_str().unwrap(),
            "--locomo-judge-tasks-output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO judge tasks"));

    let text = std::fs::read_to_string(output).unwrap();
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first["custom_id"], "locomo-judge-conv0_q0-top_1");
    assert_eq!(first["generated_answer"], "A Pride march.");
    assert!(first["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("Gold answer: A Pride march."));
}

#[test]
fn bench_can_write_locomo_final_result_json() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("locomo.json");
    let answers = dir.path().join("answers.jsonl");
    let judgments = dir.path().join("judgments.jsonl");
    let output = dir.path().join("final.json");
    std::fs::write(
        &input,
        r#"[
          {
            "conversation": {
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_1": [
                {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."}
              ]
            },
            "qa": [
              {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D1:1"], "category": 4}
            ]
          }
        ]"#,
    )
    .unwrap();
    std::fs::write(
        &answers,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A Pride march."}
"#,
    )
    .unwrap();
    std::fs::write(
        &judgments,
        r#"{"custom_id":"locomo-judge-conv0_q0-top_1","label":"CORRECT","reasoning":"matches"}
"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-evidence",
            input.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-memory-source",
            "raw-plus-fact-layer",
            "--locomo-answer-results",
            answers.to_str().unwrap(),
            "--locomo-judge-results",
            judgments.to_str().unwrap(),
            "--locomo-final-output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO final result"));

    let final_result: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    assert_eq!(
        final_result["metrics_by_cutoff"]["top_1"]["overall"]["accuracy"],
        100.0
    );
    assert_eq!(
        final_result["evaluations"][0]["cutoff_results"]["top_1"]["judgment"],
        "CORRECT"
    );
}

#[test]
fn bench_can_write_locomo_final_result_from_predict_input() {
    let dir = tempfile::tempdir().unwrap();
    let predict = dir.path().join("predict.json");
    let answers = dir.path().join("answers.jsonl");
    let judgments = dir.path().join("judgments.jsonl");
    let output = dir.path().join("final.json");
    std::fs::write(
        &predict,
        r#"{
          "metadata": {
            "benchmark": "locomo",
            "top_k": 1,
            "top_k_cutoffs": ["top_1"],
            "eval_mode": "evidence_proxy_predict"
          },
          "evaluations": [
            {
              "question_id": "conv0_q0",
              "category": 4,
              "category_name": "temporal",
              "question": "What event did Caroline attend?",
              "ground_truth_answer": "A Pride march.",
              "retrieval": {
                "total_results": 1,
                "search_results": [
                  {"id": "mem_turn_conv0_D1_1", "memory": "Caroline attended a Pride march.", "score": 1.0}
                ]
              }
            }
          ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        &answers,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A Pride march."}
"#,
    )
    .unwrap();
    std::fs::write(
        &judgments,
        r#"{"custom_id":"locomo-judge-conv0_q0-top_1","label":"CORRECT","reasoning":"matches"}
"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            predict.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-answer-results",
            answers.to_str().unwrap(),
            "--locomo-judge-results",
            judgments.to_str().unwrap(),
            "--locomo-final-output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO final result"));

    let final_result: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    assert_eq!(
        final_result["metadata"]["eval_mode"],
        "answerer_judge_offline"
    );
    assert_eq!(
        final_result["metrics_by_cutoff"]["top_1"]["overall"]["accuracy"],
        100.0
    );
}

#[test]
fn bench_can_fail_when_locomo_score_does_not_beat_mem0() {
    let dir = tempfile::tempdir().unwrap();
    let predict = dir.path().join("predict.json");
    let answers = dir.path().join("answers.jsonl");
    let judgments = dir.path().join("judgments.jsonl");
    std::fs::write(
        &predict,
        r#"{
          "metadata": {
            "benchmark": "locomo",
            "top_k": 1,
            "top_k_cutoffs": ["top_1"],
            "eval_mode": "evidence_proxy_predict"
          },
          "evaluations": [
            {
              "question_id": "conv0_q0",
              "category": 4,
              "category_name": "temporal",
              "question": "What event did Caroline attend?",
              "ground_truth_answer": "A Pride march.",
              "retrieval": {
                "total_results": 1,
                "search_results": [
                  {"id": "mem_turn_conv0_D1_1", "memory": "Caroline attended a Pride march.", "score": 1.0}
                ]
              }
            }
          ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        &answers,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A different event."}
"#,
    )
    .unwrap();
    std::fs::write(
        &judgments,
        r#"{"custom_id":"locomo-judge-conv0_q0-top_1","label":"WRONG","reasoning":"does not match"}
"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            predict.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-answer-results",
            answers.to_str().unwrap(),
            "--locomo-judge-results",
            judgments.to_str().unwrap(),
            "--locomo-require-beats-mem0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "LOCOMO score 0.0 does not exceed Mem0 LoCoMo target 92.5",
        ));
}

#[test]
fn bench_can_write_locomo_status_from_predict_input() {
    let dir = tempfile::tempdir().unwrap();
    let predict = dir.path().join("predict.json");
    let answers = dir.path().join("answers.jsonl");
    let output = dir.path().join("status.json");
    std::fs::write(
        &predict,
        r#"{
          "metadata": {
            "benchmark": "locomo",
            "top_k": 1,
            "top_k_cutoffs": ["top_1"],
            "eval_mode": "evidence_proxy_predict"
          },
          "evaluations": [
            {
              "question_id": "conv0_q0",
              "category": 4,
              "category_name": "temporal",
              "question": "What event did Caroline attend?",
              "ground_truth_answer": "A Pride march.",
              "retrieval": {
                "total_results": 1,
                "search_results": [
                  {"id": "mem_turn_conv0_D1_1", "memory": "Caroline attended a Pride march.", "score": 1.0}
                ]
              }
            }
          ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        &answers,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"zode exited with status 1"}
"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            predict.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-answer-results",
            answers.to_str().unwrap(),
            "--locomo-status-output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO status"));

    let status: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    assert_eq!(status["answers"]["host_failed"], 1);
    assert_eq!(status["answers"]["retryable"], 1);
    assert_eq!(status["metadata"]["final_ready"], false);
}

#[test]
fn bench_can_fail_when_locomo_run_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let predict = dir.path().join("predict.json");
    let answers = dir.path().join("answers.jsonl");
    std::fs::write(
        &predict,
        r#"{
          "metadata": {
            "benchmark": "locomo",
            "top_k": 1,
            "top_k_cutoffs": ["top_1"],
            "eval_mode": "evidence_proxy_predict"
          },
          "evaluations": [
            {
              "question_id": "conv0_q0",
              "category": 4,
              "category_name": "temporal",
              "question": "What event did Caroline attend?",
              "ground_truth_answer": "A Pride march.",
              "retrieval": {
                "total_results": 1,
                "search_results": [
                  {"id": "mem_turn_conv0_D1_1", "memory": "Caroline attended a Pride march.", "score": 1.0}
                ]
              }
            }
          ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        &answers,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":"zode exited with status 1"}
"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            predict.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-answer-results",
            answers.to_str().unwrap(),
            "--locomo-fail-if-incomplete",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "LOCOMO run incomplete: blocked_reason=answers_incomplete next_action=retry_answers retryable=1",
        ));
}

#[test]
fn bench_incomplete_gate_reports_provider_blocker_reason() {
    let dir = tempfile::tempdir().unwrap();
    let predict = dir.path().join("predict.json");
    let answers = dir.path().join("answers.jsonl");
    std::fs::write(
        &predict,
        r#"{
          "metadata": {
            "benchmark": "locomo",
            "top_k": 1,
            "top_k_cutoffs": ["top_1"],
            "eval_mode": "evidence_proxy_predict"
          },
          "evaluations": [
            {
              "question_id": "conv0_q0",
              "category": 4,
              "category_name": "temporal",
              "question": "What event did Caroline attend?",
              "ground_truth_answer": "A Pride march.",
              "retrieval": {
                "total_results": 1,
                "search_results": [
                  {"id": "mem_turn_conv0_D1_1", "memory": "Caroline attended a Pride march.", "score": 1.0}
                ]
              }
            }
          ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        &answers,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":"zode exited with status 1","stderr":"HTTP 402 Payment Required: Insufficient Balance"}
"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            predict.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-answer-results",
            answers.to_str().unwrap(),
            "--locomo-fail-if-incomplete",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "blocked_reason=host_provider_blocked",
        ))
        .stderr(predicate::str::contains(
            "next_action=resolve_provider_blocker",
        ))
        .stderr(predicate::str::contains(
            "provider_blocker_reason=http_402_payment_required",
        ));
}

#[test]
fn bench_can_write_locomo_retry_answer_tasks_from_predict_input() {
    let dir = tempfile::tempdir().unwrap();
    let predict = dir.path().join("predict.json");
    let answers = dir.path().join("answers.jsonl");
    let output = dir.path().join("retry-answer-tasks.jsonl");
    std::fs::write(
        &predict,
        r#"{
          "metadata": {
            "benchmark": "locomo",
            "top_k": 1,
            "top_k_cutoffs": ["top_1"],
            "eval_mode": "evidence_proxy_predict"
          },
          "evaluations": [
            {
              "question_id": "conv0_q0",
              "category": 4,
              "category_name": "single-hop",
              "question": "What event did Caroline attend?",
              "ground_truth_answer": "A Pride march.",
              "retrieval": {
                "total_results": 1,
                "search_results": [
                  {"id": "mem_turn_conv0_D1_1", "memory": "Caroline attended a Pride march.", "score": 1.0}
                ]
              }
            },
            {
              "question_id": "conv0_q1",
              "category": 4,
              "category_name": "single-hop",
              "question": "What did Melanie bring?",
              "ground_truth_answer": "Iced tea.",
              "retrieval": {
                "total_results": 1,
                "search_results": [
                  {"id": "mem_turn_conv0_D1_2", "memory": "Melanie brought iced tea.", "score": 1.0}
                ]
              }
            }
          ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        &answers,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A Pride march."}
{"custom_id":"locomo-answer-conv0_q1-top_1","answer":"zode exited with status 1"}
"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            predict.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-answer-results",
            answers.to_str().unwrap(),
            "--locomo-retry-answer-tasks-output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO retry answer tasks"));

    let text = std::fs::read_to_string(output).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    let task: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(task["custom_id"], "locomo-answer-conv0_q1-top_1");
    assert!(task["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Melanie brought iced tea."));
}

#[test]
fn bench_can_write_locomo_retry_judge_tasks_from_predict_input() {
    let dir = tempfile::tempdir().unwrap();
    let predict = dir.path().join("predict.json");
    let answers = dir.path().join("answers.jsonl");
    let judgments = dir.path().join("judgments.jsonl");
    let output = dir.path().join("retry-judge-tasks.jsonl");
    std::fs::write(
        &predict,
        r#"{
          "metadata": {
            "benchmark": "locomo",
            "top_k": 1,
            "top_k_cutoffs": ["top_1"],
            "eval_mode": "evidence_proxy_predict"
          },
          "evaluations": [
            {
              "question_id": "conv0_q0",
              "category": 4,
              "category_name": "single-hop",
              "question": "What event did Caroline attend?",
              "ground_truth_answer": "A Pride march.",
              "retrieval": {
                "total_results": 1,
                "search_results": [
                  {"id": "mem_turn_conv0_D1_1", "memory": "Caroline attended a Pride march.", "score": 1.0}
                ]
              }
            },
            {
              "question_id": "conv0_q1",
              "category": 4,
              "category_name": "single-hop",
              "question": "What did Melanie bring?",
              "ground_truth_answer": "Iced tea.",
              "retrieval": {
                "total_results": 1,
                "search_results": [
                  {"id": "mem_turn_conv0_D1_2", "memory": "Melanie brought iced tea.", "score": 1.0}
                ]
              }
            }
          ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        &answers,
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A Pride march."}
{"custom_id":"locomo-answer-conv0_q1-top_1","answer":"Iced tea."}
"#,
    )
    .unwrap();
    std::fs::write(
        &judgments,
        r#"{"custom_id":"locomo-judge-conv0_q0-top_1","label":"CORRECT","reasoning":"matches"}
{"custom_id":"locomo-judge-conv0_q1-top_1","label":"WRONG","reasoning":"zode judge output did not contain a JSON object","raw":"zode exited with status 1"}
"#,
    )
    .unwrap();

    Command::cargo_bin("noema")
        .unwrap()
        .args([
            "bench",
            "--locomo-predict-input",
            predict.to_str().unwrap(),
            "--top-k",
            "1",
            "--locomo-answer-results",
            answers.to_str().unwrap(),
            "--locomo-judge-results",
            judgments.to_str().unwrap(),
            "--locomo-retry-judge-tasks-output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote LOCOMO retry judge tasks"));

    let text = std::fs::read_to_string(output).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    let task: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(task["custom_id"], "locomo-judge-conv0_q1-top_1");
    assert_eq!(task["generated_answer"], "Iced tea.");
}
