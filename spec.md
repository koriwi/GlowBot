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

---

## 4. Core Systems

### 4.1 Telegram Integration

- Receives messages via long-polling or webhook (configurable, poll default).
- Sends responses as plain text or Markdown.
- Tracks chat context: group vs. DM, user identity (ID + username).

#### Interaction modes

| Mode | Behavior |
|------|----------|
| `every_message` | Bot reads every message and may respond autonomously. |
| `mention_only` | Bot only responds when explicitly @mentioned or replied to. |

`/commands` are always recognised regardless of interaction mode.

---

### 4.2 LLM Integration (OpenRouter)

- Sends chat context + tools + skills + memory to the configured model.
- Model is set per chat in config (overridable via `/model`).
- Handles tool-use responses (currently only `bash`).

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
- It can write simple `skill.md` files directly.
- (Phase 2) It can generate and compile Rust skills.

---

### 4.4 Memory System

#### Short-term (conversation context)

- Standard sliding-window of recent messages sent to the LLM with each request.

#### Long-term (per-user `.md` files) — MVP

```
chats/
  <chat_id>/
    <user_id>.md
    <user_id>.md
    ...
```

Each file uses YAML frontmatter for structured metadata, followed by a freeform Markdown body:

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

- The bot **autonomously** updates the frontmatter `description` and appends to the body with timestamped facts it considers worth remembering.
- **Only the YAML frontmatter** (user_id, username, call_name, description) is injected into the system prompt — keeping context usage minimal.
- The bot can read the full file body on demand via its bash tool if it needs deeper recall.
- The `call_name` is what the bot uses when addressing the user in conversation. It is inferred autonomously on first encounter, but the bot may ask the user directly if it is unsure what to call them.
- A separate file per chat means the same user can have different context in different chats.

#### Vector memory (Phase 2)

- All messages are embedded and stored in SQLite with a vector extension.
- The bot queries relevant past messages on demand for RAG.
- Per-user `.md` files remain as canonical long-term memory but are also embedded.

---

### 4.5 Bash Tool

The bot has exactly **one tool**: raw bash execution.

- Commands run in a subprocess inside the Docker container.
- No interactive/session-based commands — each invocation is stateless (oneshot).
- The container provides isolation; no additional sandboxing is required.
- The LLM uses bash to invoke skills, manipulate files, query APIs, read full memory files, etc.

---

### 4.6 Commands & Permissions

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
- [x] OpenRouter LLM integration
- [x] YAML configuration with per-chat model & interaction mode
- [x] Simple skills (`skills/<name>/skill.md` with frontmatter)
- [x] Bash tool (oneshot, container-isolated)
- [x] Per-user `.md` memory with frontmatter header, freeform body
- [x] Memory frontmatter injected into system prompt; full file readable via bash
- [x] `/model`, `/mode`, `/reload`, `/status` commands
- [x] Interaction & command whitelists per chat
- [x] Git auto-commit + push on every data write
- [x] Docker deployment with `glowbot_data/` as a volume
- [x] GitHub CI/CD with ≥95% test coverage

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
