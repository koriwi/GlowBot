# GlowBot — Specification

## 1. Overview

GlowBot is a personal Telegram chatbot inspired by OpenCLAW, built for a small private audience (me and friends). It connects to group chats and direct messages, uses either OpenRouter.ai or an OpenAI Codex subscription as its LLM backend, and augments itself with a skill system and persistent per-user memory.

The bot runs in Docker with raw bash access as its sole system tool — safe by container isolation.

---

## 2. Technology Stack

| Layer | Choice |
|-------|--------|
| Language | Rust (stable) |
| Configuration | YAML |
| Messaging | Telegram Bot API |
| LLM Backend | OpenRouter.ai or OpenAI Codex subscription |
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

provider: openrouter  # openrouter (default) | codex

# OpenRouter (required for provider: openrouter)
openrouter:
  api_key: "..."
  model: "anthropic/claude-sonnet-4"
  advice_model: "openai/gpt-4o"   # optional; when set enables ask_advisor tool

# Alternatively, use the ChatGPT/Codex subscription authenticated by `codex login`:
# codex:
#   model: "gpt-5.4"
#   auth_file: "~/.codex/auth.json"
#   reasoning_effort: "high"       # optional

# Conversation context settings
conversation:
  recent_messages_window_size: 20   # number of recent messages (default: 20)
  heartbeat_recent_messages_window_size: 10   # optional — conversation history messages for heartbeat tasks; falls back to recent_messages_window_size if unset; 0 = no history
  advice_recent_messages_window_size: 5   # number of recent messages sent to advice model via ask_advisor (default: 5)
  # max_tool_result_chars: 8000           # optional — cap tool result length; results over this limit are replaced with an error telling the LLM to reduce the response (default: no limit)

# MCP servers for additional tools
# mcp_servers:
#   - name: "my-server"
#     url: "https://mcp.example.com/mcp"
#     api_key: "optional-bearer-token"

dms:
  "123456789":
    provider: codex                       # optional, overrides global provider
    model: "gpt-5.4"                    # optional, overrides selected provider default
    commands_enabled: true
    system_prompt: ""                    # optional per-DM system prompt
    heartbeat_interval_minutes: 30
    bash_enabled: true
    advice_model: "openai/gpt-4o"   # optional, override global advice model

# Group-specific overrides (keyed by Telegram chat ID, negative)
chats:
  "-1234567890":
    provider: codex                       # optional, overrides global provider
    model: "gpt-5.4"                    # optional, overrides selected provider default
    interaction_mode: "every_message"    # "every_message" | "mention_only"
    interaction_whitelist: []            # user IDs; empty = everyone allowed
    command_whitelist: []                # user IDs allowed to run commands; empty = nobody
    system_prompt: ""                    # optional per-chat system prompt
    advice_model: "openai/gpt-4o"        # optional, override global advice model
```

`/commands` at runtime can modify settings for the active chat (if commands are enabled for that chat).

### 3.2 Git Versioning

Every write to the data directory — config changes via commands, skill creation/edits, and memory updates — triggers an automatic:

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
- Tracks chat context: group vs. DM, user identity (ID + username + display name), and Telegram message sent time.
- Each non-command message sent to the LLM is prefixed with Telegram metadata (`Sent at`, `Sender ID`, `Sender name`, `Sender username`) before the user's text, so the model can reason about who said what and when.
- Shows **typing indicator** (`sendChatAction`) while the LLM is processing a response.

#### Interaction modes

| Mode | Behavior |
|------|----------|
| `every_message` | Bot reads every message and may respond autonomously. |
| `mention_only` | Bot only responds when explicitly @mentioned or replied to. **Only applies to group chats** (negative chat IDs). **DMs (private chats) always respond** regardless of this setting — users don't @mention bots in 1:1 conversations. |

#### DM access control

DMs are configured via the `dms` map (keyed by user/chat ID). Only DMs explicitly listed in `dms` can interact with the bot. Unknown DMs receive a "I don't know you" message (includes the user's ID so the owner can add them).

`/commands` are always recognised regardless of interaction mode.

---

### 4.2 LLM Integration (OpenRouter and Codex)

- Sends chat context + tools + skills + memory to the configured model.
- `provider: openrouter` uses OpenRouter's Chat Completions API and API-key billing.
- `provider: codex` uses OpenAI's Codex Responses endpoint with the OAuth credentials generated by the official `codex login` command, so requests consume the user's ChatGPT/Codex subscription allowance. GlowBot refreshes expired access tokens and writes updated credentials back to `auth_file`.
- `provider` can be overridden in each `chats` or `dms` entry. This allows OpenRouter to remain the global default while selected chats use the Codex subscription. If that entry omits `model`, it uses the selected provider's global model.
- Codex Responses function calls are translated to GlowBot's existing tool loop. Encrypted reasoning items are replayed during the active tool turn, while reasoning summaries continue to be captured in conversation history.
- Codex OAuth does not provide embeddings. OpenRouter-only embeddings, image generation, and media fallback features continue to require an OpenRouter API key.
- Model is set per chat in config.
- Handles tool-use responses with a multi-turn loop (up to 10 rounds).
- Maintains a **conversation history** per chat, stored in-memory. Past messages are **not** automatically sent to the LLM. Instead, the bot sends only the current user message along with the system prompt. The LLM can call `get_recent_messages(count)` to retrieve prior messages on demand when it needs context.
- Previous messages are still kept for tracking purposes but do not consume context tokens unless explicitly requested.
- Responses are sent with `ParseMode::MarkdownV2`. LLM output is converted via the `telegram-markdown-v2` crate (`convert_with_strategy` with `UnsupportedTagsStrategy::Escape`), which parses standard Markdown and emits properly escaped V2. Unsupported constructs (tables, blockquotes, raw HTML) are escaped as plain text rather than crashing. The system prompt instructs the LLM to wrap tables in code blocks (```) so they render as preformatted text. Falls back to plain text on conversion failure.
- Hardcoded tools exposed to the LLM:
  - `read_memory` – returns a user's memory as structured JSON (frontmatter + body).
  - `update_memory` – partial update of a user's memory; all fields optional.
  - `read_chat_memory` – returns chat-level memory as JSON.
  - `update_chat_memory` – partial update of chat memory; all fields optional.
  - `create_skill` – create a new skill file (name, description, body). Triggers reload.
  - `read_skill` – read an existing skill's full content as JSON.
  - `update_skill` – update an existing skill (name, description?, body?). Triggers reload.
  - **`add_task`, `list_tasks`, `remove_task`** — manage the chat's task list.
  - **`create_reminder`, `list_reminders`, `remove_reminder`** — manage time-based reminders. See §4.7a.
  - **`create_todo`, `list_todos`, `edit_todo`, `delete_todo`** — manage the human todo list. See §4.7b.
  - **`get_model_info`** — returns current model, base model, specifier, config default, override status, available specifiers, and cached metadata (context length, pricing).
  - **`propose_model_change`** — proposes a model change to the user with Accept/Deny buttons. Takes `model_id` and/or `specifier`. If accepted, the model is temporarily switched (persists until `/model_default` resets it).
  - **`ask_advisor`** — calls a separate, larger/smarter advice model for private guidance. Enabled only when `openrouter.advice_model` is configured (no default). Takes `query` (string). Sends recent conversation messages (controlled by `conversation.advice_recent_messages_window_size`) plus the query to the advice model and returns its response. The advisor sees all conversation messages relabeled as `role: "user"` with `name` field distinguishing speakers — it never sees `role: "assistant"` it didn't produce. The calling model uses the advisor's output as internal guidance (not necessarily relayed verbatim to the end user).
  - **`read_config_schema`** — returns the JSON Schema for all config types.
  - **`read_config`** — returns the current config as YAML.
  - **`edit_config`** — proposes config changes via Accept/Deny dialog.
  - **`send_message`** — send a plain text message to the current chat. Always exposed — in normal conversations it's for headsup/intermediate messages (used sparingly); in heartbeat tasks for completion reports.
  - **`get_recent_messages`** — returns the last N messages from the conversation history. The bot does NOT automatically send past messages — only the current user message is included in each request. The LLM must call this tool when it needs context from earlier in the conversation.
- **MCP tools** are dynamically added from configured servers. They are prefixed `mcp_<server>_<tool>` and discovered on startup via the MCP protocol (JSON-RPC, `initialize` → `tools/list`). See §4.7.

**Important implementation detail:** Bash commands run with the data directory as working directory. All paths must be relative (e.g. `chats/123/456.md`, not `glowbot_data/chats/123/456.md`). The system prompt is given the current `chat_id` so the LLM knows the exact memory file paths.

---

#### Context usage tracking

- Each LLM response includes a `usage` object with `prompt_tokens`, `completion_tokens`, and `total_tokens`.
- The bot stores the last `prompt_tokens` per chat and uses it for `/status` display.
- Model context lengths (max tokens) are fetched once from OpenRouter's `/api/v1/models` endpoint on startup and cached in-memory.
- `/status` shows context usage as `"37k/252k (15%)"` — prompt tokens / model limit with percentage.
- If the model context length is not yet cached (e.g. OpenRouter fetch failed), `/status` shows `"unknown"`.
- Heartbeat tasks also track usage, so `/status` reflects the most recent activity even from background processing.

### 4.2a Media Ingest

The bot can receive and process **images** and **voice/audio messages** from users.

- Extraction: `IngestedMedia::try_from_message()` maps Telegram `Message` fields (photo, voice, audio) into a media enum. Videos and documents are skipped for now.
- Download: Media files are downloaded from Telegram's servers and cached to `media/ingest/`.
- Model-aware routing: The bot checks `architecture.input_modalities` from the cached model metadata to decide native vs fallback:
  - **Native image**: If the conversation model supports `image` input, the image is base64-encoded as `image_url` content part and sent directly.
  - **Native audio**: If the model supports `audio` input, the audio is base64-encoded as `input_audio` content part.
  - **Fallback image**: Uses `openrouter.image_fallback_model` to describe the image as text.
  - **Fallback audio**: Uses `openrouter.audio_fallback_model` to transcribe as text.
- Metadata: Fallback-converted media is prefixed with metadata (dimensions, duration, original filename) so the conversation model knows the context.
- Config: `image_fallback_model` and `audio_fallback_model` on `openrouter` config block.

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

- The **name and description** of each skill are injected into the system prompt as a compact list. The full body is **not** included — the LLM can call `read_skill(name)` to load the complete content on demand.
- Skills are loaded at startup from `skills/*/skill.md`.

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

### 4.4 Reasoning / Thinking Capture

Some LLM models (DeepSeek-R1, Claude with extended thinking, OpenAI o-series) return **reasoning/thinking content** — the model's internal chain of thought — alongside the final response. GlowBot can optionally capture this and include it in subsequent requests.

**How it works:**
- Reasoning is always captured from assistant API responses and included in subsequent requests, so the model always sees its previous thinking.
- Reasoning is always persisted in the `reasoning` column of the `messages` table in SQLite.
- Tool call and tool result messages are always stored in the database.
- Reasoning is included in token estimation for context trimming.
- Reasoning is **not** part of `text_content()` — it's a separate field, so embeddings and tool input don't include it.

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

**Conversation context:** Heartbeat tasks load `conversation.heartbeat_recent_messages_window_size` messages from the conversation history (falls back to `recent_messages_window_size` if unset). Set to `0` to give heartbeat tasks no conversation history. This lets you control how much context background tasks see independently from regular messages.

**How it works:**
- Users (or the LLM) add tasks via `add_task(description)`.
- `list_tasks()` shows pending tasks, `remove_task(id)` removes them.
- A background **scheduler** loop runs every 60s, scanning all chat directories for `chats/<chat_id>/tasks.yaml`.
- For each chat with pending tasks, a **dedicated timer loop** is spawned with that chat's configured interval (in seconds). If the chat has no custom interval, the global default is used. DMs always use the global default since they can't be preconfigured.
- Each per-chat loop independently: picks the oldest untried task → runs the LLM agent → sleeps for its interval → repeats. If heartbeat is disabled (interval = 0) the loop exits and will be respawned on the next scheduler scan if it becomes enabled again. If the agent completes all tasks, the loop exits immediately so the chat becomes eligible for re-discovery when new tasks are added.
- The agent uses bash, MCP tools, and task tools to complete the work. Each task is processed **at most once per cycle**, preventing re-grinding if a task cannot be completed yet (e.g. waiting for a download). If a task remains uncompleted (no `remove_task` call), it is **skipped** and the cycle moves on to the next untried task in the list. The cycle exits only when every remaining task has been tried at least once.
- The agent may send Telegram messages via the **`send_message` tool** when a task requires it — for example, to report completion or deliver results (e.g. *"Download this file, then tell me when it's done"*). It should NOT spam progress updates (e.g. "63% done") — only final or actionable results, at most once per task. If a task does not require messaging, it completes silently.
- Task processing runs in its own task — does not block message handling.

**Tools:** `add_task`, `list_tasks`, `remove_task`, `send_message`.

---

### 4.7a Reminders

Reminders fire at a **specific time** (ISO 8601 timestamp), unlike tasks which are state-dependent. Each chat can have a reminder list (`chats/<chat_id>/reminders.yaml`).

**Decision: reminder vs task:**
- Use `create_reminder` when the user wants something at a **known time** (e.g. "remind me tomorrow at 3pm to call mom").
- Use `add_task` when the trigger is **state-dependent** (e.g. "tell me when the stock hits $100" — the exact time is unknown).

**How it works:**
- The LLM calls `create_reminder(description, trigger_at, action?)` — `trigger_at` is an ISO 8601 timestamp in UTC, converted from the user's natural language.
- `action` is optional: if set, the bot performs the action (e.g. "look up mom's phone number from memory and include it") when the reminder fires, before sending the message.
- `list_reminders()` shows pending reminders with their trigger times.
- `remove_reminder(id)` removes a pending reminder.
- The background **scheduler** loop (every 60s) scans for due reminders (trigger_at in the past) across all chats. Reminders fire **independently** of heartbeat interval settings — they always fire when due.
- **No action**: the description is sent as a message (`⏰ Reminder: ...`) and the reminder is removed.
- **Has action**: a one-off LLM agent runs to perform the action, sends the result via `send_message`, then the reminder is removed.
- Data is stored per-chat in `chats/<chat_id>/reminders.yaml` (YAML, same pattern as `tasks.yaml`).

**Tools:** `create_reminder`, `list_reminders`, `remove_reminder`.

---

### 4.7b Todo List

A human-focused todo list — simple items the user wants to remember or track. Unlike tasks (which the bot works on autonomously) or reminders (which fire at a specific time), todos are just a checklist for the human.

**How it works:**
- The LLM calls `create_todo(description)` to add an item.
- `list_todos()` returns all todos with their UUIDs, descriptions, completed status, and timestamps.
- `edit_todo(id, description?, completed?)` updates a todo — change the description or toggle done/not done.
- `delete_todo(id)` removes a todo permanently (distinct from marking as completed).
- Users can list todos via the `/todos` command. `/todos details` shows full details (UUIDs, timestamps).
- Data is stored per-chat in `chats/<chat_id>/todos.yaml` (YAML, same pattern as `tasks.yaml`).

**Tools:** `create_todo`, `list_todos`, `edit_todo`, `delete_todo`.

**Command:** `/todos` — lists all todos for the current chat with completion status (✅ / ⬜). `/todos details` — full detail view with UUIDs, created/updated timestamps.

---

### 4.8 Memory System

#### Short-term (conversation context)

- Only the **current user message** is sent to the LLM with each request, along with the system prompt. The current user message includes a Telegram metadata prefix with sender identity and sent timestamp. Previous messages are stored persistently in **SQLite** (`glowbot_data/conversations.db`) but not transmitted unless explicitly requested.
- The bot provides a **`get_recent_messages(count)`** tool that queries the database and returns the last N messages. The LLM should call this when it needs to recall earlier parts of the conversation.
- The `conversation_window` config value controls the query `LIMIT` (default: 20). Older messages remain in the database but are excluded from default context.
- History **survives bot restarts** because it's stored in SQLite, not in-memory.
- Each message is a row with columns: `chat_id`, `role`, `content` (JSON), `name`, `tool_calls` (JSON), `tool_call_id`, `created_at`. This schema supports adding an `embedding` column later for vector search (Phase 2 RAG).

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

#### Vector memory (RAG) — initial implementation

- Configure `embedding_model` (e.g. `"openai/text-embedding-3-small"`) to enable conversation embedding.
- Every message is automatically embedded via OpenRouter's embeddings API and stored in the `message_embeddings` table (BLOB of f32 little-endian bytes, model name tagged per row).
- On startup: embeddings from a different model are cleaned up; then an async background task backfills any unembedded messages (500ms delay between calls, console/log progress).
- `embedding_search_limit` config (default 1000) caps how many recent embeddings are loaded for similarity search.
- LLM tool `search_conversations(query, count?)` — embeds the query, runs cosine similarity against stored embeddings for the chat, returns top-K results with content and similarity scores.
- Dimension-agnostic: BLOB storage handles any model's vector size. Model filtering ensures only same-model vectors are compared.
- Vectors are loaded into RAM for similarity computation — ~59 MB per 10k embeddings with `text-embedding-3-small` (1536 dims).

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

### 4.10 Tool Call Result Size Limit

The optional `conversation.max_tool_result_chars` config setting caps tool call results by character length. When set (e.g. `8000`), any tool result exceeding the limit is replaced with an error message telling the LLM the result was too big and suggesting it filters the output (jq, grep, head, awk), narrows its query, or uses a different tool. When unset (default), there is no limit.

### 4.11 Tool Call Logging

Every tool invocation is logged for debugging and audit:
- Written to `glowbot_data/tool_calls.log` (append-only).
- Format: `[timestamp] tool_name | args: <json> | result: <first 200 chars>`
- Also logged to stdout via `log::info!` for Docker log visibility.
- Log writes are best-effort — they never block or fail message processing.

---

### 4.12 Commands & Permissions

Commands are Telegram bot commands (`/command`) used for control and settings. **On startup, the bot registers these with Telegram via `setMyCommands`** so they appear in the in-chat bot menu and autocomplete.

| Command | Purpose | Requires |
|---------|---------|----------|
| `/status` | Show current config + context usage for this chat | command whitelist |
| `/model` | Show or set the current model; OpenRouter supports `:nitro`, `:floor`, `:free`, while Codex opens its subscription-model picker | command whitelist |
| `/models` | Browse and temporarily switch models via inline keyboard (OpenRouter catalog or Codex picker) | command whitelist |
| `/model_default` | Reset temporary model override to config default (alias: `/model_reset`) | command whitelist |
| `/tasks` | List all pending tasks for this chat | command whitelist |
| `/todos` | List all human todos for this chat | command whitelist |
| `/reminders` | List all pending reminders for this chat | command whitelist |
| `/run` | Trigger the task agent to run immediately for this chat | command whitelist |
| `/new` | Reset conversation context — stores a cutoff timestamp; only messages after this point are included in future context | command whitelist |
| `/prompt` | Show the system prompt that would be sent to the LLM | command whitelist |
| `/tools` | List all available tools for this chat (built-in + MCP) | command whitelist |
| `/config` | Show the current config with sensitive fields redacted | command whitelist |
| `/config_schema` | Show the JSON Schema for all config fields | command whitelist |
| `/stop` | Interrupt ongoing LLM processing for this chat | none (always available) |

#### `/status` output format

```
Chat ID: -1234567890
Provider: openrouter
Model: anthropic/claude-sonnet-4
Context usage: 37k/252k (15%)
Interaction mode: EveryMessage
Interaction whitelist: everyone
Command whitelist: enabled
```

- `Context usage` shows the last known prompt token count against the model's context limit, with percentage.
- Before any messages are processed, or if model metadata is unavailable, it shows `unknown`.

#### Whitelist rules

| Whitelist | Default | Controls |
|-----------|---------|----------|
| `interaction_whitelist` | Empty = **everyone** | Who the bot talks to / responds to |
| `commands_enabled` | `false` | Whether bot commands (`/status`, `/stop`, `/tasks`, `/todos`, `/run`) work |

Whitelists contain Telegram user IDs.

---

## 5. MVP Scope (v1)

### Must have

- [x] Telegram messaging (groups + DMs, long-polling)
- [x] OpenRouter LLM integration with multi-turn tool-use loop
- [x] OpenAI Codex subscription integration using official Codex CLI OAuth credentials
- [x] Per-chat and per-DM provider overrides between OpenRouter and Codex
- [x] YAML configuration with per-chat model, interaction mode, and conversation window
- [x] Simple skills (`skills/<name>/skill.md` with frontmatter)
- [x] Bash tool (oneshot, container-isolated, 30s timeout)
- [x] `read_memory` / `update_memory` structured tools for per-user `.md` memory
- [x] `create_skill` / `update_skill` structured tools for skill management
- [x] Tool call logging to `tool_calls.log`
- [x] MCP server integration for external tool discovery
- [x] Heartbeat task system with autonomous background agents
- [x] Reminders system — time-based triggers with optional LLM actions, independent of heartbeat
- [x] Human todo list — per-chat checklist with create/list/edit/delete tools and `/todos` command
- [x] Per-user `.md` memory with YAML frontmatter, freeform body
- [x] Memory frontmatter injected into system prompt; full file readable via tools
- [x] `/status`, `/tasks`, `/stop` commands
- [x] `/status` shows context usage (prompt tokens / model limit + percentage)
- [x] Model context lengths fetched and cached from OpenRouter on startup
- [x] Interaction & command whitelists per chat
- [x] Git auto-commit + push on every data write (with safe.directory and identity setup)
- [x] Docker deployment with `glowbot_data/` as a volume
- [x] GitHub CI/CD with ≥95% test coverage enforced
- [x] Conversation history (stored in SQLite, retrievable via `get_recent_messages` tool, configurable window size)
- [x] LLM reasoning/thinking capture and storage (always on)
- [x] Typing indicator while LLM is processing
- [x] MarkdownV2 rendering via `telegram-markdown-v2` crate with plain text fallback
- [x] Media ingest: images and audio (native multimodal or fallback conversion)
- [x] `ask_advisor` tool — optional second-opinion from a larger/smarter model via `openrouter.advice_model` config

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

### Short-term conversation context

- Messages are stored in **SQLite** (`conversations.db`) — one row per message — so history **survives restarts**.
- The bot only sends the **current user message** to the LLM by default (plus the system prompt). Earlier messages are not included automatically, keeping latency low.
- When the user references something earlier, the LLM calls `get_recent_messages(count)` to fetch the needed context from the database.
- The `conversation_window` config controls the query `LIMIT` (default: 20), acting as a sliding window. Older messages stay in the DB but are excluded from default context.
- This one-row-per-message design supports adding an `embedding` column later for vector/RAG search (Phase 2).

### Markdown rendering
- Telegram messages must be sent with a parse mode (`MarkdownV2` or `HTML`) or they render as plain text. Default `send_message` has no parse mode.
- **Do not hand-roll MarkdownV2 escaping.** Telegram's V2 has many reserved characters (`_ * [ ] ( ) ~ \` > # + - = | { } . !`) and context-dependent escaping rules (e.g. `-` at line start vs mid-word). LLMs will constantly hit edge cases.
- **Use the `telegram-markdown-v2` crate** (`convert()`). It parses the LLM's Markdown output and produces properly escaped MarkdownV2, handling headings→bold, lists→Unicode bullets, code blocks, links, and inline formatting correctly.
- Fall back to plain text if conversion fails (rare with a good crate).

### Bot command menu registration
- **Always call `setMyCommands`** after confirming the bot identity on startup. Without this, users won't see the command list when they type `/` or open the bot menu. The command descriptions are registered globally for all chats.
