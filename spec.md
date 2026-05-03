# GlowBot — Specification

## 1. Overview

GlowBot is a personal Telegram chatbot inspired by OpenCLAW, built for a small private audience (me and friends). It connects to group chats and direct messages, uses OpenRouter.ai as its LLM backend, and augments itself with a skill system and persistent per-user memory.

The bot runs in Docker with raw bash access as its sole system tool — safe by container isolation.

---

## 2. Technology Stack

| Layer | Choice |
|-------|--------|
| Language | Rust (stable) |
| Configuration | YAML |
| Messaging | Telegram Bot API |
| LLM Backend | OpenRouter.ai |
| Memory (later) | SQLite + vector extension (RAG) |
| Deployment | Docker (single container) |
| CI/CD | GitHub Actions |
| Test coverage | ≥ 95% (enforced) |

---

## 3. Data Directory & Configuration

All persistent data lives under a single root directory tracked by git:

```
glowbot_data/
  config.yaml
  skills/
    <skillname>/
      skill.md
  chats/
    <chat_id>/
      <user_id>.md
```

### 3.1 Configuration (`config.yaml`)

```yaml
# Global
telegram_token: "..."

# OpenRouter
openrouter_api_key: "..."
openrouter_default_model: "anthropic/claude-sonnet-4"

# Conversation context window (number of recent messages, default: 20)
conversation_window: 20

# DM access control (empty = anyone can chat, tools disabled)
dm_whitelist: []                           # user IDs for full DM tool access

# MCP servers for additional tools
# mcp_servers:
#   - name: "my-server"
#     url: "https://mcp.example.com/mcp"
#     api_key: "optional-bearer-token"

# Chat-specific overrides (keyed by Telegram chat ID)
chats:
  "-1234567890":
    model: "openai/gpt-4o"               # optional, overrides default
    interaction_mode: "every_message"    # "every_message" | "mention_only"
    interaction_whitelist: []            # user IDs; empty = everyone allowed
    command_whitelist: []                # user IDs; empty = nobody allowed
    system_prompt: ""                    # optional per-chat system prompt
```

`/commands` at runtime can modify `model` and `interaction_mode` for the active chat (if the sender is on the `command_whitelist`).

### 3.2 Git Versioning

Every write to the data directory — config changes via `/commands`, skill creation/edits, and memory updates — triggers an automatic:

1. `git add <changed file(s)>`
2. `git commit -m "<generated message describing the change>"`
3. `git push`

This provides an audit trail and makes the bot's data portable and cloneable.

The data directory is a standalone git repository, not nested inside the application source.

**Git setup on startup:** The bot automatically configures git on boot to avoid common issues:
- `git config --global --add safe.directory <data_dir>` — prevents "dubious ownership" errors in Docker where the volume owner differs from the container user.
- `git config --global user.email "glowy@glowythebot.com"` — sets commit author email.
- `git config --global user.name "GlowBot"` — sets commit author name.

---

## 4. Core Systems

### 4.1 Telegram Integration

- Receives messages via long-polling or webhook (configurable, poll default).
- Sends responses as plain text or Markdown.
- Tracks chat context: group vs. DM, user identity (ID + username).
- Shows **typing indicator** (`sendChatAction`) while the LLM is processing a response.

#### Interaction modes

| Mode | Behavior |
|------|----------|
| `every_message` | Bot reads every message and may respond autonomously. |
| `mention_only` | Bot only responds when explicitly @mentioned or replied to. **Only applies to group chats** (negative chat IDs). **DMs (private chats) always respond** regardless of this setting — users don't @mention bots in 1:1 conversations. |

#### DM tool access (`dm_whitelist`)

DMs have an additional access control separate from group interaction modes:

| `dm_whitelist` | Behavior |
|----------------|----------|
| Empty (default) | Bot responds to all DMs, but **all tools are disabled** (text-only). The bot tells the user to add their ID to the whitelist to enable tools. |
| Contains user IDs | Only whitelisted users can interact. Whitelisted users get full tool access. Non-whitelisted users are blocked with a message. |

This prevents random strangers from running arbitrary bash commands while keeping DMs open for conversation.

`/commands` are always recognised regardless of interaction mode.

---

### 4.2 LLM Integration (OpenRouter)

- Sends chat context + tools + skills + memory to the configured model.
- Model is set per chat in config (overridable via `/model`).
- Handles tool-use responses with a multi-turn loop (up to 10 rounds).
- Maintains a **sliding conversation window** (`conversation_window` in config, default 20) of recent user + assistant messages per chat, sent as context with each request.
- Responses are sent with `ParseMode::MarkdownV2`. LLM output is converted via the `telegram-markdown-v2` crate (`convert_with_strategy` with `UnsupportedTagsStrategy::Escape`), which parses standard Markdown and emits properly escaped V2. Unsupported constructs (tables, blockquotes, raw HTML) are escaped as plain text rather than crashing. The system prompt instructs the LLM to wrap tables in code blocks (```) so they render as preformatted text. Falls back to plain text on conversion failure.
- Hardcoded tools exposed to the LLM:
  - **`bash`** — raw shell execution for file ops, API calls, invoking skills.
  - **`read_memory`** — returns a user's memory as structured JSON (frontmatter + body).
  - **`update_memory`** — partial update of a user's memory; all fields optional.
  - **`read_chat_memory`** — returns chat-level memory as JSON.
  - **`update_chat_memory`** — partial update of chat memory; all fields optional.
  - **`create_skill`** — create a new skill file (name, description, body). Triggers reload.
  - **`read_skill`** — read an existing skill's full content as JSON.
  - **`update_skill`** — update an existing skill (name, description?, body?). Triggers reload.
  - **`add_task`, `list_tasks`, `remove_task`** — manage the chat's task list.
  - **`send_message`** — send a plain text message to the current chat. Available in all contexts; primarily intended for background tasks to report completion or deliver results (used sparingly, at most once per task). In normal conversation the assistant reply itself is the message.
- **MCP tools** are dynamically added from configured servers. They are prefixed `mcp_<server>_<tool>` and discovered on startup via the MCP protocol (JSON-RPC, `initialize` → `tools/list`). See §4.7.

**Important implementation detail:** Bash commands run with the data directory as working directory. All paths must be relative (e.g. `chats/123/456.md`, not `glowbot_data/chats/123/456.md`). The system prompt is given the current `chat_id` so the LLM knows the exact memory file paths.

---

### 4.3 Skills System

Skills extend the bot's capabilities. Each skill is a directory under `skills/`:

```
skills/
  <skillname>/
    skill.md          # required (v1)
    src/              # optional (v2, compiled skills)
    Cargo.toml        # optional (v2)
```

#### Simple skills (MVP)

A simple skill is a `skill.md` file inside its named directory with YAML frontmatter and a free-text body:

```markdown
---
name: search-web
description: Searches the web for a given query and returns the top results
---
To use this skill, run:
curl -s "https://html.duckduckgo.com/html/?q=<urlencoded query>" | ...
Parse the results and summarize.
```

- The body is injected into the system prompt or tool description so the LLM knows what bash commands to run.
- Skills are loaded at startup from `skills/*/skill.md` and can be reloaded at runtime via `/reload`.

#### Compiled skills (Phase 2)

- A compiled skill adds Rust source code (`src/`, `Cargo.toml`) alongside `skill.md` in the same directory.
- Communication: **oneshot stdin/stdout** — the bot writes the LLM's request as JSON to stdin, reads the result from stdout.
- The bot can generate Rust source, compile it (`cargo build`), and hot-load the binary.

#### Skill creation

- The bot only creates or modifies skills when explicitly asked by a user.
- Skills are created and updated via structured tools (`create_skill`, `update_skill`) — the LLM never writes skill files by hand via bash. This guarantees correct YAML frontmatter format.
- `create_skill(name, description, body)` — creates `skills/<name>/skill.md` and reloads.
- `update_skill(name, description?, body?)` — updates an existing skill, only overwrites provided fields, then reloads.
- (Phase 2) It can generate and compile Rust skills.

### 4.6 MCP Integration

GlowBot can connect to [MCP (Model Context Protocol)](https://modelcontextprotocol.io) servers to discover and use external tools.

**Configuration:**
```yaml
mcp_servers:
  - name: "my-server"
    transport: "streamable"          # "streamable" (session-based, default) or "http" (stateless)
    url: "https://mcp.example.com/mcp"
    api_key: "optional-bearer-token"
```

**How it works:**
- On startup, the bot connects to each configured MCP server.
- Protocol version negotiation: tries `2025-11-25` → `2025-06-18` → `2024-11-05`.
- For `streamable` transport: captures `Mcp-Session-Id` from initialize response and includes it on all subsequent requests.
- Discovers tools via `tools/list`.
- Exposes discovered tools to the LLM as `mcp_<server>_<tool_name>`.
- When the LLM calls an MCP tool, the bot proxies `tools/call` to the server (with session ID if streamable).
- Failed server connections are non-fatal — logged as warnings, bot continues.
- Authorization: `Bearer <api_key>` header on all requests if `api_key` is set.

---

### 4.7 Heartbeat Task System

Each chat can have a task list (`chats/<chat_id>/tasks.yaml`) that the bot processes autonomously on a timer.

**Configuration:**
```yaml
heartbeat_interval_minutes: 90   # global (0 = disabled)
chats:
  "-123":
    heartbeat_interval_minutes: 30   # per-chat override
```

**How it works:**
- Users (or the LLM) add tasks via `add_task(description)`.
- `list_tasks()` shows pending tasks, `remove_task(id)` removes them.
- A background **scheduler** loop runs every 60s, scanning all chat directories for `chats/<chat_id>/tasks.yaml`.
- For each chat with pending tasks, a **dedicated timer loop** is spawned with that chat's configured interval (in seconds). If the chat has no custom interval, the global default is used. DMs always use the global default since they can't be preconfigured.
- Each per-chat loop independently: picks the oldest task → runs the LLM agent → sleeps for its interval → repeats. If heartbeat is disabled (interval = 0) the loop exits and will be respawned on the next scheduler scan if it becomes enabled again. If the agent completes all tasks, the loop exits immediately so the chat becomes eligible for re-discovery when new tasks are added.
- The agent uses bash, MCP tools, and task tools to complete the work. Each task is processed **at most once per cycle**, preventing re-grinding if a task cannot be completed yet (e.g. waiting for a download). If the agent loops back to a task already handled this interval, the cycle exits early.
- The agent may send Telegram messages via the **`send_message` tool** when a task requires it — for example, to report completion or deliver results (e.g. *"Download this file, then tell me when it's done"*). It should NOT spam progress updates (e.g. "63% done") — only final or actionable results, at most once per task. If a task does not require messaging, it completes silently.
- Task processing runs in its own task — does not block message handling.

**Tools:** `add_task`, `list_tasks`, `remove_task`, `send_message`.

---

### 4.8 Memory System

#### Short-term (conversation context)

- Sliding-window of recent messages (user + assistant) sent to the LLM with each request.
- Window size configurable via `conversation_window` in `config.yaml` (default: 20).
- Stored in-memory per chat; resets on bot restart.

#### Long-term (per-user `.md` files)

```
chats/
  <chat_id>/
    _chat.md        ← chat-level memory (name, description, log)
    <user_id>.md    ← per-user memory
    <user_id>.md
    ...
```

Each file uses YAML frontmatter for structured metadata, followed by a freeform Markdown body. The `_chat.md` file uses the same format but describes the chat itself (call_name, description, log) rather than a user.

```markdown
---
user_id: "123456789"
username: "@korwi"
call_name: "Koriwi"
description: |
  Koriwi is a Rust developer. Prefers concise answers. Skill level: expert.
  Dislikes small talk. Works on open-source projects in their free time.
---

# Log

- 2026-05-02: Mentioned they use NixOS on their main machine.
- 2026-05-02: Asked about async Rust patterns — comfortable with tokio.
```

- The LLM uses **structured tools** (`read_memory` and `update_memory`) to interact with memory files — it never edits the YAML by hand via bash. This guarantees correct format.
  - `read_memory(user_id)` → returns the full memory as JSON: `{user_id, username, call_name, description, body}`.
  - `update_memory(user_id, ...)` → partial update. All fields optional (`username`, `call_name`, `description`, `log_entry`). Only provided fields overwrite existing values. `log_entry` appends a timestamped line to the body.
- **Chat-level memory** uses the same tools:
  - `read_chat_memory()` → returns chat memory as JSON: `{call_name, description, body}`.
  - `update_chat_memory(...)` → partial update with optional fields (`call_name`, `description`, `log_entry`).
- The bot **autonomously** calls `update_memory` / `update_chat_memory` with facts it considers worth remembering.
- **Only the YAML frontmatter** (user_id, username, call_name, description) is injected into the system prompt — keeping context usage minimal.
- The LLM can still read the full file body via `read_memory` (or raw `bash`) if it needs deeper recall.
- The `call_name` is what the bot uses when addressing the user in conversation. It is inferred autonomously on first encounter, but the bot may ask the user directly if it is unsure what to call them.
- A separate file per chat means the same user can have different context in different chats.

#### Vector memory (Phase 2)

- All messages are embedded and stored in SQLite with a vector extension.
- The bot queries relevant past messages on demand for RAG.
- Per-user `.md` files remain as canonical long-term memory but are also embedded.

---

### 4.9 Bash Tool

The bot exposes a **bash** tool for raw shell execution.

- Commands run in a subprocess inside the Docker container (working directory = data directory).
- No interactive/session-based commands — each invocation is stateless (oneshot).
- 30-second timeout per command.
- The container provides isolation; no additional sandboxing is required.
- The LLM uses bash to invoke skills, manipulate files, query APIs, etc.
- **Memory files should be accessed via `read_memory` / `update_memory` tools**, not raw bash — this guarantees correct YAML frontmatter format.
- All file paths in bash commands are relative to the data directory (e.g. `chats/123/456.md`).

### 4.10 Tool Call Logging

Every tool invocation is logged for debugging and audit:
- Written to `glowbot_data/tool_calls.log` (append-only).
- Format: `[timestamp] tool_name | args: <json> | result: <first 200 chars>`
- Also logged to stdout via `log::info!` for Docker log visibility.
- Log writes are best-effort — they never block or fail message processing.

---

### 4.11 Commands & Permissions

Commands are Telegram bot commands (`/command`) used for control and settings.

| Command | Purpose | Requires |
|---------|---------|----------|
| `/model <name>` | Change the LLM model for this chat | command whitelist |
| `/mode <every_message\|mention_only>` | Change interaction mode | command whitelist |
| `/reload` | Reload skills from disk | command whitelist |
| `/status` | Show current config for this chat | interaction whitelist |

#### Whitelist rules

| Whitelist | Default | Controls |
|-----------|---------|----------|
| `interaction_whitelist` | Empty = **everyone** | Who the bot talks to / responds to |
| `command_whitelist` | Empty = **nobody** | Who can run `/commands` |

Whitelists contain Telegram user IDs.

---

## 5. MVP Scope (v1)

### Must have

- [x] Telegram messaging (groups + DMs, long-polling)
- [x] OpenRouter LLM integration with multi-turn tool-use loop
- [x] YAML configuration with per-chat model, interaction mode, and conversation window
- [x] Simple skills (`skills/<name>/skill.md` with frontmatter)
- [x] Bash tool (oneshot, container-isolated, 30s timeout)
- [x] `read_memory` / `update_memory` structured tools for per-user `.md` memory
- [x] `create_skill` / `update_skill` structured tools for skill management
- [x] Tool call logging to `tool_calls.log`
- [x] MCP server integration for external tool discovery
- [x] Heartbeat task system with autonomous background agents
- [x] Per-user `.md` memory with YAML frontmatter, freeform body
- [x] Memory frontmatter injected into system prompt; full file readable via tools
- [x] `/model`, `/mode`, `/reload`, `/status` commands
- [x] Interaction & command whitelists per chat
- [x] Git auto-commit + push on every data write (with safe.directory and identity setup)
- [x] Docker deployment with `glowbot_data/` as a volume
- [x] GitHub CI/CD with ≥95% test coverage enforced
- [x] Conversation history (sliding window, configurable size)
- [x] Typing indicator while LLM is processing
- [x] MarkdownV2 rendering via `telegram-markdown-v2` crate with plain text fallback

### Explicitly out of scope for v1

- Compiled skills (Rust binaries)
- Skill compilation by the bot
- Message embedding / RAG / SQLite vector
- Webhook mode (polling only)
- Obsidian-like `.md` knowledge base

---

## 6. Roadmap

### v1 (MVP)
Everything in §5.

### v2
- Compiled skills (Rust subprocess, oneshot stdin/stdout)
- Bot-generated & compiled skills
- Message embedding + SQLite vector RAG
- Embed per-user `.md` memory files into vector DB

### v3 (candidates)
- Webhook mode for Telegram
- Obsidian-style interlinked `.md` knowledge base (if it proves useful)
- Additional tools beyond bash (if needed)

---

## 7. Development Practices

- All code in Rust (stable channel).
- Tests must reach ≥95% line coverage; CI fails below threshold.
- Linting (`clippy`) and formatting (`rustfmt`) enforced in CI.
- Docker image built and pushed on each tagged commit.
- `Dockerfile` is multi-stage: builder stage (Rust toolchain) → slim runtime stage.
- The `glowbot_data/` directory (config, skills, chats) is mounted as a Docker volume — not baked into the image.
- `glowbot_data/` is a standalone git repository. The container needs git installed and push access (SSH key or token).

## 8. Operational Notes & Learnings

### Docker / Git
- **Git "dubious ownership":** When the data volume is mounted from the host, the directory owner differs from the container user. The bot runs `git config --global --add safe.directory` on startup to suppress this.
- **Git identity:** Commit author name/email must be set (`git config --global user.name/email`) or commits fail silently.

### Memory file paths
- Bash commands run with the data dir as working directory. The LLM must use **relative paths** (`chats/123/456.md`), not absolute (`glowbot_data/chats/123/456.md`). The system prompt is given the current `chat_id` to construct correct paths.

### Memory format
- Letting the LLM edit YAML frontmatter by hand via bash heredocs/sed is unreliable — it will invent its own format. Use **structured JSON tools** (`read_memory` / `update_memory`) instead. The tool guarantees correct format.

### DMs vs Groups
- Telegram DMs have positive chat IDs, groups have negative. The `mention_only` interaction mode should **only apply to groups** — DMs always respond since users don't @mention in 1:1 chats.

### Typing indicator
- Send `sendChatAction(Typing)` before starting LLM processing so the user sees the "..." animation while waiting.

### Conversation history
- Without a sliding window of recent messages, the bot has no short-term memory and can't follow multi-turn conversations. The window size should be configurable (`conversation_window`).

### Markdown rendering
- Telegram messages must be sent with a parse mode (`MarkdownV2` or `HTML`) or they render as plain text. Default `send_message` has no parse mode.
- **Do not hand-roll MarkdownV2 escaping.** Telegram's V2 has many reserved characters (`_ * [ ] ( ) ~ \` > # + - = | { } . !`) and context-dependent escaping rules (e.g. `-` at line start vs mid-word). LLMs will constantly hit edge cases.
- **Use the `telegram-markdown-v2` crate** (`convert()`). It parses the LLM's Markdown output and produces properly escaped MarkdownV2, handling headings→bold, lists→Unicode bullets, code blocks, links, and inline formatting correctly.
- Fall back to plain text if conversion fails (rare with a good crate).
