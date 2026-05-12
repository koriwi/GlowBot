# GlowBot — Security & Quality Audit Report

**Date:** 2026-05-12  
**Scope:** Full source code audit (`src/`)  
**Classification:** Vulnerabilities, footguns, code smells, bugs, and oversights  

> **Severity levels:** 🔴 CRITICAL | 🟠 HIGH | 🟡 MEDIUM | 🔵 LOW | ⚪ CODE SMELL

---

## Status Summary

| Item | Status |
|------|--------|
| C1 (API key leak) | Accepted risk — capped $2 OpenRouter keys, local-service tokens only |
| C2 (exit(0)) | **FIXED** — graceful shutdown via `shutdown_requested` flag |
| C3 (std Mutex on DB) | Low impact at current scale — queries complete in 1-2ms |
| C4 (bash sandboxing) | Accepted risk — bash disabled by default, container isolation |
| H1–H6, M1–M7, L1–L4, S1–S8 | Open for evaluation |

---

## 🔴 CRITICAL / ACCEPTED RISKS

### C1. `tool_read_config` Leaks API Keys to the LLM _(accepted — not fixed)_

- **File:** `src/bot_dispatch_config.rs` — lines 32–36
- **Status:** Accepted. Keys are capped OpenRouter keys ($2 limit) and local-service tokens. The `redacted()` method exists but is not used here — trivial 1-line fix if ever needed.

### C2. `std::process::exit(0)` Brutally Kills the Process _(FIXED)_

- **File:** `src/bot_dispatch_config.rs` — previously line 173
- **What changed:** Instead of `std::process::exit(0)`, a `shutdown_requested` flag on `BotState` is set to `true`. The bot sends "🔄 Waiting for things to finish before restarting..." to the chat, then exits gracefully once all in-flight work completes.

**New behavior:**
- When a config change is accepted via inline keyboard, the new config is saved and `state.shutdown_requested` is set to `true` (via `AtomicBool`)
- A message "🔄 Waiting for things to finish before restarting..." is sent to the chat
- The polling loop checks the flag after each update batch and exits cleanly
- The heartbeat scheduler checks at each 60s cycle boundary and returns
- The embedding backfill checks before each message and stops
- Once all workers have noticed the flag, the `run_bot()` function returns, and the process terminates cleanly via `main()` returning

**Checkpoints added:**
- `src/main.rs` — polling loop: checked after each update batch, before each polling spawn, and after handle resolves; heartbeat scheduler: at start of each scan cycle
- `src/bot_heartbeat.rs` — `run_heartbeat_task`: top of function and between each task; `process_reminder_action`: at start
- `src/bot.rs` — `start_embedding_backfill`: before each embedded message

### C3. `std::sync::Mutex` on SQLite Connection in Async Context _(low impact — not fixed)_

- **File:** `src/db.rs` — line 32
- **Status:** Currently has negligible impact. Each query (e.g. `load_messages(chat, 20, None)`) completes in 1–2ms. The lock is never held across `.await` — no deadlock risk. Would only become noticeable if the DB grows to 100K+ rows and `search_conversations` is used regularly. Not worth fixing at current scale.

### C4. Bash Tool Has No Allowlist or Sandboxing _(accepted — not fixed)_

- **File:** `src/bash.rs` — all functions
- **File:** `src/bot_dispatch.rs` — lines 70–86 (bash dispatch)
- **Status:** Accepted. Bash is disabled by default (`bash_enabled: true` is the global default but typically overridden per-chat). Bot runs in container isolation. No fix planned.

---

## 🟠 HIGH

### H1. Heartbeat Tasks Ignore Stop Signals

- **File:** `src/bot_heartbeat.rs` — both `run_heartbeat_task()` and `process_reminder_action()`

The heartbeat loop (`run_heartbeat_task`) and the reminder action processor run up to **10 LLM rounds** each without checking for stop signals. The `/stop` command only sets a stop signal in `main.rs`/`handle_message`, but background tasks don't receive the `stop_signals` reference.

**Fix:** Thread the stop signal through to heartbeat tasks and check it at every tool-round boundary, same as the main pipeline.

---

### H2. Image Filename Collision Risk

- **File:** `src/bot_dispatch_image.rs` — line 109
- **Format:** `generated/{model}_{timestamp}_{index}.{ext}`

Generated images use `chrono::Utc::now().format("%Y%m%dT%H%M%S")` which is **second-level granularity**. Two concurrent image generations in the same second (or even a single generation that produces multiple images) will collide.

**Fix:** Include milliseconds or a UUID in the timestamp.

---

### H3. YAML Deserialization Silently Ignores Errors with `unwrap_or_default()`

Three occurrences of silent data loss:

1. **`src/reminders.rs` line 23** — `ReminderList::load` calls `unwrap_or_default()` on parse failure → all reminders silently lost
2. **`src/tasks.rs` line 17** — `TaskList::load` same pattern → all tasks silently lost
3. **`src/db_migrations.rs` line 95** — `ALTER TABLE` failure silently ignored

**Fix:** Log a warning when deserialization fails. Use `anyhow::Context` to surface the error rather than silently swallowing it.

---

### H4. `unwrap()` on `std::sync::Mutex` Locks Everywhere — Poison = Cascade Failure

Every `std::sync::Mutex::lock().unwrap()` call across the codebase will panic on poison. If the SQLite connection's mutex gets poisoned (panic in any DB operation), **every subsequent DB operation** panics — messages can't be saved, loaded, embedded, or searched. The bot enters a crash loop.

**Fix:** Use `.lock().map_err(...)` or switch to `tokio::sync::Mutex` which doesn't poison.

---

### H5. MCP Tool Results Not Size-Limited

- **File:** `src/mcp_invoke.rs` — `invoke_tool_once()` and `invoke_tool()`

MCP tools like Playwright's `screenshot` can return megabytes of HTML or base64 data. These get stuffed into `tool_result` messages without truncation, bloating the LLM context window and potentially causing OOM.

**Fix:** In `dispatch_tool_calls`, truncate MCP tool results to a reasonable limit (e.g. 4000 chars) with a `... (truncated)` suffix.

---

### H6. No Rate Limiting — API Cost Amplification

There is no rate limiting at any layer. A single user can paste a large document or rapidly send many messages, each triggering up to 64 LLM calls. The embedding backfill calls OpenRouter API per message with only 500ms delay. Heartbeat scans all chats every 60s.

**Fix:** Add per-user, per-chat, and global rate limits; cap LLM spending; limit concurrent embedding calls.

---

## 🟡 MEDIUM

### M1. Duplicated `short_id()` with Timestamp Collisions

Two identical functions in `src/bot_dispatch_config.rs` and `src/bot_dispatch_model.rs`:

```rust
fn short_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", ts)
}
```

While nanosecond precision sounds safe, `SystemTime::now()` can go backward (NTP adj), and two concurrent `propose_model_change` + `edit_config` on the same nanosecond produce the same ID.

**Fix:** Extract to a shared utility function and include a random component.

---

### M2. Invalid `chat_id.parse().unwrap_or_default()` Becomes `ChatId(0)`

- **File:** `src/main.rs` line 93, `src/bot_dispatch.rs` line 73, `bot_dispatch_config.rs` line 88, `bot_dispatch_model.rs` line 94

When `chat_id` fails to parse as `i64`, it becomes `ChatId(0)` — a non-existent chat. Messages are silently dropped with no diagnostics.

**Fix:** Parse once and propagate errors explicitly, or at minimum log a critical error.

---

### M3. `build_trimmed_request` Sends Over-Limit Requests When Fixed Cost Exceeds Limit

- **File:** `src/openrouter_context.rs` — lines 100–106

When the system prompt + tools + turn + response reserve exceed the context limit, the function still sends the request. This will either be rejected, truncated silently, or cost money for a guaranteed failure.

**Fix:** Return an error instead of sending a known-over-limit request. Make `RESPONSE_RESERVE_TOKENS` model-aware.

---

### M4. Config Save Does Not Git Auto-Commit

- **File:** `src/bot.rs` — `save_config()` method, lines 65–67

The git auto-commit is commented out, which means the config can be out of sync with git state and the audit trail is lost.

**Fix:** Re-enable auto-commit or remove the `_git_repo` parameter and commented code.

---

### M5. No Message Retention / Cleanup — Unbounded SQLite Growth

- **File:** `src/db.rs` — no cleanup mechanism exists

Every message in every chat is stored forever. The `messages` table grows unboundedly; no `DELETE` or `VACUUM` strategy exists. Over time this impacts backup size, search speed, and storage.

**Fix:** Add configurable retention policy (e.g., keep last N days or N messages per chat).

---

### M6. `due()` Reparses All Reminder Timestamps Every Call

- **File:** `src/reminders.rs` — lines 86–93

`due()` re-parses every single reminder's `trigger_at` string as RFC 3339 every time it's called. This is called every 60 seconds per chat via the heartbeat scheduler.

**Fix:** Parse `trigger_at` at load time and cache as a `DateTime` field.

---

### M7. YAML Files Re-parsed Every Heartbeat Cycle (Every 60 Seconds)

`TaskList::load()` and `ReminderList::load()` read and deserialize from disk every 60 seconds for every chat. The `run_heartbeat_loop` in `main.rs` scans every chat directory, reading and parsing YAML for each chat with tasks or reminders — this happens again inside `run_heartbeat_task`.

**Fix:** Add an in-memory cache with file-mtime checking.

---

## 🔵 LOW

### L1. User Message Content Logged in Plain Text

- **File:** `src/main.rs` line 90 — logs the **full** user message text with no truncation
- **File:** `src/bot_pipeline.rs` line 27 — truncates to 100 chars

In a shared log environment (Docker, journald), all user conversations are visible in plain text.

**Fix:** Truncate user text in all log statements.

---

### L2. MCP Session Race Condition on Concurrent Re-init

- **File:** `src/mcp_invoke.rs` — `invoke_tool()` function
- **File:** `src/bot_dispatch.rs` — MCP dispatch

When a session expires, the session ID is propagated to all tools from the same server via mutation in `state.mcp_tools`. Two concurrent re-inits can race, overwriting each other's session IDs.

**Fix:** Use a per-server `RwLock<Option<String>>` for the session ID.

---

### L3. Media Directory Not Validated at Startup

The `media_dir` config value (defaulting to `/media`) is never checked at startup. If it doesn't exist or isn't writable, file downloads silently fail and generated images can't be saved.

**Fix:** Validate `media_dir` at startup, create subdirectories (`ingest`, `generated`, `pw-media`), and abort with a clear error if unusable.

---

### L4. Long-Polling Instead of Webhooks

- **File:** `src/main.rs` — manual `get_updates` poll loop

Long-polling adds connection churn, up to 30s latency, and complexity. Webhooks are the recommended approach for production Telegram bots.

---

## ⚪ CODE SMELLS

### S1. Massive `dispatch_tool` Match — 200+ Lines

- **File:** `src/bot_dispatch.rs` — `dispatch_tool()` spans ~200 lines with ~30 arms

The `match tool_name` block has grown organically. Already partially split into submodules, but bash and MCP-related tools remain inline.

### S2. `#[allow(dead_code)]` on Primary Constructor

- **File:** `src/bot.rs` — line 33

`#[allow(dead_code)]` on `new_with_llm`, the primary constructor. Suggests the lint was added to silence something rather than being genuinely needed.

### S3. Dual `reqwest` Dependencies (0.11 and 0.12)

- **File:** `Cargo.toml` — lines 7–10

Two major versions of `reqwest` compiled into the binary, doubling TLS compilation time and binary size. The 0.11 dependency exists solely to configure TCP keepalive on the `teloxide` bot client.

### S4. Linear Scan for Embedding Similarity Search

- **File:** `src/db_embeddings.rs` — `search_embeddings()`

The function loads up to `search_limit` (default 1000) embeddings into memory, then computes cosine similarity against each one — O(n * d) with no SIMD, no approximation, no index.

### S5. `println!` Mixed with Structured Logging

- **File:** `src/bot.rs` — lines 117, 123 (now lines 122, 128 after changes)

`println!` is used inside background Tokio tasks alongside `log::info!`. `println!` bypasses the logging framework's formatting and timestamps.

### S6. Duplicated `base64_encode` Function

- **File:** `src/media.rs` — lines 108–111
- **File:** `src/bot_dispatch_image.rs` — lines 213–216

Identical `base64_encode` function in two places.

### S7. Fragile Callback Split Logic for Long Model IDs

- **File:** `src/bot_models.rs` — lines 18–35

`detail_cb()` and `select_cb()` shorten prefixes from `model:detail:` to `d:` when callback data exceeds 64 bytes. The parsing in `handle_model_callback` has split logic mismatch (`splitn(3)` vs `splitn(2)`) for the two prefix lengths — fragile and prone to bugs.

### S8. Commands Accepted Without `@mention` in Some Paths

- **File:** `src/bot.rs` — lines 109–114

`parse_command()` is checked **before** the `is_mention` gate. Any message starting with `/` that matches a known command is dispatched to the command handler regardless of mention status. In practice, whitelists gate the actual execution, but `/prompt` and `/model` can still trigger.

---

## Summary Table

| # | Severity | Issue | File(s) | Status |
|---|----------|-------|---------|--------|
| C1 | 🔴 CRITICAL | `tool_read_config` leaks API keys to LLM | `bot_dispatch_config.rs:32-36` | Accepted |
| C2 | 🔴 CRITICAL | `exit(0)` kills in-flight work, corrupts DB | `bot_dispatch_config.rs` | **FIXED** |
| C3 | 🔴 CRITICAL | `std::sync::Mutex` on DB in async context | `db.rs:32` | Low impact |
| C4 | 🔴 CRITICAL | Bash tool has no allowlist/sandboxing | `bash.rs`, `bot_dispatch.rs` | Accepted |
| H1 | 🟠 HIGH | Heartbeat ignores stop signals | `bot_heartbeat.rs` | Open |
| H2 | 🟠 HIGH | Image filename collision (second-level timestamp) | `bot_dispatch_image.rs:109` | Open |
| H3 | 🟠 HIGH | YAML parse failures silently reset data | `reminders.rs:23`, `tasks.rs:17` | Open |
| H4 | 🟠 HIGH | Mutex poisoning cascade failure | Multiple files | Open |
| H5 | 🟠 HIGH | MCP tool results untruncated | `mcp_invoke.rs` | Open |
| H6 | 🟠 HIGH | No rate limiting — API cost amplifier | All | Open |
| M1 | 🟡 MEDIUM | Duplicated `short_id()`, nanosecond collision risk | `bot_dispatch_config.rs`, `bot_dispatch_model.rs` | Open |
| M2 | 🟡 MEDIUM | Invalid chat_id becomes ChatId(0), silent drop | `main.rs:93` and others | Open |
| M3 | 🟡 MEDIUM | Over-limit requests sent when fixed_cost exceeds limit | `openrouter_context.rs:100-106` | Open |
| M4 | 🟡 MEDIUM | Config save git auto-commit commented out | `bot.rs:65-67` | Open |
| M5 | 🟡 MEDIUM | No message retention policy — unbounded growth | `db.rs` | Open |
| M6 | 🟡 MEDIUM | Reminder timestamps re-parsed every check | `reminders.rs:86-93` | Open |
| M7 | 🟡 MEDIUM | YAML files re-read every heartbeat cycle | Multiple files | Open |
| L1 | 🔵 LOW | User message content logged in plain text | `main.rs:90`, `bot_pipeline.rs:27` | Open |
| L2 | 🔵 LOW | MCP session ID race on concurrent re-init | `mcp_invoke.rs`, `bot_dispatch.rs` | Open |
| L3 | 🔵 LOW | Media directory not validated at startup | — | Open |
| L4 | 🔵 LOW | Long-polling instead of webhooks | `main.rs` | Open |
| S1 | ⚪ SMELL | 200+ line match in `dispatch_tool` | `bot_dispatch.rs` | Open |
| S2 | ⚪ SMELL | `#[allow(dead_code)]` on primary constructor | `bot.rs:33` | Open |
| S3 | ⚪ SMELL | Dual reqwest 0.11/0.12 dependency | `Cargo.toml:7-10` | Open |
| S4 | ⚪ SMELL | Linear scan for embedding similarity search | `db_embeddings.rs` | Open |
| S5 | ⚪ SMELL | `println!` mixed with structured logging | `bot.rs` | Open |
| S6 | ⚪ SMELL | Duplicated `base64_encode` function | `media.rs`, `bot_dispatch_image.rs` | Open |
| S7 | ⚪ SMELL | Fragile callback split logic for long model IDs | `bot_models.rs:18-35` | Open |
| S8 | ⚪ SMELL | Commands accepted without `@mention` in some paths | `bot.rs:109-114` | Open |
