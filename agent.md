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
| `lib.rs` | Library exports — `escape_v2_safe()` for MarkdownV2 escaping. |
| `bot.rs` | Core orchestration: `GlowBot`, `BotState`, tool dispatch, heartbeat tasks. Heavy integration tests. |
| `config.rs` | YAML config loading/saving — `Config`, `ChatConfig`, `McpServer`, allowlists. |
| `commands.rs` | Telegram command parsing: `/model`, `/mode`, `/reload`, `/status`. |
| `memory.rs` | User and chat memory files in Markdown with YAML frontmatter. |
| `skills.rs` | Skill directories with `skill.md` files (frontmatter + body). |
| `system_prompt.rs` | Assembles the full system prompt from skills, memories, config. |
| `tasks.rs` | Per-chat task lists (pending background work). |
| `llm.rs` | LLM trait (`LlmBackend`) and mock implementation. |
| `openrouter.rs` | OpenRouter API types and client — chat completions, tool definitions. |
| `mcp.rs` | MCP (Model Context Protocol) client — JSON-RPC over HTTP to external tool servers. |
| `bash.rs` | Shell command execution utility. |
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
