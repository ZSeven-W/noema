# Noema

Noema is a local-first, non-vector memory system for coding agents.

It stores durable agent memory as inspectable files, keeps new memories behind a
review queue, recalls context with lexical signals instead of embeddings, and is
designed to work across hosts such as zode, Codex, Claude Code, and MCP-capable
agent runtimes.

> Status: early implementation. The P0-P6 architecture is represented in this
> repository, but public APIs, file formats, and integration contracts may still
> change before a stable release.

## Why Noema

Most agent memory systems are built around vector databases. That is useful for
semantic search, but it can make memory opaque, hard to audit, hard to delete,
and difficult to run safely in enterprise environments.

Noema takes a different path:

- **No vectors or embeddings**: recall is lexical, explainable, and reproducible.
- **Local-first storage**: the default root is `~/.agent-memory/`, with an
  optional `NOEMA_ROOT` override for tests and isolated workspaces.
- **Human review by default**: new candidates enter a hippocampus-style queue
  before becoming durable cortex memory.
- **Agent-neutral protocol**: the same memory store can be used by the CLI,
  zode, MCP clients, and future host adapters.
- **Enterprise-aware from the start**: tenant boundaries, sensitivity levels,
  audit events, ACL policy hooks, KMS metadata policy, and S3-compatible cold
  offload are part of the design rather than later patches.

## Features

- Tenant-scoped file layout under `~/.agent-memory/tenants/<tenant>/`.
- Markdown memory records with structured JSON frontmatter.
- Event-sourced review queue using `hippocampus/inbox.jsonl` and
  `hippocampus/decisions.jsonl`.
- Candidate decisions: accept, reject, edit, and merge.
- Lexical recall that returns a `MemoryPack` with scores and source metadata.
- `explain` support for understanding why a memory matched a query.
- Automatic rejection of `secret` candidates before review.
- Personal-mode sensitivity cap: `public` and `internal` only.
- Payload-free audit events for review and memory lifecycle operations.
- Vacuum support for compacting review history.
- Forget/tombstone flow with restore-time deletion manifest checks.
- S3-compatible cold offload scaffolding with local fake-store tests.
- MCP stdio tool surface.
- Minimal enterprise server boundary for signed principals and policy checks.

## Repository Layout

```text
.
+-- crates/
|   +-- noema-core/      # protocol types, paths, storage, recall, review, policy
|   +-- noema-cli/       # `noema` command-line interface
|   +-- noema-mcp/       # MCP stdio tool surface
|   +-- noema-server/    # enterprise boundary and status service
+-- tests/               # CLI, MCP, enterprise, and offload contract tests
+-- Cargo.toml
+-- README.md
```

## Install From Source

Noema is currently distributed from source.

Requirements:

- Rust stable toolchain
- Git

```bash
git clone https://github.com/ZSeven-W/noema.git
cd noema
cargo build --workspace
```

Run the CLI from the workspace:

```bash
cargo run -p noema -- --help
```

Install it into your Cargo bin directory:

```bash
cargo install --path crates/noema-cli
```

## Quick Start

Use a temporary root while trying Noema:

```bash
export NOEMA_ROOT="$(mktemp -d)"

cargo run -p noema -- init
cargo run -p noema -- remember "Prefer Rust for Noema implementation work."
cargo run -p noema -- review
```

If `remember` auto-accepts the candidate, it will print an accepted memory id.
If it queues the candidate, accept it from the review queue:

```bash
cargo run -p noema -- accept cand_xxxxx
```

For explicit host-agent memory writes that should be available immediately:

```bash
cargo run -p noema -- remember "Prefer Rust for Noema implementation work." --accept
```

Search and inspect recall:

```bash
cargo run -p noema -- recall "Rust implementation"
cargo run -p noema -- search "Rust implementation"
cargo run -p noema -- explain mem_xxxxx --query "Rust implementation"
```

For normal use, omit `NOEMA_ROOT` and Noema will use:

```text
~/.agent-memory/
```

## CLI Overview

| Command | Purpose |
| --- | --- |
| `noema init` | Create the local memory layout and config. |
| `noema remember <text>` | Create a memory candidate. |
| `noema remember <text> --accept` | Persist an explicit memory immediately. |
| `noema review` | List pending candidates. |
| `noema edit <candidate> --body ... --reason ...` | Revise a pending candidate before acceptance. |
| `noema merge <candidate> <memory> --reason ...` | Drop a duplicate candidate in favor of an existing memory. |
| `noema accept <candidate>` | Persist a pending candidate as memory. |
| `noema reject <candidate> --reason ...` | Reject a pending candidate. |
| `noema recall <query>` | Print a markdown memory pack with full memory text. |
| `noema search <query>` | Search matching memory ids and scores. |
| `noema explain <memory> --query ...` | Explain a recall score. |
| `noema vacuum` | Compact terminal review events into snapshots and archives. |
| `noema sleep [--llm]` | Move stale or low-utility memory toward deeper storage. |
| `noema bench` | Run a synthetic recall benchmark and print README-ready tables. |
| `noema bench --mem0-targets` | Print the Mem0 benchmark scores Noema must exceed. |
| `noema bench --mem0-result <file>` | Summarize a Mem0 benchmark result JSON file. |
| `noema bench --locomo-dataset <file>` | Summarize a LOCOMO dataset file. |
| `noema bench --locomo-evidence <file>` | Run Noema LOCOMO evidence retrieval. Use `--locomo-memory-source raw\|observation\|raw-plus-observation\|fact-layer\|raw-plus-fact-layer` to compare layers. |
| `noema bench --locomo-evidence <file> --locomo-predict-output <file>` | Write a mem0-like predict/proxy JSON file with retrieval contexts and evidence-hit cutoff results. |
| `noema bench --locomo-predict-input <file>` | Continue from an existing LOCOMO predict JSON without recomputing retrieval. |
| `noema bench --locomo-evidence <file> --locomo-answer-tasks-output <file>` | Write host-LLM answer-generation tasks from the same top-k LOCOMO retrieval context. |
| `noema bench --locomo-predict-input <file> --locomo-retry-answer-tasks-output <file>` | Write only missing, empty, or host-failed LOCOMO answer tasks. |
| `noema bench --locomo-predict-input <file> --locomo-answer-tasks-input <file> --locomo-retention-output <file>` | Audit budgeted answer-task prompt retention against the full predict retrieval context. |
| `noema bench --locomo-evidence <file> --locomo-judge-tasks-output <file>` | Convert answer results into judge tasks for LOCOMO correctness grading. |
| `noema bench --locomo-predict-input <file> --locomo-retry-judge-tasks-output <file>` | Write only missing, malformed, or host-failed LOCOMO judge tasks. |
| `noema bench --locomo-evidence <file> --locomo-final-output <file>` | Combine predict, answer, and judge results into final LOCOMO metrics. |
| `noema bench --locomo-predict-input <file> --locomo-target-output <file>` | Compare a judged LOCOMO final result against the Mem0 LoCoMo target. |
| `noema bench --locomo-predict-input <file> --locomo-require-beats-mem0` | Exit non-zero unless the judged LOCOMO score exceeds Mem0's LoCoMo score. |
| `noema bench --locomo-predict-input <file> --locomo-status-output <file>` | Summarize answer/judge completeness for an offline LOCOMO run. |
| `noema bench --locomo-predict-input <file> --locomo-report-output <file>` | Combine proxy, prompt retention, and answer/judge readiness into one run report. |
| `noema bench --locomo-predict-input <file> --locomo-host-manifest-input <file> --locomo-report-output <file>` | Embed zode host-run provenance in the LOCOMO run report. |
| `noema bench --locomo-predict-input <file> --locomo-fail-if-incomplete` | Exit non-zero until the LOCOMO answer and judge artifacts are ready for final scoring. |
| `noema forget <memory> [--hard]` | Tombstone or hard-erase a memory. |
| `noema offload status` | Show cold-offload status. |
| `noema offload run` | Run cold-offload processing. |
| `noema restore <snapshot-or-id>` | Restore while applying deletion manifests. |
| `noema doctor` | Check local store health. |
| `noema reindex` | Rebuild local indexes. |

## Storage Model

Noema keeps storage boring on purpose. The hot path is made of directories,
Markdown files, JSONL event logs, and lock files that are easy to inspect and
back up.

```text
~/.agent-memory/
+-- config.toml
+-- manifests/
+-- tenants/
    +-- personal/
        +-- tenant.lock
        +-- audit.jsonl
        +-- users/
        |   +-- <user-id>/
        |       +-- memories/
        +-- projects/
        |   +-- <project-id>/
        |       +-- memories/
        +-- hippocampus/
        |   +-- inbox.jsonl
        |   +-- decisions.jsonl
        |   +-- snapshots/
        |   +-- archive/
        +-- indexes/
        +-- trash/
        +-- cold/
```

The default personal tenant is intentionally simple. Enterprise deployments can
add signed principals, stronger tenant boundaries, ACL policy, and external cold
storage without changing the local memory protocol.

## Memory Lifecycle

```text
candidate
  -> pending review
  -> accepted memory
  -> recalled cortex memory
  -> deep memory when stale
  -> resurrected for one request when relevant
  -> deep again unless confirmed
  -> forgotten or hard-erased when required
```

Noema separates capture from persistence. Hosts can submit candidates
automatically, but the review queue remains the standard control point. A
configuration can make review more automatic, while sensitivity ceilings and
secret rejection still apply.

## Security And Privacy

Noema treats memory as potentially sensitive infrastructure.

- `secret` candidates are rejected before they enter review.
- Personal mode rejects `confidential` and `restricted` writes, so users do not
  create memory they cannot recall.
- Recall filters memory above the principal's clearance.
- Audit events avoid storing raw memory payloads.
- Redacted or summary-only recall requires precomputed safe variants; Noema does
  not rely on runtime prose redaction for sensitive data.
- Sensitive full-text indexing is isolated from normal turn recall.
- Hard erasure must cascade across hot memory, deep memory, local stubs, indexes,
  cold payload metadata, and restore deletion manifests.
- Restores apply deletion manifests so old snapshots do not silently resurrect
  erased records.

These controls are intentionally conservative. The personal CLI is useful on its
own, but the file protocol and policy model are built for multi-tenant
enterprise use.

## Integrations

### CLI

`noema` is the reference interface and the best place to inspect behavior while
the project is early.

For host agents that do not use MCP, install the CLI and the bundled skill:

```bash
cargo install --path crates/noema-cli
cp -R skills/noema-memory "${CODEX_HOME:-$HOME/.codex}/skills/"
```

The skill uses:

```bash
noema recall "<user request>"
noema remember "<stable fact or preference>" --accept
```

`recall` prints a markdown memory pack with full memory text for model context.
`remember --accept` is for explicit host-agent memory writes; plain
`remember` still follows the review queue policy.

### MCP

`noema-mcp` exposes a stdio JSON-RPC surface for MCP-capable hosts:

```bash
cargo install --path crates/noema-mcp
noema-mcp
```

Generic JSON MCP clients can point at the stdio server:

```json
{
  "mcpServers": {
    "noema": {
      "command": "noema-mcp"
    }
  }
}
```

Codex-style TOML MCP config can use the same command:

```toml
[mcp_servers.noema]
command = "noema-mcp"
```

The MCP contract is tested end-to-end: tool listing, `noema_remember`, and
`noema_recall` must round-trip through an isolated Noema root.

### zode

Noema is designed to be consumed by zode as a native memory backend while
remaining a standalone repository. zode can depend on it through a submodule,
workspace dependency, or package release without making memory storage
zode-specific.

### Enterprise Server

`noema-server` contains the beginning of the enterprise trust boundary: signed
principal claims, tenant-aware policy checks, and service status endpoints.

## S3-Compatible Cold Offload

Noema's primary store is local, but cold memory can be offloaded to an
S3-compatible backend to control disk growth. The cold-offload design keeps local
metadata and restore safeguards, including deletion manifests for erased records.

Current implementation includes the storage abstraction and fake filesystem
backend used by tests. Production S3 wiring should be treated as an integration
target while APIs are still unstable.

## Benchmarks

### Mem0 Benchmark Targets

Noema is being built against the public
[mem0ai/memory-benchmarks](https://github.com/mem0ai/memory-benchmarks/tree/4b61c5d31b9c668a12b4f5e78064248a02c82d2b)
suite at commit `4b61c5d31b9c668a12b4f5e78064248a02c82d2b`.

That suite evaluates memory systems with an `Ingest -> Search -> Evaluate`
pipeline across LOCOMO, LongMemEval, and BEAM. The target is not to reproduce
Mem0's architecture; Noema intentionally stays local-first and non-vector. The
target is to beat the published scores while keeping Noema's auditability,
deletion, tenant isolation, and sensitivity controls intact.

Current external reference scores:

```bash
cargo run -p noema -- bench --mem0-targets
```

| Benchmark | Metric | Mem0 score | Noema must exceed | Notes |
| --- | --- | ---: | ---: | --- |
| LoCoMo | overall score | 92.5 | > 92.5 | multi-session dialogue memory |
| LongMemEval | overall score | 94.4 | > 94.4 | long-term memory questions |
| BEAM 1M | average score | 64.1 | > 64.1 | 1M-token memory ability benchmark |
| BEAM 10M | average score | 48.6 | > 48.6 | 10M-token memory ability benchmark |

These are external Mem0 baselines. Current judged Noema results:

| Benchmark | Cutoff | Runner | Noema score | Mem0 score | Margin | Correct / total | Status |
| --- | --- | --- | ---: | ---: | ---: | ---: | --- |
| LoCoMo | top_200 | zode host runner, v7 96k prompts | 92.6623 | 92.5 | +0.1623 | 1427 / 1540 | exceeds Mem0 and Noema target |
| LongMemEval | - | - | - | 94.4 | - | - | not yet run |
| BEAM 1M | - | - | - | 64.1 | - | - | not yet run |
| BEAM 10M | - | - | - | 48.6 | - | - | not yet run |

The LoCoMo run used the Mem0 benchmark data at commit
`4b61c5d31b9c668a12b4f5e78064248a02c82d2b`, Noema's
`raw-plus-fact-layer` top-200 retrieval export, a 96k-character answer prompt
budget, and zode as the offline answerer/judge host runner. Noema is considered
competitive for LoCoMo top-200 only; LongMemEval and BEAM still need comparable
runs before the broader benchmark goal is complete.

Noema can also summarize checked-in Mem0 result JSON files from that repository:

```bash
cargo run -p noema -- bench --mem0-result results/platform/beam_1m_results.json
```

Example summary for the Mem0 BEAM 1M result at the referenced commit:

| Cutoff | Metric | Score | Total |
| --- | --- | ---: | ---: |
| top_200 | avg_score | 64.1 | 700 |

Noema's benchmark adapter should map the mem0 suite as follows:

- `Ingest`: convert benchmark conversations into Noema memory candidates and
  durable memory records, preserving user/session/time metadata.
- `Search`: run Noema lexical recall at the same top-k cutoffs used by the mem0
  suite, especially top 50 and top 200.
- `Evaluate`: support predict-only retrieval first, then answerer/judge mode
  through a host LLM so Noema can report comparable accuracy/pass-rate scores.
- `Report`: group results by LOCOMO category, LongMemEval question type, and
  BEAM memory ability type.

Current LOCOMO adapter status:

```bash
cargo run -p noema -- bench --locomo-dataset /tmp/locomo10.json
```

| Dataset | Conversations | Sessions | Turns | Questions | Evaluable questions |
| --- | ---: | ---: | ---: | ---: | ---: |
| LOCOMO-10 | 10 | 272 | 5882 | 1986 | 1540 |

LOCOMO category counts for the mem0-evaluated set:

| Category | Questions |
| --- | ---: |
| multi-hop | 282 |
| open-domain | 96 |
| single-hop | 841 |
| temporal | 321 |

Noema can now run an evidence-retrieval proxy over LOCOMO. The raw mode uses
original conversation turns with previous/next turn context plus session-level
hippocampal episode memories; the fact-layer mode adds observation-derived fact
memories plus speaker-level summary memories with provenance back to the source
`dia_id`s.

```bash
cargo run -p noema -- bench --locomo-evidence /tmp/locomo10.json --top-k 200 --locomo-memory-source raw-plus-fact-layer
```

Current proxy result:

| Source | Top K | Any evidence hit | All evidence hit | Questions with resolved evidence |
| --- | ---: | ---: | ---: | ---: |
| raw | 50 | 99.7% | 97.4% | 1536 |
| raw | 200 | 99.9% | 99.9% | 1536 |
| raw-plus-fact-layer | 50 | 99.9% | 99.0% | 1536 |
| raw-plus-fact-layer | 200 | 100.0% | 99.9% | 1536 |

Noema can also export a mem0-like predict/proxy JSON file for the next
answerer/judge phase, plus a per-question directory that matches mem0
`--evaluate-only` expectations:

```bash
cargo run -p noema -- bench \
  --locomo-evidence /tmp/locomo10.json \
  --top-k 200 \
  --locomo-memory-source raw-plus-fact-layer \
  --locomo-predict-output /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --locomo-predict-dir /tmp/noema-locomo-mem0-predict-top200 \
  --locomo-answer-prompt-char-budget 96000 \
  --locomo-answer-tasks-output /tmp/noema-locomo-answer-tasks-top200-budget96k.jsonl
```

The current top-200 unified export is about 323 MB and contains all 1540 mem0
evaluated category 1-4 questions. Questions whose evidence IDs cannot be mapped
to a Noema memory are kept in the export and counted as proxy misses, so mem0
`--evaluate-only` can run without missing per-question files.
LOCOMO evidence fields that pack multiple `dia_id`s into one semicolon- or
space-separated string are split before scoring and task export, and leading
zero forms such as `D30:05` are normalized to the matching turn id.

| Artifact | Source | Top K | Proxy score | Correct / total |
| --- | --- | ---: | ---: | ---: |
| `/tmp/noema-locomo-raw-plus-fact-layer-top200.json` | raw-plus-fact-layer | 200 | 99.7% | 1536 / 1540 |
| `/tmp/noema-locomo-mem0-predict-top200/` | raw-plus-fact-layer | 200 | 99.7% | 1540 question files + summary |
| `/tmp/noema-locomo-answer-tasks-top200-budget96k.jsonl` | raw-plus-fact-layer | 200 | answerer input | 1540 tasks |

The unbudgeted answer-task JSONL is about 201 MB after query-aware episode
compaction. With `--locomo-answer-prompt-char-budget 96000`, the top-200
answer-task JSONL is about 148 MB and keeps the answerer prompt under roughly
96k characters per question without changing the predict/evidence-proxy export.
Each line contains a `custom_id`, the
LOCOMO question metadata, and a host-LLM `messages` payload built from the same
top-k retrieval context. It is intended for zode, Codex, Claude Code, or an
external batch runner to generate answers before a judge pass computes the final
LOCOMO accuracy.

Prompt budgets are a cost/retention tradeoff. Noema can audit a budgeted answer
task file against the full predict JSON:

```bash
cargo run -p noema -- bench \
  --locomo-predict-input /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --top-k 200 \
  --locomo-answer-tasks-input /tmp/noema-locomo-answer-tasks-top200-budget96k.jsonl \
  --locomo-retention-output /tmp/noema-locomo-answer-retention-top200-budget96k.json
```

The conservative evidence-ID audit maps each budgeted task back to the first
`retrieval_results_in_prompt` records and checks whether LOCOMO `D...` evidence
IDs still appear in those retained memories. It is stricter than the evidence
proxy because fact-layer summary hits do not always carry raw `D...` IDs. The
same retention JSON includes `prompt_summary`, so prompt character totals,
estimated prompt tokens, retained retrieval counts, omitted retrieval counts,
and truncation counts can be reproduced without a separate zode dry-run.

| Answer prompt budget | Task file bytes | Estimated prompt tokens | Avg retained retrievals | ID any-hit / evaluable | ID all-hit / evaluable | Baseline any/all lost |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 48k chars | 72,455,091 | 17,805,319 | 32.8 | 1515 / 1536 | 1440 / 1536 | 19 / 91 |
| 64k chars | 98,329,446 | 24,239,274 | 58.2 | 1529 / 1536 | 1487 / 1536 | 5 / 44 |
| 96k chars, v7 | 148,381,415 | 36,611,760 | 155.3 | 1534 / 1536 | 1529 / 1536 | 0 / 2 |

With zode as the host LLM runner:

```bash
python3 /Users/kayshen/Workspace/ZSeven-W/zode/benchmarks/noema_locomo.py \
  --tasks /tmp/noema-locomo-answer-tasks-top200-budget96k.jsonl \
  --output /tmp/noema-locomo-answer-results.jsonl \
  --zode-config-dir ~/.zode \
  --jobs 1 \
  --resume \
  --retry-empty \
  --retry-failed \
  --summary-output /tmp/noema-locomo-answer-results.summary.json \
  --manifest-output /tmp/noema-locomo-answer-run.manifest.json \
  --stop-on-provider-blocker \
  --retries 2
```

For long runs, Noema can also emit a compact retry-only answer task file from
the current append-only result JSONL:

```bash
cargo run -p noema -- bench \
  --locomo-predict-input /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --top-k 200 \
  --locomo-answer-prompt-char-budget 96000 \
  --locomo-answer-results /tmp/noema-locomo-answer-results.jsonl \
  --locomo-retry-answer-tasks-output /tmp/noema-locomo-answer-tasks-retry-top200-budget96k.jsonl
```

That retry file preserves the original `custom_id`s and contains only missing,
empty, or host-failed answers, so zode can append recovered results back into
the same answer-results file. In the final v7 96k run, the first full answer
pass produced 16 retryable rows; a resume pass with
`--retry-empty --retry-failed` recovered all of them, leaving 1540 valid answers
and zero retryable answer rows.

Noema can also combine the current proxy, prompt-retention audit, and
answer/judge readiness into a single run report:

```bash
cargo run -p noema -- bench \
  --locomo-predict-input /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --top-k 200 \
  --locomo-answer-tasks-input /tmp/noema-locomo-answer-tasks-top200-budget96k.jsonl \
  --locomo-answer-results /tmp/noema-locomo-answer-results-zode-top200.jsonl \
  --locomo-judge-results /tmp/noema-locomo-judge-results-empty.jsonl \
  --locomo-host-manifest-input /tmp/noema-locomo-answer-run.manifest.json \
  --locomo-report-output /tmp/noema-locomo-run-report-top200-budget96k.json
```

The current v7 96k full report records
`predict_proxy.overall.accuracy=99.74025974025976`,
`prompt_retention.overall.retained_any_evidence_hits=1534`,
`prompt_retention.overall.retained_all_evidence_hits=1529`,
`completion.final_ready=true`, `completion.blocked_reason=ready`, and a
`target_verdict` with `score=92.66233766233766`,
`correct=1427`, `total=1540`, `exceeds_mem0=true`, and
`meets_noema_target=true`.
When `--locomo-host-manifest-input` is provided, the report embeds the zode
runner manifest under `host_runner`, adding provider-run provenance such as
`unrun_due_to_provider_blocker`.

After answer generation, Noema can convert answer results into judge-task JSONL:

```bash
cargo run -p noema -- bench \
  --locomo-predict-input /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --top-k 200 \
  --locomo-answer-results /tmp/noema-locomo-answer-results.jsonl \
  --locomo-judge-tasks-output /tmp/noema-locomo-judge-tasks-top200.jsonl
```

Answer result lines can use `{ "custom_id": "...", "answer": "..." }`,
`generated_answer`, or a common batch response shape with
`response.body.choices[0].message.content`. The generated judge tasks ask for a
JSON `{ "reasoning": "...", "label": "CORRECT|WRONG" }` response and preserve
the original `question_id`/cutoff in `custom_id`.

Run the judge tasks through zode the same way:

```bash
python3 /Users/kayshen/Workspace/ZSeven-W/zode/benchmarks/noema_locomo.py \
  --tasks /tmp/noema-locomo-judge-tasks-top200.jsonl \
  --output /tmp/noema-locomo-judge-results.jsonl \
  --zode-config-dir ~/.zode \
  --jobs 1 \
  --resume \
  --retry-empty \
  --retry-failed \
  --summary-output /tmp/noema-locomo-judge-results.summary.json \
  --manifest-output /tmp/noema-locomo-judge-run.manifest.json \
  --stop-on-provider-blocker \
  --retries 2
```

The zode runner summary is a quick host-side count of latest answer/judge rows.
It can inspect a task file without an output file or host LLM call:

```bash
python3 /Users/kayshen/Workspace/ZSeven-W/zode/benchmarks/noema_locomo.py \
  --dry-run \
  --tasks /tmp/noema-locomo-answer-tasks-top200-budget96k.jsonl \
  --summary-output /tmp/noema-locomo-answer-dry-run.summary.json \
  --manifest-output /tmp/noema-locomo-answer-dry-run.manifest.json
```

It can also summarize existing result files without a task file or host LLM call:

```bash
python3 /Users/kayshen/Workspace/ZSeven-W/zode/benchmarks/noema_locomo.py \
  --summary-only \
  --fail-on-retryable \
  --output /tmp/noema-locomo-answer-results.jsonl \
  --summary-output /tmp/noema-locomo-answer-results.summary.json \
  --manifest-output /tmp/noema-locomo-answer-summary.manifest.json
```

With `--fail-on-retryable`, the runner exits with status 2 when latest results
still contain ordinary retryable answer or judge rows; if those retryables include
a provider blocker such as `http_402_payment_required`, summary-only exits with
status 3.
For provider-balance or billing-sensitive retry batches, run the zode runner
with `--jobs 1 --stop-on-provider-blocker`; it stops at the first blocker such
as `http_402_payment_required`, writes the partial JSONL and manifest, and exits
with status 3.

Noema can gate the full offline benchmark state across predict, answer, and
judge artifacts:

```bash
cargo run -p noema -- bench \
  --locomo-predict-input /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --top-k 200 \
  --locomo-answer-tasks-input /tmp/noema-locomo-answer-tasks-top200-budget96k.jsonl \
  --locomo-answer-results /tmp/noema-locomo-answer-results-zode-top200.jsonl \
  --locomo-judge-results /tmp/noema-locomo-judge-results-empty.jsonl \
  --locomo-host-manifest-input /tmp/noema-locomo-answer-summary-zode-top200.manifest.json \
  --locomo-fail-if-incomplete
```

Before answer and judge artifacts are complete, the gate returns an error such as
`LOCOMO run incomplete: blocked_reason=host_provider_blocked
next_action=resolve_provider_blocker retryable=0
provider_blocker_reason=http_402_payment_required` while a provider balance
blocker is unresolved. Once answer and judge artifacts are complete, the same
gate prints that the run is ready for final scoring.

`--manifest-output` records the host-run provenance needed for reproducible
benchmark reports: task/result paths, zode binary, provider/model names, retry
settings, run/skipped counts, `tasks_total`, `pending_before_run`,
`unrun_due_to_provider_blocker`, task input size stats, prompt character
distribution, a rough `ceil(prompt_chars / 4)` prompt-token estimate, the same
latest-row summary, provider-blocker status, and answer failure-reason buckets
such as `http_402_payment_required`. It intentionally does not include provider
API keys.
Use `--zode-config-dir` to run against an isolated zode config directory instead
of whichever global config is active in the shell.

The current v7 96k-budget answer run manifest records
`task_file_bytes=148381415`, `tasks_loaded=1540`, and prompt chars
`p50=95738`, `p95=95959`, `max=96000`. It estimates prompt input at `36611760`
tokens total, with `p50=23935`, `p95=23990`, and `max=24000` per task using the
runner's 4 chars/token estimate. It also records Noema prompt-retention stats:
`retrieval_results_in_prompt.mean=155.3422077922078`, `p95=192`, and
`truncated_memories.total=0`. For comparison, the unbudgeted compacted top-200
task file records `task_file_bytes=200747951`, prompt chars `p50=130719`,
`p95=147523`, `max=162186`, and about `49778930` estimated prompt tokens total.
The current full judge-task artifact is
`/tmp/noema-locomo-judge-tasks-zode-top200-v7-96k-full.jsonl`; its zode
manifest records `task_file_bytes=2608433`, `tasks_loaded=1540`, prompt chars
`total=1683047`, `p50=1070`, `p95=1323`, `max=2110`, and estimated prompt input
tokens `total=421327`, `p50=268`, `p95=331`, `max=528`.

Use Noema's status JSON below as the authoritative readiness check because it
also knows the predict set and expected task ids.

If a judge run is interrupted or returns malformed host output, emit retry-only
judge tasks from the current answer and judge results:

```bash
cargo run -p noema -- bench \
  --locomo-predict-input /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --top-k 200 \
  --locomo-answer-results /tmp/noema-locomo-answer-results.jsonl \
  --locomo-judge-results /tmp/noema-locomo-judge-results.jsonl \
  --locomo-retry-judge-tasks-output /tmp/noema-locomo-judge-tasks-retry-top200.jsonl
```

At any point, write a benchmark-status JSON report from the current predict,
answer, and optional judge outputs:

```bash
cargo run -p noema -- bench \
  --locomo-predict-input /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --top-k 200 \
  --locomo-answer-results /tmp/noema-locomo-answer-results.jsonl \
  --locomo-status-output /tmp/noema-locomo-status-top200.json
```

The status report uses append-only/latest-row semantics for result JSONL, matching
the zode runner's `--resume` behavior. It counts valid answers, empty answers,
host failures, missing answers, valid judge labels, malformed/host-failed judge
rows, and whether the run is ready for final scoring. Add
`--locomo-judge-results <file>` once judge results exist.

After the judge pass finishes, Noema can recompute the final LOCOMO
`metrics_by_cutoff` from the judge labels:

```bash
cargo run -p noema -- bench \
  --locomo-predict-input /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --top-k 200 \
  --locomo-answer-results /tmp/noema-locomo-answer-results.jsonl \
  --locomo-judge-results /tmp/noema-locomo-judge-results.jsonl \
  --locomo-final-output /tmp/noema-locomo-final-results-top200.json
```

The final-output step requires complete answer and judge results for every
exported evaluation. It preserves the original retrieval context, replaces the
proxy cutoff result with `CORRECT`/`WRONG` judge labels, and marks the output as
`eval_mode = "answerer_judge_offline"`.

When the judged result is ready, Noema can write and enforce the Mem0 target
verdict:

```bash
cargo run -p noema -- bench \
  --locomo-predict-input /tmp/noema-locomo-raw-plus-fact-layer-top200.json \
  --top-k 200 \
  --locomo-answer-results /tmp/noema-locomo-answer-results.jsonl \
  --locomo-judge-results /tmp/noema-locomo-judge-results.jsonl \
  --locomo-target-output /tmp/noema-locomo-target-verdict-top200.json \
  --locomo-require-beats-mem0
```

The verdict records the judged score, Mem0's LoCoMo target (`92.5`),
Noema's strict target (`92.6`), and booleans for `exceeds_mem0` and
`meets_noema_target`. The combined run report embeds the same verdict under
`target_verdict` whenever `completion.final_ready=true`.

Current zode-hosted full run status for the top-200 export:

| Artifact | Total | Valid answers | Valid judge labels | Correct | Wrong | Retryable answers | Retryable judges | Final ready |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `/tmp/noema-locomo-target-verdict-zode-top200-v7-96k-full.json` | 1540 | 1540 | 1540 | 1427 | 113 | 0 | 0 | yes |

The result files are append-only/latest-row JSONL. The final answer summary has
`rows=1556` because it includes the initial 16 retryable answer rows plus the
successful resume rows, but `unique=1540`, `answers.valid=1540`, and
`answers.retryable=0`. The judge summary has `rows=1540`, `unique=1540`,
`judges.correct=1427`, `judges.wrong=113`, and `judges.retryable=0`.

The `raw-plus-fact-layer` evidence proxy remains useful for retrieval debugging,
but the reported LoCoMo score above is the final answerer/judge offline score
for the exported 1540 questions. It is directly compared against the Mem0
LoCoMo top-200 reference score of 92.5 from the referenced benchmark commit.

### Local Recall Latency

Noema includes a small synthetic benchmark runner for recall-path tracking:

```bash
cargo run --release -p noema -- bench --memories 1000 --queries 8 --iterations 50
```

The benchmark creates a temporary Noema root, writes synthetic Markdown memory
records, and measures two paths:

- `noema_engine_recall`: reuse one `NoemaEngine` and call `recall`.
- `zode_turn_injection_equivalent`: mirror zode's current memory injection path
  by constructing an engine from `NOEMA_ROOT`, recalling, and rendering the
  `MemoryPack` to Markdown for each turn. This does not include provider/model
  latency.

Measured on 2026-06-21 with a release build on macOS 26.5.1, Apple M2 Pro,
`rustc 1.95.0`, `cargo 1.95.0`.

Dataset: 1000 generated memories, 8 queries, 50 iterations, 400 measured
operations per scenario, 150262 generated memory-body bytes.

| Scenario | Operations | Total ms | Mean us/op | p50 us | p95 us |
| --- | ---: | ---: | ---: | ---: | ---: |
| noema_engine_recall | 400 | 9391.788 | 23479.470 | 22976.084 | 25944.042 |
| zode_turn_injection_equivalent | 400 | 10524.228 | 26310.570 | 23319.334 | 46983.917 |

Phase breakdown:

| Scenario | Phase | Operations | Total ms | Mean us/op |
| --- | --- | ---: | ---: | ---: |
| noema_engine_recall | load_memories | 400 | 8153.373 | 20383.432 |
| noema_engine_recall | score_memories | 400 | 1143.644 | 2859.110 |
| noema_engine_recall | build_pack | 400 | 43.103 | 107.756 |
| zode_turn_injection_equivalent | create_engine | 400 | 0.101 | 0.252 |
| zode_turn_injection_equivalent | load_memories | 400 | 9255.372 | 23138.431 |
| zode_turn_injection_equivalent | score_memories | 400 | 1170.879 | 2927.197 |
| zode_turn_injection_equivalent | build_pack | 400 | 43.847 | 109.619 |
| zode_turn_injection_equivalent | render_markdown | 400 | 2.085 | 5.211 |

These numbers measure the current file-backed lexical recall path. They are a
baseline for future work on cached indexes, zode-native turn integration, and
cold-store hydration behavior.

## Development

Run the standard checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run a local smoke test:

```bash
export NOEMA_ROOT="$(mktemp -d)"
cargo run -p noema -- init
cargo run -p noema -- remember "Prefer Rust for Noema."
cargo run -p noema -- review
cargo run -p noema-mcp <<< '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

## Roadmap

- Stabilize the file protocol and frontmatter schema.
- Harden MCP and host integration contracts.
- Expand zode-native review and recall workflows.
- Complete production S3-compatible offload configuration.
- Add richer enterprise policy management and tenant administration.
- Publish crate/package releases after the storage contract is stable.

## License

MIT. See the workspace package metadata in `Cargo.toml`.
