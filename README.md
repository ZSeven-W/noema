# Noema

Noema is a local-first, non-vector memory system for coding agents.

This first implementation supports:

- `tenant=personal`
- user/project memory shape
- Markdown memory files under `~/.agent-memory/`
- JSONL hippocampus queue
- payload-free JSONL audit
- file locks on mutation paths
- review, accept, reject, edit, and merge actions
- duplicate/conflict checks before auto-write
- merge target validation, with target consolidation deferred
- full-scan lexical recall and `noema explain`
- personal mode sensitivity capped at `public` / `internal`
- enterprise-mode `normal` / `never` sensitivity behavior

Enterprise broker, MCP, S3 cold offload, redacted/summary variants, and KMS policy are outside the first deliverable.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## CLI Smoke

```bash
NOEMA_ROOT="$(mktemp -d)" cargo run -p noema -- init
NOEMA_ROOT="$NOEMA_ROOT" cargo run -p noema -- remember "Prefer Rust for Noema."
NOEMA_ROOT="$NOEMA_ROOT" cargo run -p noema -- review
```
