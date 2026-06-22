---
name: noema-memory
description: Use Noema CLI for cross-session agent memory in Codex, Claude Code, or other CLI hosts. Trigger when a task asks to remember, recall, use long-term memory, answer from previous sessions, or maintain personal/project preferences without an MCP server.
---

# Noema Memory

Use the local `noema` CLI as the memory backend. Keep memory use silent unless
the user asks about memory status or a command fails in a way that affects the
answer.

## Recall

Before answering a user request that may depend on previous sessions, preferences,
people, aliases, project conventions, or remembered facts, run:

```bash
noema recall "<user request>"
```

Read the returned markdown memory pack. Use relevant memories as context, but do
not quote the memory pack unless the user asks to inspect memory. If the command
is unavailable or returns no relevant memories, continue normally.

## Remember

When the user explicitly asks to remember something, or states a stable
preference/fact that should persist, run:

```bash
noema remember "<stable fact or preference>" --accept
```

Examples worth storing:

```text
请记住我喜欢 Rust 工具
老李就是李小红
老李爱健身
For this project, prefer pnpm.
```

Do not store secrets, credentials, one-time instructions, transient debugging
state, or content the user told you not to remember. If the content is sensitive
or ambiguous, ask before storing it.

## Project Memory

For project-specific conventions, include project scope:

```bash
noema remember "<project convention>" --scope project --accept
```

Run recall from the project directory so Noema can apply project context:

```bash
noema recall "<question>"
```

## Useful Commands

```bash
noema search "<query>"
noema review
noema forget <memory_id>
```

Use `search` for diagnostics and `recall` for model context, because `recall`
returns the full memory text.
