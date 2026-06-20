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

Search and inspect recall:

```bash
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
| `noema review` | List pending candidates. |
| `noema edit <candidate> --body ... --reason ...` | Revise a pending candidate before acceptance. |
| `noema merge <candidate> <memory> --reason ...` | Drop a duplicate candidate in favor of an existing memory. |
| `noema accept <candidate>` | Persist a pending candidate as memory. |
| `noema reject <candidate> --reason ...` | Reject a pending candidate. |
| `noema search <query>` | Recall matching memories. |
| `noema explain <memory> --query ...` | Explain a recall score. |
| `noema vacuum` | Compact terminal review events into snapshots and archives. |
| `noema sleep [--llm]` | Move stale or low-utility memory toward deeper storage. |
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

### MCP

`noema-mcp` exposes a stdio JSON-RPC surface for MCP-capable hosts:

```bash
cargo run -p noema-mcp
```

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
