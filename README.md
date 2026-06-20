# Noema

Noema is a local-first, non-vector memory system for coding agents.

Implemented scope:

- P0 file protocol and tenant-scoped layout
- P1 lexical recall and MemoryPack output
- P2 event-sourced review queue
- P3 zode native recall and extraction queue
- P4 MCP tool surface
- P5 S3-compatible cold offload metadata and restore safeguards
- P6 signed-principal enterprise boundary, ACL policy, and KMS metadata policy

Noema intentionally does not use vectors or embeddings.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Local Smoke

```bash
NOEMA_ROOT="$(mktemp -d)" cargo run -p noema -- init
NOEMA_ROOT="$NOEMA_ROOT" cargo run -p noema -- remember "Prefer Rust for Noema."
NOEMA_ROOT="$NOEMA_ROOT" cargo run -p noema -- review
cargo run -p noema-mcp <<< '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```
