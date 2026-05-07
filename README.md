# GlowBot

A personal Telegram chatbot backed by OpenRouter LLMs, with skills, memory, MCP tool servers, and autonomous background tasks. Runs in a single Docker container with bash as its system tool — safe by container isolation.

## Quick Start

```bash
# 1. Clone and set up the data directory
git clone https://github.com/koriwi/GlowBot.git
cd GlowBot
mkdir -p glowbot_data
cp config.example.yaml glowbot_data/config.yaml
# Edit glowbot_data/config.yaml — fill in your Telegram token and OpenRouter API key

# 2. Build and run
docker build -t glowbot .
docker run -v $(pwd)/glowbot_data:/glowbot_data glowbot

# Or run natively (requires Rust toolchain)
cargo run
```

## Configuration

All settings live in `glowbot_data/config.yaml`. See [`config.example.yaml`](config.example.yaml) for the full annotated template.

### Minimum config

```yaml
telegram_token: "123:abc"
openrouter:
  api_key: "sk-or-..."
  model: "anthropic/claude-sonnet-4"
```

### MCP Servers (optional)

Add external tool servers via the [Model Context Protocol](https://modelcontextprotocol.io):

```yaml
mcp_servers:
  - name: "my-server"
    transport: "streamable"   # "streamable" (session-based, default) or "http" (stateless)
    url: "https://mcp.example.com/mcp"
    api_key: "optional-bearer-token"
```

Tools from MCP servers are discovered on startup and exposed to the LLM as `mcp_<server>_<tool_name>`.

### Full config reference

See [`config.example.yaml`](config.example.yaml) for every option including:
- Per-chat and per-DM model overrides, system prompts, heartbeat intervals
- Interaction modes (`every_message` / `mention_only`)
- DM access control (`dm_enabled`, `dms` map)
- Embedding / RAG search settings
- Bash tool toggle (global and per-chat)
- Media directory path

## Logging

GlowBot uses the [`log`](https://crates.io/crates/log) crate with [`env_logger`](https://crates.io/crates/env_logger). Control verbosity with the `RUST_LOG` environment variable.

### Log levels

| Level | When to use |
|-------|-------------|
| `error` | Something is broken (e.g. all MCP protocol versions failed) |
| `warn` | An operation failed but the bot continues (e.g. one MCP server unreachable) |
| `info` | Normal operational events — startup, connections, tool counts |
| `debug` | Detailed per-request diagnostics — URLs, auth status, response bodies |

### Usage

```bash
# Default: info and above (warn, error)
cargo run

# Show warnings and errors only
RUST_LOG=warn cargo run

# Show everything — useful for debugging MCP connections
RUST_LOG=debug cargo run

# Show debug for MCP only, info for everything else
RUST_LOG=info,glowbot::mcp=debug cargo run

# Docker
docker run -e RUST_LOG=debug -v $(pwd)/glowbot_data:/glowbot_data glowbot
```

### Debugging MCP connections

When an MCP server isn't working, set `RUST_LOG=info` or `RUST_LOG=debug` and look for these log lines:

```
MCP discover_all: 2 server(s) configured
MCP discover_all: connecting to 'my-server' at https://mcp.example.com/mcp (transport=streamable, auth=true)
MCP 'my-server': connecting to https://mcp.example.com/mcp (transport=streamable, auth=true)
```

**At `RUST_LOG=info` you can diagnose:**

| Symptom | Log line |
|---------|----------|
| URL unreachable / DNS failure | `MCP 'my-server': network error reaching https://... for initialize: ...` |
| HTTP error (wrong URL, auth middleware) | `MCP 'my-server' HTTP 4xx from https://... (method=initialize): ...` |
| Auth rejected | `MCP 'my-server' HTTP 401 from https://... (method=initialize): ...` |
| Protocol version mismatch | `MCP 'my-server': protocol 2025-11-25 failed: ...` (repeated for each version) |
| All versions exhausted | `MCP 'my-server': all protocol versions failed for https://...` (error level) |
| Server connected, no tools | `MCP server 'my-server' connected: 0 tools discovered` |
| Server OK, tools loaded | `MCP server 'my-server' connected: 5 tools discovered` |

**At `RUST_LOG=debug` you additionally see:**

| Detail | Log line |
|--------|----------|
| Auth key preview (masked) | `MCP 'my-server': using Bearer auth (key=sk-a...b12c)` |
| Session ID | `MCP 'my-server': using session id sess-abc123` |
| Every RPC method call | `MCP 'my-server' → https://... tools/list (transport=streamable, auth=true)` |
| Pagination progress | `MCP 'my-server': fetching tools/list page 1` / `page 1 returned 10 tools` |
| JSON parse failures | `MCP 'my-server': failed to parse JSON-RPC response from ...` |
| Tool invocation details | `mcp_my-server_search: calling search at https://... (transport=streamable, session=true, auth=true)` |

Common issues and how to spot them:

- **Wrong URL or port**: `network error` — check the URL is reachable from the Docker container
- **Wrong transport**: An `http` server won't handle `Mcp-Session-Id`; a `streamable` server may require it. Check the `transport` field.
- **Missing API key**: Look for `auth=false` in connection logs when the server requires one
- **Server doesn't support any protocol version**: `all protocol versions failed` at error level, with per-version warnings showing what each returned
- **Session expired mid-use**: `log::info!("MCP session expired for ..., attempting re-initialization")` — the bot retries automatically

## Commands

| Command | Purpose |
|---------|---------|
| `/status` | Show current config and context usage for this chat |
| `/tasks` | List pending heartbeat tasks |
| `/stop` | Interrupt ongoing LLM processing |
| `/new` | Reset conversation context (sets a cutoff timestamp) |

Commands must be enabled per-chat via `commands_enabled: true` (except `/stop` which always works).

## Features

- **Multi-model**: Per-chat and per-DM model selection via OpenRouter
- **Skills**: Extend the bot with `skill.md` files under `skills/<name>/`
- **Memory**: Per-user and per-chat Markdown files with YAML frontmatter, plus SQLite-backed conversation history
- **MCP tools**: Connect to external MCP servers for additional tool access
- **Heartbeat tasks**: Autonomous background task processing on a timer per chat
- **Bash tool**: Oneshot shell execution, isolated by Docker container
- **Reasoning capture**: Optionally capture and reuse LLM thinking/reasoning chains
- **RAG search**: Semantic search over conversation history (requires embedding model)
- **Git auto-commit**: Every config/memory/skill write triggers `git add` → `commit` → `push`
- **MarkdownV2 rendering**: Proper Telegram MarkdownV2 output with fallback to plain text

## Development

```bash
# Run tests
cargo test

# Run with coverage (requires cargo-llvm-cov)
cargo llvm-cov --summary-only
cargo llvm-cov --html --open

# Lint
cargo clippy

# Format
cargo fmt
```

Test coverage target is ≥95% line coverage. See [`AGENTS.md`](AGENTS.md) for the full development guide including project structure, test patterns, and refactoring rules.

## License

MIT
