#![allow(dead_code)]

use std::time::{Duration, Instant};

use crate::api::{
    NoemaEngine, RecallRequest, RememberRequest, ReviewAction, ReviewDecisionRequest,
};
use crate::error::Result;
use crate::ids::UserId;
use crate::memory::{MemoryKind, Scope};
use crate::sensitivity::{Principal, SensitivityLevel};

use super::{
    BenchmarkPhaseSample, BenchmarkReport, BenchmarkSample, BenchmarkScenario, PhaseAccumulator,
    PhaseMeasurement,
};

pub fn run_recall_benchmark(
    root: &std::path::Path,
    scenario: BenchmarkScenario,
) -> Result<BenchmarkReport> {
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
