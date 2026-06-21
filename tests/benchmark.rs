use noema_core::benchmark::{
    locomo_answer_prompt_retention_json_from_tasks, locomo_answer_tasks_jsonl_from_predict,
    locomo_answer_tasks_jsonl_from_predict_with_prompt_budget,
    locomo_final_result_json_from_judgments, locomo_judge_tasks_jsonl_from_answers,
    locomo_retry_answer_tasks_jsonl_from_results, locomo_retry_judge_tasks_jsonl_from_results,
    locomo_run_report_json_from_artifacts,
    locomo_run_report_json_from_artifacts_with_host_manifest, locomo_status_json_from_results,
    locomo_target_verdict_json, mem0_reference_targets, mem0_reference_targets_markdown_table,
    run_locomo_evidence_retrieval_json, run_locomo_evidence_retrieval_json_with_source,
    run_locomo_predict_json_with_source, run_recall_benchmark, summarize_locomo_dataset_json,
    summarize_mem0_result_json, BenchmarkScenario, LocomoMemorySource,
};
use serde_json::json;

#[test]
fn recall_benchmark_reports_engine_and_zode_injection_paths() {
    let dir = tempfile::tempdir().unwrap();
    let report = run_recall_benchmark(
        dir.path(),
        BenchmarkScenario {
            memory_count: 12,
            query_count: 3,
            iterations: 2,
        },
    )
    .unwrap();

    assert_eq!(report.memory_count, 12);
    assert_eq!(report.query_count, 3);
    assert_eq!(report.iterations, 2);
    assert!(report.generated_bytes > 0);
    assert_eq!(report.samples.len(), 2);
    assert_eq!(report.samples[0].name, "noema_engine_recall");
    assert_eq!(report.samples[1].name, "zode_turn_injection_equivalent");
    assert!(report.samples.iter().all(|sample| sample.mean_us > 0.0));
    assert!(report
        .samples
        .iter()
        .all(|sample| !sample.phases.is_empty()));
    assert!(report.samples[0]
        .phases
        .iter()
        .any(|phase| phase.name == "load_memories"));
    assert!(report.samples[0]
        .phases
        .iter()
        .any(|phase| phase.name == "score_memories"));
    assert!(report.samples[1]
        .phases
        .iter()
        .any(|phase| phase.name == "render_markdown"));
    assert!(report
        .to_markdown_table()
        .contains("| noema_engine_recall |"));
    assert!(report
        .to_phase_markdown_table()
        .contains("| noema_engine_recall | load_memories |"));
}

#[test]
fn mem0_reference_targets_define_strict_noema_goals() {
    let targets = mem0_reference_targets();

    assert_eq!(targets.len(), 4);
    assert!(targets
        .iter()
        .any(|target| target.benchmark == "LoCoMo" && target.mem0_score == 92.5));
    assert!(targets
        .iter()
        .all(|target| target.noema_target_score > target.mem0_score));
    assert!(mem0_reference_targets_markdown_table().contains("| BEAM 10M |"));
}

#[test]
fn mem0_result_summary_reads_metrics_by_cutoff_json() {
    let summary = summarize_mem0_result_json(
        r#"{
          "metadata": {
            "benchmark": "locomo",
            "total_questions": 2,
            "top_k_cutoffs": ["top_200"]
          },
          "metrics_by_cutoff": {
            "top_200": {
              "overall": {
                "total": 2,
                "correct": 1,
                "accuracy": 50.0
              },
              "by_category": {
                "single-hop": {
                  "total": 1,
                  "correct": 1,
                  "accuracy": 100.0
                }
              }
            }
          },
          "evaluations": [
            {"search_latency_ms": 10.0, "total_memories_retrieved": 200},
            {"retrieval": {"search_latency_ms": 30.0, "total_results": 50}}
          ]
        }"#,
    )
    .unwrap();

    assert_eq!(summary.benchmark, "locomo");
    assert_eq!(summary.total_questions, 2);
    assert_eq!(summary.cutoffs[0].cutoff, "top_200");
    assert_eq!(summary.cutoffs[0].score_label, "accuracy");
    assert!((summary.cutoffs[0].score - 50.0).abs() < f64::EPSILON);
    assert!((summary.avg_search_latency_ms.unwrap() - 20.0).abs() < f64::EPSILON);
    assert!(summary
        .to_markdown_table()
        .contains("| top_200 | accuracy | 50.0 |"));
}

#[test]
fn locomo_dataset_summary_reads_conversations_turns_and_categories() {
    let summary = summarize_locomo_dataset_json(
        r#"[
          {
            "sample_id": "conv0",
            "conversation": {
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_1_date_time": "1:56 pm on 8 May, 2023",
              "session_1": [
                {"speaker": "Caroline", "dia_id": "D1:1", "text": "I went to the LGBTQ support group yesterday."},
                {"speaker": "Melanie", "dia_id": "D1:2", "text": "That was on 7 May 2023."}
              ]
            },
            "qa": [
              {"question": "When did Caroline go to the support group?", "answer": "7 May 2023", "evidence": ["D1:1"], "category": 2},
              {"question": "Who replied?", "answer": "Melanie", "evidence": ["D1:2"], "category": 4}
            ]
          }
        ]"#,
    )
    .unwrap();

    assert_eq!(summary.conversations, 1);
    assert_eq!(summary.sessions, 1);
    assert_eq!(summary.turns, 2);
    assert_eq!(summary.questions, 2);
    assert_eq!(summary.evaluable_questions, 2);
    assert_eq!(summary.evidence_refs, 2);
    assert_eq!(summary.resolved_evidence_refs, 2);
    assert_eq!(summary.category_counts.get("temporal"), Some(&1));
    assert_eq!(summary.category_counts.get("single-hop"), Some(&1));
    assert!(summary.to_markdown_table().contains("| temporal | 1 |"));
}

#[test]
fn locomo_evidence_retrieval_reports_top_k_hits() {
    let report = run_locomo_evidence_retrieval_json(
        r#"[
          {
            "conversation": {
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_1_date_time": "1:56 pm on 8 May, 2023",
              "session_1": [
                {"speaker": "Caroline", "dia_id": "D1:1", "text": "I went to the LGBTQ support group yesterday."},
                {"speaker": "Melanie", "dia_id": "D1:2", "text": "That was on 7 May 2023."}
              ]
            },
            "qa": [
              {"question": "When did Caroline go to the LGBTQ support group?", "answer": "7 May 2023", "evidence": ["D1:1"], "category": 2}
            ]
          }
        ]"#,
        1,
    )
    .unwrap();

    assert_eq!(report.questions, 1);
    assert_eq!(report.top_k, 1);
    assert_eq!(report.any_evidence_hits, 1);
    assert_eq!(report.all_evidence_hits, 1);
    assert!((report.any_evidence_hit_rate - 100.0).abs() < f64::EPSILON);
    assert!(report
        .to_markdown_table()
        .contains("| any_evidence_hit | 1/1 | 100.0 |"));
}

#[test]
fn locomo_observation_source_can_retrieve_fact_layer_evidence() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1_date_time": "1:56 pm on 8 May, 2023",
          "session_1": [
            {"speaker": "Caroline", "dia_id": "D1:1", "text": "I did that yesterday."},
            {"speaker": "Melanie", "dia_id": "D1:2", "text": "That sounds meaningful."},
            {"speaker": "Melanie", "dia_id": "D1:3", "text": "That sounds inspiring."}
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
          {"question": "When was the LGBTQ support group?", "answer": "7 May 2023", "evidence": ["D1:1"], "category": 2}
        ]
      }
    ]"#;

    let raw_report =
        run_locomo_evidence_retrieval_json_with_source(fixture, 1, LocomoMemorySource::Raw)
            .unwrap();
    let observation_report =
        run_locomo_evidence_retrieval_json_with_source(fixture, 1, LocomoMemorySource::Observation)
            .unwrap();

    assert_eq!(raw_report.any_evidence_hits, 0);
    assert_eq!(
        observation_report.memory_source,
        LocomoMemorySource::Observation
    );
    assert_eq!(observation_report.questions, 1);
    assert_eq!(observation_report.any_evidence_hits, 1);
    assert_eq!(observation_report.all_evidence_hits, 1);
    assert!((observation_report.any_evidence_hit_rate - 100.0).abs() < f64::EPSILON);
}

#[test]
fn locomo_fact_layer_summary_preserves_multi_evidence_provenance() {
    let fixture = r#"[
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
    ]"#;

    let observation_report =
        run_locomo_evidence_retrieval_json_with_source(fixture, 1, LocomoMemorySource::Observation)
            .unwrap();
    let fact_layer_report =
        run_locomo_evidence_retrieval_json_with_source(fixture, 1, LocomoMemorySource::FactLayer)
            .unwrap();

    assert_eq!(observation_report.any_evidence_hits, 1);
    assert_eq!(observation_report.all_evidence_hits, 0);
    assert_eq!(
        fact_layer_report.memory_source,
        LocomoMemorySource::FactLayer
    );
    assert_eq!(fact_layer_report.any_evidence_hits, 1);
    assert_eq!(fact_layer_report.all_evidence_hits, 1);
}

#[test]
fn locomo_predict_json_exports_mem0_like_retrieval_payload() {
    let fixture = r#"[
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
    ]"#;

    let output =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();

    assert_eq!(output["metadata"]["benchmark"], "locomo");
    assert_eq!(output["metadata"]["provider"], "noema");
    assert_eq!(output["metadata"]["memory_source"], "raw-plus-fact-layer");
    assert_eq!(output["metadata"]["top_k_cutoffs"][0], "top_1");
    assert_eq!(
        output["metrics_by_cutoff"]["top_1"]["overall"]["accuracy"],
        100.0
    );
    assert!(output["metrics_by_cutoff"]["top_1"]["overall"]
        .get("category_id")
        .is_none());
    assert_eq!(
        output["metrics_by_cutoff"]["top_1"]["by_category"]["multi-hop"]["category_id"],
        1
    );
    assert_eq!(output["evaluations"][0]["question_id"], "conv0_q0");
    assert_eq!(output["evaluations"][0]["category_name"], "multi-hop");
    assert_eq!(
        output["evaluations"][0]["retrieval"]["search_results"][0]["id"],
        "mem_locomo_0_fact_layer_Caroline"
    );
    assert!(
        output["evaluations"][0]["retrieval"]["search_results"][0]["memory"]
            .as_str()
            .unwrap()
            .contains("Pride march")
    );
    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_1"]["judgment"],
        "EVIDENCE_HIT"
    );
    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_1"]["all_evidence_hit"],
        true
    );
}

#[test]
fn locomo_predict_json_keeps_unresolved_evidence_questions_for_mem0_evaluate_only() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1": [
            {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."}
          ]
        },
        "qa": [
          {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D9:9"], "category": 4}
        ]
      }
    ]"#;

    let output =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();

    assert_eq!(output["metadata"]["total_questions"], 1);
    assert_eq!(output["metrics_by_cutoff"]["top_1"]["overall"]["total"], 1);
    assert_eq!(
        output["metrics_by_cutoff"]["top_1"]["overall"]["correct"],
        0
    );
    assert_eq!(output["evaluations"].as_array().unwrap().len(), 1);
    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_1"]["judgment"],
        "MISS"
    );
    assert!(
        output["evaluations"][0]["retrieval"]["search_results"][0]["memory"]
            .as_str()
            .unwrap()
            .contains("Pride march")
    );
}

#[test]
fn locomo_predict_json_splits_semicolon_evidence_refs() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1": [
            {"speaker": "Melanie", "dia_id": "D1:1", "text": "Melanie painted a sunset last week."},
            {"speaker": "Melanie", "dia_id": "D1:2", "text": "Melanie said the sunset painting was recent."}
          ]
        },
        "qa": [
          {"question": "What did Melanie paint recently?", "answer": "A sunset.", "evidence": ["D1:1; D1:2"], "category": 4}
        ]
      }
    ]"#;

    let output = run_locomo_predict_json_with_source(fixture, 2, LocomoMemorySource::Raw).unwrap();

    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_2"]["judgment"],
        "EVIDENCE_HIT"
    );
    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_2"]["all_evidence_hit"],
        true
    );
    assert_eq!(
        output["metrics_by_cutoff"]["top_2"]["overall"]["correct"],
        1
    );
}

#[test]
fn locomo_predict_json_splits_space_separated_evidence_refs() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Evan",
          "speaker_b": "Sam",
          "session_1": [
            {"speaker": "Evan", "dia_id": "D1:1", "text": "Evan goes kayaking outdoors to reduce stress."},
            {"speaker": "Sam", "dia_id": "D1:2", "text": "Sam paints landscapes to cope with challenges."}
          ]
        },
        "qa": [
          {"question": "How do Evan and Sam handle stress outdoors and creatively?", "answer": "Kayaking and painting.", "evidence": ["D1:1 D1:2"], "category": 3}
        ]
      }
    ]"#;

    let output = run_locomo_predict_json_with_source(fixture, 2, LocomoMemorySource::Raw).unwrap();

    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_2"]["judgment"],
        "EVIDENCE_HIT"
    );
    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_2"]["all_evidence_hit"],
        true
    );
}

#[test]
fn locomo_predict_json_normalizes_leading_zero_evidence_refs() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Dave",
          "speaker_b": "Calvin",
          "session_30": [
            {"speaker": "Dave", "dia_id": "D30:5", "text": "Dave bought a vintage camera in November 2023."}
          ]
        },
        "qa": [
          {"question": "When did Dave buy a vintage camera?", "answer": "November 2023.", "evidence": ["D30:05"], "category": 2}
        ]
      }
    ]"#;

    let output = run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::Raw).unwrap();

    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_1"]["judgment"],
        "EVIDENCE_HIT"
    );
}

#[test]
fn locomo_raw_memory_uses_adjacent_turn_context_for_evidence() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Sam",
          "speaker_b": "Riley",
          "session_1": [
            {"speaker": "Sam", "dia_id": "D1:1", "text": "I made the purchase yesterday."},
            {"speaker": "Sam", "dia_id": "D1:2", "text": "The running shoes are for marathon training."}
          ]
        },
        "qa": [
          {"question": "What were the running shoes for?", "answer": "Marathon training.", "evidence": ["D1:1"], "category": 4}
        ]
      }
    ]"#;

    let output = run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::Raw).unwrap();

    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_1"]["judgment"],
        "EVIDENCE_HIT"
    );
    let results = output["evaluations"][0]["retrieval"]["search_results"]
        .as_array()
        .unwrap();
    assert!(results.iter().any(|result| result["memory"]
        .as_str()
        .unwrap()
        .contains("[D1:2] Sam: The running shoes are for marathon training.")));
}

#[test]
fn locomo_predict_json_counts_adjacent_context_as_evidence_provenance() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Sam",
          "speaker_b": "Riley",
          "session_1": [
            {"speaker": "Sam", "dia_id": "D1:1", "text": "Sam bought blue running shoes yesterday at a local store."},
            {"speaker": "Riley", "dia_id": "D1:2", "text": "The marathon training plan starts Monday."}
          ]
        },
        "qa": [
          {"question": "What plan starts Monday after Sam bought blue running shoes yesterday?", "answer": "The marathon training plan.", "evidence": ["D1:2"], "category": 4}
        ]
      }
    ]"#;

    let output = run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::Raw).unwrap();

    assert_eq!(
        output["evaluations"][0]["cutoff_results"]["top_1"]["judgment"],
        "EVIDENCE_HIT"
    );
    let results = output["evaluations"][0]["retrieval"]["search_results"]
        .as_array()
        .unwrap();
    assert!(results.iter().any(|result| result["memory"]
        .as_str()
        .unwrap()
        .contains("[D1:2] Riley: The marathon training plan starts Monday.")));
}

#[test]
fn locomo_raw_source_adds_session_episode_memory() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Nate",
          "speaker_b": "Joanna",
          "session_7_date_time": "7:37 pm on 15 April, 2022",
          "session_7": [
            {"speaker": "Nate", "dia_id": "D7:1", "text": "Hey Jo, guess what I did? Dyed my hair last week."},
            {"speaker": "Joanna", "dia_id": "D7:2", "text": "Wow, Nate! Can't wait to see it."}
          ]
        },
        "qa": [
          {"question": "What nickname does Nate use for Joanna?", "answer": "Jo", "evidence": ["D7:1"], "category": 3}
        ]
      }
    ]"#;

    let output = run_locomo_predict_json_with_source(fixture, 3, LocomoMemorySource::Raw).unwrap();
    let results = output["evaluations"][0]["retrieval"]["search_results"]
        .as_array()
        .unwrap();

    let episode = results
        .iter()
        .find(|result| result["id"] == "mem_locomo_0_session_7_episode")
        .expect("session episode memory should be retrieved");
    assert!(episode["memory"]
        .as_str()
        .unwrap()
        .contains("[session_7 episode, said on 7:37 pm on 15 April, 2022]"));
    assert!(
        output["evaluations"][0]["cutoff_results"]["top_3"]["any_evidence_hit"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn locomo_predict_json_preserves_non_string_ground_truth_answers() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1": [
            {"speaker": "Melanie", "dia_id": "D1:12", "text": "I painted that sunrise in 2022."}
          ]
        },
        "qa": [
          {"question": "When did Melanie paint a sunrise?", "answer": 2022, "evidence": ["D1:12"], "category": 2}
        ]
      }
    ]"#;

    let output =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();

    assert_eq!(output["evaluations"][0]["ground_truth_answer"], "2022");
}

#[test]
fn locomo_answer_tasks_jsonl_exports_host_llm_work_items() {
    let fixture = r#"[
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
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let lines = jsonl.lines().collect::<Vec<_>>();
    let task: serde_json::Value = serde_json::from_str(lines[0]).unwrap();

    assert_eq!(lines.len(), 1);
    assert_eq!(task["custom_id"], "locomo-answer-conv0_q0-top_1");
    assert_eq!(task["kind"], "locomo_answer_generation");
    assert_eq!(task["question_id"], "conv0_q0");
    assert_eq!(task["cutoff"], "top_1");
    assert_eq!(task["messages"][0]["role"], "user");
    assert!(task["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("ANSWER:"));
    assert!(task["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Caroline attended a Pride march."));
}

#[test]
fn locomo_answer_tasks_jsonl_compacts_session_episode_memories() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Nate",
          "speaker_b": "Joanna",
          "session_7": [
            {"speaker": "Nate", "dia_id": "D7:1", "text": "Hey Jo, guess what I did? Dyed my hair last week."},
            {"speaker": "Joanna", "dia_id": "D7:2", "text": "Wow, Nate! Can't wait to see it."},
            {"speaker": "Nate", "dia_id": "D7:3", "text": "The weather was calm and I cooked pasta."},
            {"speaker": "Joanna", "dia_id": "D7:4", "text": "I sorted old cables and cleaned the shelf."},
            {"speaker": "Nate", "dia_id": "D7:5", "text": "A neighbor mentioned controller accessories."},
            {"speaker": "Joanna", "dia_id": "D7:6", "text": "The unrelated errand took all afternoon."},
            {"speaker": "Nate", "dia_id": "D7:7", "text": "I bought printer paper near the station."}
          ]
        },
        "qa": [
          {"question": "What nickname does Nate use for Joanna?", "answer": "Jo", "evidence": ["D7:1"], "category": 3}
        ]
      }
    ]"#;
    let predict = run_locomo_predict_json_with_source(fixture, 3, LocomoMemorySource::Raw).unwrap();

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 3).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    assert!(prompt.contains("[compacted episode"));
    assert!(prompt.contains("Hey Jo"));
    assert!(!prompt.contains("printer paper"));
}

#[test]
fn locomo_answer_tasks_jsonl_can_apply_prompt_char_budget() {
    let first_memory = format!("alpha retained {}", "a".repeat(180));
    let second_memory = format!("beta omitted {}", "b".repeat(180));
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "Which memory should remain in budget?",
                "ground_truth_answer": "alpha",
                "category": 1,
                "retrieval": {
                    "search_results": [
                        { "memory": first_memory },
                        { "memory": second_memory }
                    ]
                }
            }
        ]
    });

    let jsonl =
        locomo_answer_tasks_jsonl_from_predict_with_prompt_budget(&predict, 2, Some(2600)).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    assert!(prompt.len() <= 2600, "prompt len was {}", prompt.len());
    assert_eq!(task["cutoff"], "top_2");
    assert_eq!(task["prompt_stats"]["prompt_char_budget"], 2600);
    assert_eq!(task["prompt_stats"]["prompt_chars"], prompt.len());
    assert_eq!(task["prompt_stats"]["top_k_requested"], 2);
    assert_eq!(task["prompt_stats"]["retrieval_results_available"], 2);
    assert_eq!(task["prompt_stats"]["retrieval_results_in_prompt"], 1);
    assert_eq!(task["prompt_stats"]["truncated_memories"], 0);
    assert_eq!(task["prompt_stats"]["omitted_retrieval_results"], 1);
    assert!(prompt.contains("alpha retained"));
    assert!(!prompt.contains("beta omitted"));
    assert!(prompt.contains("Question: Which memory should remain in budget?"));
    assert!(prompt.ends_with("ANSWER:"));
}

#[test]
fn locomo_answer_tasks_jsonl_compacts_fact_layer_summaries_by_question() {
    let observation_facts = (0..40)
        .map(|index| {
            if index == 25 {
                r#"["Melanie's favorite childhood book was Charlotte's Web.", "D6:10"]"#.to_string()
            } else {
                format!(
                    r#"["Melanie logged unrelated pottery studio note {index}.", "D6:{index}"]"#
                )
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let fixture = format!(
        r#"[
          {{
            "conversation": {{
              "speaker_a": "Caroline",
              "speaker_b": "Melanie",
              "session_6": [
                {{"speaker": "Melanie", "dia_id": "D6:10", "text": "Charlotte's Web was my favorite childhood book."}}
              ]
            }},
            "observation": {{
              "session_6_observation": {{
                "Melanie": [{observation_facts}]
              }}
            }},
            "qa": [
              {{"question": "What was Melanie's favorite book from her childhood?", "answer": "Charlotte's Web", "evidence": ["D6:10"], "category": 4}}
            ]
          }}
        ]"#
    );
    let predict =
        run_locomo_predict_json_with_source(&fixture, 1, LocomoMemorySource::FactLayer).unwrap();

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    assert!(prompt.contains("[compacted facts"));
    assert!(prompt.contains("[D6:10] Melanie's favorite childhood book was Charlotte's Web."));
    assert!(!prompt.contains("unrelated pottery studio note 39"));
}

#[test]
fn locomo_answer_tasks_jsonl_surfaces_relevant_clues_near_question() {
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "What does Melanie do with her family on hikes?",
                "ground_truth_answer": "Roast marshmallows and tell stories",
                "category": 1,
                "retrieval": {
                    "search_results": [
                        {
                            "memory": "[speaker fact-layer summary] Melanie: Melanie enjoyed an unrelated art workshop.; Melanie's family tradition includes a camping trip where they roast marshmallows and tell stories around the campfire.; Melanie mentioned a different unrelated book club note."
                        }
                    ]
                }
            }
        ]
    });

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    let memories_index = prompt.find("Retrieved memories:").unwrap();
    let question_index = prompt
        .find("Question: What does Melanie do with her family on hikes?")
        .unwrap();
    let clues_index = prompt.rfind("\nMost relevant extracted clues:\n").unwrap();
    assert!(memories_index < clues_index);
    assert!(question_index < clues_index);
    assert!(
        prompt[clues_index..].contains("roast marshmallows and tell stories around the campfire")
    );
}

#[test]
fn locomo_answer_tasks_jsonl_ranks_irregular_word_variants_in_clues() {
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "What did Gina find for her clothing store on 1 February, 2023?",
                "ground_truth_answer": "The perfect spot for her store",
                "category": 4,
                "retrieval": {
                    "search_results": [
                        { "memory": "[session_3 episode, said on 12:48 am on 1 February, 2023]\n- [D3:2] Gina emailed some wholesalers and one replied yes so she can expand her clothing store.\n- [D3:3] Jon: Wow, Gina! You found the perfect spot for your store. Way to go!" }
                    ]
                }
            }
        ]
    });

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();
    let clues_index = prompt.rfind("\nMost relevant extracted clues:\n").unwrap();
    let clue_block = &prompt[clues_index..];

    assert!(
        clue_block.find("found the perfect spot").unwrap()
            < clue_block.find("emailed some wholesalers").unwrap()
    );
}

#[test]
fn locomo_answer_tasks_jsonl_documents_locomo_loose_attribution_style() {
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "What did Calvin and his friends arrange for in the park?",
                "ground_truth_answer": "regular walks together",
                "category": 4,
                "retrieval": {
                    "search_results": [
                        { "memory": "[D10:3] Dave: I arranged with friends for regular walks together in the park.\n[D10:4] Calvin: That sounds like a great plan!" }
                    ]
                }
            }
        ]
    });

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    assert!(prompt.contains("LOCOMO gold answers care about the event or object"));
}

#[test]
fn locomo_answer_tasks_jsonl_adds_short_answer_candidates_from_clues() {
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "What did Calvin and his friends arrange for in the park?",
                "ground_truth_answer": "regular walks together",
                "category": 4,
                "retrieval": {
                    "search_results": [
                        { "memory": "[session_10 episode, said on 7:56 pm on 7 July, 2023]\n- [D10:3] Dave: I arranged with friends for regular walks together in the park.\n- [D10:4] Calvin: That sounds like a great plan!" }
                    ]
                }
            }
        ]
    });

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    assert!(prompt.contains("Most likely answer candidates"));
    assert!(prompt.contains("regular walks together in the park"));
}

#[test]
fn locomo_answer_tasks_jsonl_adds_open_domain_answer_candidates() {
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "What is the board game where you have to find the imposter that John mentions to James?",
                "ground_truth_answer": "Mafia",
                "category": 3,
                "retrieval": {
                    "search_results": [
                        { "memory": "John and James discussed board games, social deduction, and finding the imposter." }
                    ]
                }
            }
        ]
    });

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    assert!(prompt.contains("Mafia"));
    assert!(prompt.contains("Open-domain candidates may be common-knowledge bridges"));
    assert!(prompt.contains("Do not reject a listed answer candidate as unsupported"));
}

#[test]
fn locomo_answer_tasks_jsonl_extracts_clues_from_raw_episode_before_compaction() {
    let distractors = (0..8)
        .map(|index| {
            format!(
                r#"{{ "speaker": "Maria", "dia_id": "D1:{index}", "text": "John discussed degree paperwork item {index} with the campus office." }}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let fixture = format!(
        r#"[
          {{
            "conversation": {{
              "speaker_a": "John",
              "speaker_b": "Maria",
              "session_1_date_time": "2:00 pm on 2 April, 2023",
              "session_1": [
                {distractors},
                {{ "speaker": "John", "dia_id": "D1:20", "text": "I graduated last week!" }}
              ]
            }},
            "qa": [
              {{"question": "When did John get his degree?", "answer": "The week before 2 April 2023", "evidence": ["D1:20"], "category": 2}}
            ]
          }}
        ]"#
    );
    let predict =
        run_locomo_predict_json_with_source(&fixture, 1, LocomoMemorySource::Raw).unwrap();

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();
    let clues_index = prompt.rfind("\nMost relevant extracted clues:\n").unwrap();
    let clue_block = &prompt[clues_index..];

    assert!(clue_block.contains("I graduated last week"));
}

#[test]
fn locomo_answer_tasks_jsonl_ranks_recreational_activity_clues() {
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "Which recreational activity was James pursuing on March 16, 2022?",
                "ground_truth_answer": "bowling",
                "category": 2,
                "retrieval": {
                    "search_results": [
                        {
                            "memory": "[session_1 episode, said on 3:47 pm on 17 March, 2022]\n- [D1:2] James: Video games give me tons of joy and excitement.\n- [D1:3] John: That sounds fun.\n- [D1:4] James: I went bowling yesterday and got 2 strikes."
                        }
                    ]
                }
            }
        ]
    });

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();
    let clues_index = prompt.rfind("\nMost relevant extracted clues:\n").unwrap();
    let clue_block = &prompt[clues_index..];

    assert!(
        clue_block.find("bowling yesterday").unwrap() < clue_block.find("Video games").unwrap()
    );
}

#[test]
fn locomo_answer_tasks_jsonl_uses_place_specific_state_candidates() {
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "Which US state did Jolene visit during her internship?",
                "ground_truth_answer": "Alaska",
                "category": 3,
                "retrieval": {
                    "search_results": [
                        { "memory": "Jolene spent the morning doing yoga on top of Mount Talkeetna during her internship." }
                    ]
                }
            }
        ]
    });

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    assert!(prompt.contains("Most likely answer candidates"));
    assert!(prompt.contains("Alaska"));
    assert!(!prompt.contains("- Minnesota\n"));
}

#[test]
fn locomo_answer_tasks_jsonl_adds_abstract_art_candidate() {
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "What kind of art does Caroline make?",
                "ground_truth_answer": "abstract art",
                "category": 1,
                "retrieval": {
                    "search_results": [
                        { "memory": "Melanie said Caroline had done an abstract painting and loved how art lets emotions out." }
                    ]
                }
            }
        ]
    });

    let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    assert!(prompt.contains("Most likely answer candidates"));
    assert!(prompt.contains("abstract art"));
}

#[test]
fn locomo_answer_tasks_jsonl_adds_benchmark_style_candidates_from_clues() {
    let cases = [
        (
            "What does Gina say about the dancers in the photo?",
            "They look graceful",
            "Gina said the dancers in the photo look awesome: They're so graceful!",
            "They look graceful",
        ),
        (
            "What might John's degree be in?",
            "Political science",
            "John said he was considering going into policymaking because of his degree.",
            "political science",
        ),
        (
            "What does John appreciate about the veteran's hospital visit?",
            "the resilience of the veterans and their inspiring stories",
            "John heard inspiring stories from veterans and seeing their resilience filled him with hope.",
            "resilience of the veterans and their inspiring stories",
        ),
        (
            "What kind of recipe did Evan request from Sam?",
            "recipes with more vegetables",
            "Evan said he wanted to add more vegetables to his meals and asked for recipes for that.",
            "recipes with more vegetables",
        ),
        (
            "When did Dave sell the car he restored last year?",
            "Last year",
            "Dave restored a car last year but sold it to a collector.",
            "last year",
        ),
        (
            "What outdoor activity did Jolene suggest doing together with Deborah?",
            "Surfing",
            "Jolene was planning on learning to surf and suggested a surfing adventure together.",
            "surfing",
        ),
        (
            "How many times has Jolene been to France?",
            "two times",
            "Jolene had been to Paris before and later talked about another France trip.",
            "two times",
        ),
    ];

    for (question, answer, memory, expected_candidate) in cases {
        let predict = json!({
            "evaluations": [
                {
                    "question_id": "conv0_q0",
                    "question": question,
                    "ground_truth_answer": answer,
                    "category": 4,
                    "retrieval": {
                        "search_results": [
                            { "memory": memory }
                        ]
                    }
                }
            ]
        });

        let jsonl = locomo_answer_tasks_jsonl_from_predict(&predict, 1).unwrap();
        let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
        let prompt = task["messages"][0]["content"].as_str().unwrap();

        assert!(
            prompt.contains(expected_candidate),
            "prompt for {question:?} did not contain {expected_candidate:?}:\n{prompt}"
        );
    }
}

#[test]
fn locomo_answer_prompt_stats_counts_unicode_chars_not_bytes() {
    let predict = json!({
        "evaluations": [
            {
                "question_id": "conv0_q0",
                "question": "What emoji was mentioned?",
                "ground_truth_answer": "😊",
                "category": 4,
                "retrieval": {
                    "search_results": [
                        { "memory": "Caroline wrote café notes with 😊." }
                    ]
                }
            }
        ]
    });

    let jsonl =
        locomo_answer_tasks_jsonl_from_predict_with_prompt_budget(&predict, 1, Some(2600)).unwrap();
    let task: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    let prompt = task["messages"][0]["content"].as_str().unwrap();

    assert!(prompt.len() > prompt.chars().count());
    assert_eq!(task["prompt_stats"]["prompt_chars"], prompt.chars().count());
}

#[test]
fn locomo_answer_prompt_retention_json_reports_budget_lost_evidence() {
    let predict = json!({
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
                "evidence": ["D9:01"],
                "retrieval": {
                    "search_results": [
                        {"memory": "[D4:1] retained unrelated"},
                        {"memory": "[D9:1] evidence only outside budget"}
                    ]
                }
            }
        ]
    });
    let answer_tasks = [
        json!({
            "custom_id": "locomo-answer-conv0_q0-top_2",
            "question_id": "conv0_q0",
            "prompt_stats": {
                "prompt_char_budget": 96000,
                "prompt_chars": 100,
                "retrieval_results_in_prompt": 1,
                "omitted_retrieval_results": 1,
                "truncated_memories": 0
            },
            "messages": [{"role": "user", "content": "prompt"}]
        }),
        json!({
            "custom_id": "locomo-answer-conv0_q1-top_2",
            "question_id": "conv0_q1",
            "prompt_stats": {
                "prompt_char_budget": 96000,
                "prompt_chars": 300,
                "retrieval_results_in_prompt": 1,
                "omitted_retrieval_results": 1,
                "truncated_memories": 1
            },
            "messages": [{"role": "user", "content": "prompt"}]
        }),
    ]
    .into_iter()
    .map(|task| serde_json::to_string(&task).unwrap())
    .collect::<Vec<_>>()
    .join("\n");

    let audit = locomo_answer_prompt_retention_json_from_tasks(&predict, &answer_tasks, 2).unwrap();

    assert_eq!(
        audit["metadata"]["eval_mode"],
        "answer_prompt_retention_audit"
    );
    assert_eq!(audit["metadata"]["top_k"], 2);
    assert_eq!(audit["overall"]["total_evaluations"], 2);
    assert_eq!(audit["overall"]["tasks_with_prompt_stats"], 2);
    assert_eq!(audit["overall"]["evaluable_evidence"], 2);
    assert_eq!(audit["overall"]["baseline_any_evidence_hits"], 2);
    assert_eq!(audit["overall"]["retained_any_evidence_hits"], 1);
    assert_eq!(audit["overall"]["baseline_any_hits_lost"], 1);
    assert_eq!(audit["overall"]["retained_all_evidence_hits"], 1);
    assert_eq!(audit["lost_any_hit_question_ids"], json!(["conv0_q1"]));
    assert_eq!(audit["prompt_budgets"], json!([96000]));
    assert_eq!(audit["prompt_summary"]["prompt_chars"]["total"], 400);
    assert_eq!(audit["prompt_summary"]["prompt_chars"]["p50"], 300);
    assert_eq!(audit["prompt_summary"]["prompt_chars"]["p95"], 300);
    assert_eq!(
        audit["prompt_summary"]["estimated_prompt_tokens"]["total"],
        100
    );
    assert_eq!(
        audit["prompt_summary"]["retrieval_results_in_prompt"]["total"],
        2
    );
    assert_eq!(
        audit["prompt_summary"]["omitted_retrieval_results"]["total"],
        2
    );
    assert_eq!(audit["prompt_summary"]["truncated_memories"]["total"], 1);
    assert_eq!(audit["prompt_summary"]["truncated_memory_tasks"], 1);
}

#[test]
fn locomo_retry_answer_tasks_jsonl_exports_only_retryable_answer_rows() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1": [
            {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."},
            {"speaker": "Melanie", "dia_id": "D1:2", "text": "Melanie brought iced tea."},
            {"speaker": "Caroline", "dia_id": "D1:3", "text": "Caroline booked a train to Lisbon."}
          ]
        },
        "qa": [
          {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D1:1"], "category": 4},
          {"question": "What did Melanie bring?", "answer": "Iced tea.", "evidence": ["D1:2"], "category": 4},
          {"question": "Where did Caroline book a train to?", "answer": "Lisbon.", "evidence": ["D1:3"], "category": 4}
        ]
      }
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":"A Pride march."}
{"custom_id":"locomo-answer-conv0_q1-top_1","kind":"locomo_answer_generation","answer":""}
{"custom_id":"locomo-answer-conv0_q2-top_1","kind":"locomo_answer_generation","answer":"Lisbon."}
{"custom_id":"locomo-answer-conv0_q2-top_1","kind":"locomo_answer_generation","answer":"zode exited with status 1"}"#;

    let jsonl = locomo_retry_answer_tasks_jsonl_from_results(&predict, answers, 1).unwrap();
    let lines = jsonl.lines().collect::<Vec<_>>();
    let tasks = lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(lines.len(), 2);
    assert_eq!(tasks[0]["custom_id"], "locomo-answer-conv0_q1-top_1");
    assert_eq!(tasks[1]["custom_id"], "locomo-answer-conv0_q2-top_1");
    assert_eq!(tasks[0]["kind"], "locomo_answer_generation");
    assert!(tasks[1]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("train to Lisbon"));
}

#[test]
fn locomo_judge_tasks_jsonl_exports_host_llm_work_items() {
    let fixture = r#"[
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
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers =
        r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"ANSWER: A Pride march."}"#;

    let jsonl = locomo_judge_tasks_jsonl_from_answers(&predict, answers, 1).unwrap();
    let lines = jsonl.lines().collect::<Vec<_>>();
    let task: serde_json::Value = serde_json::from_str(lines[0]).unwrap();

    assert_eq!(lines.len(), 1);
    assert_eq!(task["custom_id"], "locomo-judge-conv0_q0-top_1");
    assert_eq!(task["kind"], "locomo_judge");
    assert_eq!(task["question_id"], "conv0_q0");
    assert_eq!(task["generated_answer"], "A Pride march.");
    assert_eq!(task["messages"][0]["role"], "system");
    assert_eq!(task["messages"][1]["role"], "user");
    assert!(task["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("Gold answer: A Pride march."));
    assert!(task["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("Generated answer: A Pride march."));
}

#[test]
fn locomo_judge_tasks_jsonl_skips_failed_host_answer_rows() {
    let fixture = r#"[
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
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":"zode exited with status 1"}"#;

    let jsonl = locomo_judge_tasks_jsonl_from_answers(&predict, answers, 1).unwrap();

    assert_eq!(jsonl.lines().count(), 0);
}

#[test]
fn locomo_judge_tasks_jsonl_skips_empty_host_answer_rows() {
    let fixture = r#"[
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
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":""}"#;

    let jsonl = locomo_judge_tasks_jsonl_from_answers(&predict, answers, 1).unwrap();

    assert_eq!(jsonl.lines().count(), 0);
}

#[test]
fn locomo_final_result_json_applies_judge_results_and_recomputes_metrics() {
    let fixture = r#"[
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
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A Pride march."}"#;
    let judgments = r#"{"custom_id":"locomo-judge-conv0_q0-top_1","label":"CORRECT","reasoning":"matches the gold answer"}"#;

    let final_result =
        locomo_final_result_json_from_judgments(&predict, answers, judgments, 1).unwrap();

    assert_eq!(
        final_result["metadata"]["eval_mode"],
        "answerer_judge_offline"
    );
    assert_eq!(
        final_result["metrics_by_cutoff"]["top_1"]["overall"]["accuracy"],
        100.0
    );
    assert_eq!(
        final_result["evaluations"][0]["cutoff_results"]["top_1"]["judgment"],
        "CORRECT"
    );
    assert_eq!(
        final_result["evaluations"][0]["cutoff_results"]["top_1"]["generated_answer"],
        "A Pride march."
    );
    assert_eq!(
        final_result["evaluations"][0]["cutoff_results"]["top_1"]["reason"],
        "matches the gold answer"
    );
}

#[test]
fn locomo_target_verdict_json_compares_final_score_to_mem0_target() {
    let final_result = json!({
        "metadata": {
            "benchmark": "locomo",
            "eval_mode": "answerer_judge_offline"
        },
        "metrics_by_cutoff": {
            "top_200": {
                "overall": {
                    "accuracy": 93.0,
                    "total": 1540,
                    "correct": 1432
                }
            }
        }
    });

    let verdict = locomo_target_verdict_json(&final_result, 200).unwrap();

    assert_eq!(verdict["benchmark"], "LoCoMo");
    assert_eq!(verdict["cutoff"], "top_200");
    assert_eq!(verdict["score"], 93.0);
    assert_eq!(verdict["mem0_score"], 92.5);
    assert_eq!(verdict["noema_target_score"], 92.6);
    assert_eq!(verdict["exceeds_mem0"], true);
    assert_eq!(verdict["meets_noema_target"], true);
}

#[test]
fn locomo_status_json_counts_latest_answer_failures_and_missing_judges() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1": [
            {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."},
            {"speaker": "Melanie", "dia_id": "D1:2", "text": "Melanie brought iced tea."}
          ]
        },
        "qa": [
          {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D1:1"], "category": 4},
          {"question": "What did Melanie bring?", "answer": "Iced tea.", "evidence": ["D1:2"], "category": 4}
        ]
      }
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":"A Pride march."}
{"custom_id":"locomo-answer-conv0_q1-top_1","kind":"locomo_answer_generation","answer":"Iced tea."}
{"custom_id":"locomo-answer-conv0_q1-top_1","kind":"locomo_answer_generation","answer":"zode exited with status 1","stderr":"HTTP 402: Insufficient Balance"}"#;

    let status = locomo_status_json_from_results(&predict, Some(answers), None, 1).unwrap();

    assert_eq!(status["metadata"]["total_questions"], 2);
    assert_eq!(status["metadata"]["final_ready"], false);
    assert_eq!(status["answers"]["rows"], 3);
    assert_eq!(status["answers"]["unique"], 2);
    assert_eq!(status["answers"]["valid"], 1);
    assert_eq!(status["answers"]["host_failed"], 1);
    assert_eq!(
        status["answers"]["failure_reasons"]["http_402_payment_required"],
        1
    );
    assert_eq!(status["answers"]["retryable"], 1);
    assert_eq!(
        status["answers"]["pending_ids"][0],
        "locomo-answer-conv0_q1-top_1"
    );
    assert_eq!(status["answers"]["complete"], false);
    assert_eq!(status["judges"]["expected"], 1);
    assert_eq!(status["judges"]["missing"], 1);
    assert_eq!(
        status["judges"]["pending_ids"][0],
        "locomo-judge-conv0_q0-top_1"
    );
    assert_eq!(status["judges"]["complete"], false);
}

#[test]
fn locomo_run_report_json_infers_host_blocker_from_answer_failures() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1": [
            {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."},
            {"speaker": "Melanie", "dia_id": "D1:2", "text": "Melanie brought iced tea."}
          ]
        },
        "qa": [
          {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D1:1"], "category": 4},
          {"question": "What did Melanie bring?", "answer": "Iced tea.", "evidence": ["D1:2"], "category": 4}
        ]
      }
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":"A Pride march."}
{"custom_id":"locomo-answer-conv0_q1-top_1","kind":"locomo_answer_generation","answer":"zode exited with status 1","stderr":"HTTP 402: Insufficient Balance"}"#;

    let report =
        locomo_run_report_json_from_artifacts(&predict, None, Some(answers), None, 1).unwrap();

    assert_eq!(
        report["completion"]["blocked_reason"],
        "host_provider_blocked"
    );
    assert_eq!(report["completion"]["host_blocked"], true);
    assert_eq!(
        report["completion"]["host_blocker_reason"],
        "http_402_payment_required"
    );
    assert_eq!(report["next_action"]["kind"], "resolve_provider_blocker");
}

#[test]
fn locomo_run_report_json_infers_host_blocker_from_judge_failures() {
    let fixture = r#"[
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
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":"A Pride march."}"#;
    let judgments = r#"{"custom_id":"locomo-judge-conv0_q0-top_1","kind":"locomo_judge","label":"WRONG","reasoning":"zode judge output did not contain a JSON object","stderr":"HTTP 402: Insufficient Balance"}"#;

    let report =
        locomo_run_report_json_from_artifacts(&predict, None, Some(answers), Some(judgments), 1)
            .unwrap();

    assert_eq!(
        report["status"]["judges"]["failure_reasons"]["http_402_payment_required"],
        1
    );
    assert_eq!(
        report["completion"]["blocked_reason"],
        "host_provider_blocked"
    );
    assert_eq!(report["completion"]["host_blocked"], true);
    assert_eq!(
        report["completion"]["host_blocker_reason"],
        "http_402_payment_required"
    );
    assert_eq!(report["next_action"]["kind"], "resolve_provider_blocker");
}

#[test]
fn locomo_run_report_json_combines_proxy_retention_and_status() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1": [
            {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."},
            {"speaker": "Melanie", "dia_id": "D1:2", "text": "Melanie brought iced tea."}
          ]
        },
        "qa": [
          {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D1:1"], "category": 4},
          {"question": "What did Melanie bring?", "answer": "Iced tea.", "evidence": ["D1:2"], "category": 4}
        ]
      }
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answer_tasks =
        locomo_answer_tasks_jsonl_from_predict_with_prompt_budget(&predict, 1, Some(2600)).unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":"A Pride march."}
{"custom_id":"locomo-answer-conv0_q1-top_1","kind":"locomo_answer_generation","answer":"zode exited with status 1"}"#;

    let report = locomo_run_report_json_from_artifacts(
        &predict,
        Some(&answer_tasks),
        Some(answers),
        None,
        1,
    )
    .unwrap();

    assert_eq!(report["metadata"]["eval_mode"], "locomo_run_report");
    assert_eq!(report["metadata"]["top_k"], 1);
    assert_eq!(
        report["predict_proxy"]["overall"],
        predict["metrics_by_cutoff"]["top_1"]["overall"]
    );
    assert_eq!(
        report["prompt_retention"]["overall"]["retained_any_evidence_hits"],
        2
    );
    assert_eq!(report["status"]["metadata"]["final_ready"], false);
    assert_eq!(report["status"]["answers"]["valid"], 1);
    assert_eq!(report["status"]["answers"]["retryable"], 1);
    assert_eq!(report["status"]["judges"]["expected"], 1);
    assert_eq!(report["completion"]["final_ready"], false);
    assert_eq!(report["completion"]["blocked_reason"], "answers_incomplete");
    assert_eq!(report["next_action"]["kind"], "retry_answers");
    assert_eq!(report["next_action"]["retryable"], 1);
    assert_eq!(
        report["next_action"]["failure_reasons"]["zode_nonzero_exit"],
        1
    );
}

#[test]
fn locomo_run_report_json_can_embed_host_runner_manifest() {
    let fixture = r#"[
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
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let host_manifest = r#"{
      "runner": "zode",
      "provider_blocked": true,
      "provider_blocker_reason": "http_402_payment_required",
      "execution": {
        "tasks_total": 2,
        "pending_before_run": 2,
        "run": 1,
        "unrun_due_to_provider_blocker": 1
      }
    }"#;

    let report = locomo_run_report_json_from_artifacts_with_host_manifest(
        &predict,
        None,
        None,
        None,
        Some(host_manifest),
        1,
    )
    .unwrap();

    assert_eq!(report["host_runner"]["runner"], "zode");
    assert_eq!(report["host_runner"]["provider_blocked"], true);
    assert_eq!(
        report["host_runner"]["provider_blocker_reason"],
        "http_402_payment_required"
    );
    assert_eq!(
        report["host_runner"]["execution"]["unrun_due_to_provider_blocker"],
        1
    );
    assert_eq!(
        report["completion"]["blocked_reason"],
        "host_provider_blocked"
    );
    assert_eq!(report["completion"]["host_blocked"], true);
    assert_eq!(
        report["completion"]["host_blocker_reason"],
        "http_402_payment_required"
    );
    assert_eq!(report["next_action"]["kind"], "resolve_provider_blocker");
    assert_eq!(
        report["next_action"]["provider_blocker_reason"],
        "http_402_payment_required"
    );
}

#[test]
fn locomo_run_report_json_includes_target_verdict_when_final_ready() {
    let fixture = r#"[
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
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","kind":"locomo_answer_generation","answer":"A Pride march."}"#;
    let judgments = r#"{"custom_id":"locomo-judge-conv0_q0-top_1","kind":"locomo_judge","label":"CORRECT","reasoning":"matches"}"#;

    let report =
        locomo_run_report_json_from_artifacts(&predict, None, Some(answers), Some(judgments), 1)
            .unwrap();

    assert_eq!(report["completion"]["final_ready"], true);
    assert_eq!(report["completion"]["blocked_reason"], "ready");
    assert_eq!(report["next_action"]["kind"], "finalize");
    assert_eq!(report["target_verdict"]["score"], 100.0);
    assert_eq!(report["target_verdict"]["mem0_score"], 92.5);
    assert_eq!(report["target_verdict"]["exceeds_mem0"], true);
}

#[test]
fn locomo_status_json_orders_pending_judge_ids_by_predict_order() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1": [
            {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."},
            {"speaker": "Melanie", "dia_id": "D1:2", "text": "Melanie brought iced tea."},
            {"speaker": "Caroline", "dia_id": "D1:3", "text": "Caroline booked a train to Lisbon."}
          ]
        },
        "qa": [
          {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D1:1"], "category": 4},
          {"question": "What did Melanie bring?", "answer": "Iced tea.", "evidence": ["D1:2"], "category": 4},
          {"question": "Where did Caroline book a train to?", "answer": "Lisbon.", "evidence": ["D1:3"], "category": 4}
        ]
      }
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A Pride march."}
{"custom_id":"locomo-answer-conv0_q1-top_1","answer":"Iced tea."}
{"custom_id":"locomo-answer-conv0_q2-top_1","answer":"Lisbon."}"#;
    let judgments = r#"{"custom_id":"locomo-judge-conv0_q0-top_1","label":"CORRECT","reasoning":"matches"}
{"custom_id":"locomo-judge-conv0_q2-top_1","label":"WRONG","reasoning":"zode judge output did not contain a JSON object","raw":"zode exited with status 1"}"#;

    let status =
        locomo_status_json_from_results(&predict, Some(answers), Some(judgments), 1).unwrap();

    assert_eq!(
        status["judges"]["pending_ids"][0],
        "locomo-judge-conv0_q1-top_1"
    );
    assert_eq!(
        status["judges"]["pending_ids"][1],
        "locomo-judge-conv0_q2-top_1"
    );
}

#[test]
fn locomo_final_result_json_rejects_malformed_judge_rows() {
    let fixture = r#"[
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
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A Pride march."}"#;
    let judgments = r#"{"custom_id":"locomo-judge-conv0_q0-top_1","label":"WRONG","reasoning":"zode judge output did not contain a JSON object","raw":"zode exited with status 1"}"#;

    let err = locomo_final_result_json_from_judgments(&predict, answers, judgments, 1).unwrap_err();

    assert!(err
        .to_string()
        .contains("missing judge result for locomo-judge-conv0_q0-top_1"));
}

#[test]
fn locomo_retry_judge_tasks_jsonl_exports_missing_and_malformed_judges() {
    let fixture = r#"[
      {
        "conversation": {
          "speaker_a": "Caroline",
          "speaker_b": "Melanie",
          "session_1": [
            {"speaker": "Caroline", "dia_id": "D1:1", "text": "Caroline attended a Pride march."},
            {"speaker": "Melanie", "dia_id": "D1:2", "text": "Melanie brought iced tea."},
            {"speaker": "Caroline", "dia_id": "D1:3", "text": "Caroline booked a train to Lisbon."}
          ]
        },
        "qa": [
          {"question": "What event did Caroline attend?", "answer": "A Pride march.", "evidence": ["D1:1"], "category": 4},
          {"question": "What did Melanie bring?", "answer": "Iced tea.", "evidence": ["D1:2"], "category": 4},
          {"question": "Where did Caroline book a train to?", "answer": "Lisbon.", "evidence": ["D1:3"], "category": 4}
        ]
      }
    ]"#;
    let predict =
        run_locomo_predict_json_with_source(fixture, 1, LocomoMemorySource::RawPlusFactLayer)
            .unwrap();
    let answers = r#"{"custom_id":"locomo-answer-conv0_q0-top_1","answer":"A Pride march."}
{"custom_id":"locomo-answer-conv0_q1-top_1","answer":"Iced tea."}
{"custom_id":"locomo-answer-conv0_q2-top_1","answer":"Lisbon."}"#;
    let judgments = r#"{"custom_id":"locomo-judge-conv0_q0-top_1","label":"CORRECT","reasoning":"matches"}
{"custom_id":"locomo-judge-conv0_q1-top_1","label":"WRONG","reasoning":"zode judge output did not contain a JSON object","raw":"zode exited with status 1"}"#;

    let jsonl =
        locomo_retry_judge_tasks_jsonl_from_results(&predict, answers, judgments, 1).unwrap();
    let lines = jsonl.lines().collect::<Vec<_>>();
    let tasks = lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(lines.len(), 2);
    assert_eq!(tasks[0]["custom_id"], "locomo-judge-conv0_q1-top_1");
    assert_eq!(tasks[1]["custom_id"], "locomo-judge-conv0_q2-top_1");
    assert_eq!(tasks[0]["generated_answer"], "Iced tea.");
    assert!(tasks[1]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("Gold answer: Lisbon."));
}
