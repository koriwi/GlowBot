# GlowBot Agent Guide

## Coverage & Testing

### Running Tests with Code Coverage

This project uses `cargo-llvm-cov` for code coverage (installed via `cargo install cargo-llvm-cov`).

```bash
# Run all tests
cargo test

# Run tests with coverage report (summary only)
cargo llvm-cov --summary-only

# Run tests and generate HTML report
cargo llvm-cov --html

# Open HTML report after generation
cargo llvm-cov --html --open
```

Current coverage target is **≥95% line coverage** (excluding `main.rs` which is the async entrypoint and inherently hard to unit-test).

### Test Dependencies

- **mockall** (0.13): For mocking traits with `#[automock]`
- **wiremock** (0.6): For mocking HTTP servers in async tests
- **tempfile** (3): For temporary directories in tests

See existing `Wiremock` tests in `bot.rs`, `mcp.rs`, etc. for patterns.

## Project Structure

### Source Layout (`src/`)

| File | Purpose |
|------|---------|
| `main.rs` | Async Telegram bot entrypoint — polling loop, message handler, heartbeat orchestrator. Not unit-tested. |
| `lib.rs` | Library exports — `escape_v2_safe()` for MarkdownV2 escaping, module declarations. |
| `bot.rs` | Core orchestration: `GlowBot`, `BotState`, message processing, command handling, LLM pipeline. |
| `bot_dispatch.rs` | Tool dispatch functions (`dispatch_tool`, `dispatch_tool_calls`, `log_tool_call_to`). Submodule of `bot`. |
| `bot_heartbeat.rs` | Background heartbeat task runner. Submodule of `bot`. |
| `bot_tests.rs` | All integration tests for bot module. Included via `#[cfg(test)] #[path = "bot_tests.rs"] mod tests;`. |
| `config.rs` | YAML config loading/saving — `Config`, `ChatConfig`, `McpServer`, allowlists. |
| `commands.rs` | Telegram command parsing: `/model`, `/mode`, `/reload`, `/status`. |
| `memory.rs` | User and chat memory files in Markdown with YAML frontmatter. |
| `skills.rs` | Skill directories with `skill.md` files (frontmatter + body). |
| `system_prompt.rs` | Assembles the full system prompt from skills, memories, config. |
| `tasks.rs` | Per-chat task lists (pending background work). |
| `llm.rs` | LLM trait (`LlmBackend`) and mock implementation. |
| `openrouter.rs` | OpenRouter API types and client — chat message types, completion request/response, token estimation, context trimming. |
| `openrouter_tools.rs` | All tool definitions for the LLM (bash, memory, skills, tasks, etc.). Submodule of `openrouter`. |
| `openrouter_tests.rs` | All tests for openrouter module. Included via `#[cfg(test)] #[path = "openrouter_tests.rs"] mod tests;`. |
| `mcp.rs` | MCP (Model Context Protocol) client — JSON-RPC over HTTP to external tool servers. |
| `mcp_tests.rs` | All tests for MCP module. Included via `#[cfg(test)] #[path = "mcp_tests.rs"] mod tests;`. |
| `bash.rs` | Shell command execution utility. |
| `db.rs` | SQLite-backed conversation history. |
| `git.rs` | Git auto-commit wrapper for data directory. |

### Test Patterns

- **Mock LLM**: `MockLlmBackend` in `llm.rs` — queue responses with `mock.add_response(...)`
- **TempDir**: Always create `glowbot_data/` under a `TempDir` and write `config.yaml`
- **Wiremock**: For HTTP-dependent modules (MCP, OpenRouter), spin up a local mock server
- **Bot setup helpers**: `setup_test_bot()` and `setup_test_bot_with_whitelisted_chat()` in `bot.rs`

### Key Behaviours

- **Interaction modes**: `MentionOnly` (default for groups), `EveryMessage`
- **DM handling**: Positive chat IDs = DM. Always respond, but tools only if whitelisted.
- **Whitelists**: Empty = allow all. Non-empty = block non-listed users.
- **Heartbeat**: Background task processing on a timer. Uses `send_message` tool.
- **Tool loop**: Max 10 rounds to prevent infinite tool-call loops.
- **Auto-commit**: Git commits after config saves and tool executions.

## Dev Workflow

1. Read `spec.md` for current behaviour spec
2. Add tests → run `cargo test`
3. Check coverage → `cargo llvm-cov --summary-only`
4. If coverage dropped or new features added: update `spec.md`
5. `git add`, `git commit`, `git push`

## Refactoring Rules

### File Size Limits

- **Target**: max ~300 lines per source file
- **Hard upper limit**: 450 lines (only if splitting would make code harder to read)
- Test files don't count toward the limit (they're moved to separate files)

### Splitting Strategy for Rust Modules

When a `.rs` file gets too large, split it using submodules:

1. **Move tests to a separate file**: Use `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;` in the parent. Tests keep access to private items via `use super::*;`.

2. **Extract logical groups into submodules**: Create `foo_bar.rs` and declare it in the parent with `#[path = "foo_bar.rs"] mod foo_bar;`. Use `#[path]` because Rust expects submodule files in a directory named after the parent module — `#[path]` keeps them flat in `src/`.

3. **Re-export public API**: Use `pub use self::foo_bar::some_function;` in the parent so callers don't need to change import paths.

4. **Make extracted functions `pub(crate)`**: If they're only used within the crate. For items used from `main.rs` (binary crate), they need to be fully `pub`.

### Common Pitfalls

- `pub(crate)` items cannot be `pub use`d outside the crate — the binary crate (`main.rs`) counts as external.
- When calling a function moved to a submodule from the parent, add `use self::submodule::function;` in the parent.
- The `teloxide::prelude::*` import is needed in any file that calls `send_message` or other `Requester` trait methods.
- After splitting, always run `cargo test` and `cargo llvm-cov` to verify nothing broke.
