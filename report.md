# GlowBot Codebase Audit Report

**Generated:** 2026-05-12  
**Scope:** All source files in `/opt/custom_docker/GlowBot/src/` plus `Cargo.toml`, `config.example.yaml`, and `spec.md`  
**Auditor:** Automated static analysis

## Table of Contents

1. [CRITICAL: Security Vulnerabilities](#1-critical-security-vulnerabilities)
2. [CRITICAL: Data Loss & Corruption](#2-critical-data-loss--corruption)
3. [HIGH: Bugs](#3-high-bugs)
4. [MEDIUM: Code Smells](#4-medium-code-smells)
5. [LOW: Oversights & Design Issues](#5-low-oversights--design-issues)
6. [Dependency Issues](#6-dependency-issues)
7. [Testing Coverage Gaps](#7-testing-coverage-gaps)

---

## 1. CRITICAL: Security Vulnerabilities

### 1.1 Process Termination via Config Change (C1)

**File:** `src/bot_dispatch_config.rs`, line ~277  
**Severity:** CRITICAL — Denial of Service, Data Loss

```rust
tokio::spawn(async {
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    log::info!("Exiting for restart after config change.");
    std::process::exit(0);
});
```

After a user accepts a config change via inline keyboard, the bot spawns a background task that calls `std::process::exit(0)` after 500ms. This:

- **Kills all in-flight requests** — any ongoing LLM calls, database writes, heartbeat tasks, or message processing are immediately terminated.
- **Drops uncommitted SQLite transactions** — SQLite WAL writes that haven't been checkpointed or committed are lost.
- **Skips git push** — the git auto-commit for the config change may not have completed.
- **Skips pending Telegram callback answers** — the callback query edit message response may not be sent.

**Recommendation:** Use a graceful shutdown mechanism (signal propagation via `tokio::sync::watch` or `CancellationToken`) instead of `std::process::exit`. Let the main polling loop drain naturally.

---

### 1.2 MCP Tool Invocation Without Timeout (C2)

**File:** `src/mcp_invoke.rs`, `invoke_tool_once()` — line ~25-26  
**Severity:** HIGH — Resource Exhaustion

```rust
let client = reqwest::Client::new();
// no timeout set
```

The `invoke_tool_once` function creates a raw `reqwest::Client` with **no timeout**. If an MCP server is slow or hangs, the tool call blocks the Tokio task indefinitely. The MCP client does not use the timeout set on `OpenRouterClient` (120s).

Meanwhile in the main pipeline, the tool-loop does not have a per-round timeout either. A single hung MCP call will block the entire pipeline for that chat forever.

**Recommendation:** Create the HTTP client with a reasonable timeout (e.g. `std::time::Duration::from_secs(60)`) in `invoke_tool_once`. Also consider adding a total timeout around the tool-loop.

---

### 1.3 Sensitive Data in Debug Logs (C3)

**File:** `src/mcp_client.rs`, line ~64  
**Severity:** HIGH — Credential Exposure

```rust
format!("{}...{}", &key[..4], &key[key.len()-4..])
```

While this masks the middle of the API key, it still emits the first 4 and last 4 characters at `log::debug!` level. This leaks partial credential information into logs. The same log line also emits the auth status (`self.server.api_key.is_some()`).

Additionally, `tool_calls.log` (written to `data_dir.join("tool_calls.log")`) records **all tool arguments and results** including:
- Bash commands and their output (could contain passwords, tokens, DB queries)
- Memory read/write contents
- MCP tool inputs and outputs
- Config file contents

This file grows unboundedly with no rotation mechanism.

**Recommendations:**
- Remove key-prefix logging entirely; just log `"auth=true"` or `"auth=false"`.
- Add log rotation (e.g. `logrotate` or a simple size cap with truncation).
- Add a config option to disable tool logging or redact sensitive fields.

---

### 1.4 TOCTOU Race in MCP Blacklist Check (C4)

**File:** `src/bot_dispatch.rs`, the `mcp_` match arm  
**Severity:** MEDIUM — Privilege Escalation

```rust
match tool_idx.map(|idx| {
    (idx, s.config.is_mcp_server_allowed(chat_id, &s.mcp_tools[idx].server_name))
}) {
    Some((_, false)) => format!("MCP tool blacklisted for this chat: {}", tool_name),
    Some((idx, true)) => {
        let mut tc = s.mcp_tools[idx].clone();
        let server = tc.server_name.clone();
        drop(s); // <-- LOCK IS DROPPED HERE
        let result = crate::mcp::invoke_tool(&mut tc, &args_clone).await;
        // ... after invoke, re-acquires lock to propagate session_id
```

There is a TOCTOU (time-of-check-time-of-use) race condition: the blacklist is checked while holding the mutex, then the lock is dropped, and the tool is invoked. In between, another operation could have modified the config (adding/removing the server from the blacklist). The subsequent session_id propagation also briefly re-acquires the lock.

**Recommendation:** While functionally low-risk (config changes are rare), document this as a known race, or move the blacklist check into the invoke path where it's enforced atomically.

---

## 2. CRITICAL: Data Loss & Corruption

### 2.1 Dead-Code Config Git Commit (D1)

**File:** `src/bot.rs`, `save_config()` — lines ~89-91  
**Severity:** HIGH — Audit Trail Broken

```rust
pub async fn save_config(&self) -> anyhow::Result<()> {
    let state = self.state.lock().await;
    let path = state.config_path();
    state.config.save(&path)?;
    drop(state);
    // self.git_repo
    //     .auto_commit("Update configuration via /command")?;
    Ok(())
}
```

The git auto-commit after config changes is **commented out**. Config changes via commands / config-diff acceptance will not be tracked in the git history. This also means the data directory's git log will be incomplete, defeating the purpose of having git versioning.

Additionally, the `/status` handler at `bot_commands.rs:~268` also doesn't trigger a git commit after `save_config`.

---

### 2.2 YAML Parse Errors Silently Swallowed (D2)

**Files:** `src/reminders.rs`, `src/tasks.rs`  
**Severity:** HIGH — Silent Data Loss

```rust
let list: Self = serde_yaml::from_str(&data).unwrap_or_default();
```

If the YAML file (reminders.yaml or tasks.yaml) gets corrupted — partial write, disk error, concurrent edit — the `serde_yaml::from_str` will fail and the error is silently swallowed with `unwrap_or_default()`. The user gets an empty task/reminder list, and their data is effectively lost. The corrupted file remains on disk, so the next write will overwrite the corrupted file with a clean empty state, making recovery impossible.

**Recommendation:** Log parse errors at `warn!` level so operators know. Even better: return the error and let the caller handle it, or rename the corrupted file and start fresh (preserving the original for recovery).

---

### 2.3 Race Condition in Embedding Backfill (D3)

**File:** `src/bot.rs`, `start_embedding_backfill()` — lines ~114-175  
**Severity:** MEDIUM — Duplicate/Colliding Embeddings

The backfill runs `find_unembedded_messages()` which does a LEFT JOIN to find messages without embeddings. But while the backfill is running (iterating through messages with 500ms delays), new messages are being saved by `process_with_llm_impl` which also spawns an embedding task. This means:

1. The backfill finds message_id=100 as "unembedded".
2. A new user message comes in, gets saved as message_id=105, and its embedding task starts.
3. The backfill is still iterating earlier messages and eventually reaches message_id=105.
4. Both the pipeline task and the backfill try to save embedding for message_id=105.

The `INSERT INTO message_embeddings` has no `UNIQUE` constraint on `(message_id, model)`, so duplicate entries can be created.

**Recommendation:** Add a `UNIQUE(message_id, model)` constraint to the `message_embeddings` table, or use `INSERT OR IGNORE` / `INSERT OR REPLACE`.

---

## 3. HIGH: Bugs

### 3.1 Telegram Message Char Limit vs Byte Limit (B1)

**File:** `src/bot_send.rs`, `split_for_telegram()`  
**Severity:** MEDIUM — Silent Truncation

```rust
const MAX_CHUNK_CHARS: usize = 4000;
```

Telegram's message limit is **4096 bytes**, not characters. The current code counts **Unicode code points** (`text.chars().count()`). A message containing many multi-byte characters (CJK, emoji, etc.) could be under 4000 characters but exceed 4096 bytes.

For example:
- 4000 CJK characters = ~12,000 bytes (3 bytes each in UTF-8)
- This would be well over Telegram's 4096 byte limit

The split logic would not split this message, and Telegram would reject it with a "MESSAGE_TOO_LONG" error.

**Recommendation:** Count bytes instead of characters, or use a more conservative limit (e.g. 3500 chars) as a heuristic.

---

### 3.2 `short_id()` Collision Risk (B2)

**Files:** `src/bot_dispatch_config.rs`, `src/bot_dispatch_model.rs`  
**Severity:** LOW-MEDIUM — Collision

```rust
fn short_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", ts)
}
```

Two calls within the **same nanosecond** (which is achievable on modern multi-core machines or under low load when the system clock hasn't ticked) will produce identical IDs. If two config changes are proposed simultaneously, the second will overwrite the first's pending entry.

**Recommendation:** Append a small random component (e.g. `rand::thread_rng().next_u32()`) or use the `uuid` crate already in dependencies.

---

### 3.3 `/stop` Race with Lock Acquisition (B3)

**File:** `src/main.rs`, `handle_message()` — lines ~112-119  
**Severity:** MEDIUM — Race Condition

```rust
// Acquire per-chat lock for normal message processing
let chat_lock = { ... };
let _guard = chat_lock.lock().await;

// Check if we were stopped while waiting for the lock.
```

The `/stop` command bypasses the per-chat lock. But another message that arrived *just before* `/stop` may already be holding the lock and processing. If that message has a long LLM pipeline (multiple tool rounds), it can continue running for a long time after the stop signal was set. The "check if stopped" inside `process_with_llm_impl` should eventually catch it, but there's a race window.

Also, the `/stop` handler sends a single `"⏹ Stop signal sent..."` message but doesn't wait for the running operation to actually stop. The user gets immediate feedback but the operation might continue for many more seconds.

**Recommendation:** Consider using a `tokio::sync::watch` channel for stop signals that the lock guard monitors, or add a brief wait in `/stop` for confirmation.

---

### 3.4 `strip_orphaned_tool_results` Resets on Each Assistant Message (B4)

**File:** `src/openrouter_context.rs`, `strip_orphaned_tool_results()`  
**Severity:** MEDIUM — Incorrect Message Dropping

```rust
if msg.role == "assistant" {
    if let Some(tcs) = &msg.tool_calls {
        open_ids.clear(); // <-- resets the set
        for tc in tcs {
            open_ids.insert(tc.id.clone());
        }
    }
    return true;
}
```

This function allows messages in sequence like: `assistant(tool_calls=[A,B])` → `tool(A)` → `tool(B)` → `assistant(tool_calls=[C])` → `tool(C)`. But because `open_ids` is cleared on every assistant message:

- Input: `[assistant(tc=[A]), tool(A), assistant(tc=[C]), tool(C)]`
  - Processing: assistant(A) opens {A}, tool(A) matches ✓, assistant(C) clears {A} and opens {C}, tool(C) matches ✓
  - Correct! ✓

- Input: `[assistant(tc=[A]), tool(B)]` (orphaned: B not in [A])
  - Processing: assistant(A) opens {A}, tool(B) doesn't match → stripped ✓
  - Correct! ✓

- Input: `[tool(A)]` (no preceding assistant)
  - Processing: open_ids is empty, tool(A) is stripped ✓
  - Correct! ✓ (though arguably a tool result without a preceding assistant is always orphaned)

However, consider this edge case: `[assistant(tc=[A]), tool(A), tool_result(X)]` where X has no tool_call_id:
- Processing: assistant(A) opens {A}, tool(A) matches ✓, tool_result(X) has no tool_call_id → falls through to the non-tool, non-assistant case → always kept.
- This could keep a tool_result that has no matching tool_call_id because the `tool_call_id` field is `None`. This should probably also be stripped.

**Recommendation:** Add a check for messages with role "tool" but no `tool_call_id` — they are malformed and should be stripped.

---

### 3.5 `GLOWBOW_SCHEMA_DIR` Fallback to CWD (B5)

**File:** `src/bot.rs`  
**Severity:** MEDIUM — Migration Failure in Production

```rust
let schema_dir = std::env::var("GLOWBOT_SCHEMA_DIR")
    .map(std::path::PathBuf::from)
    .unwrap_or_else(|_| std::path::PathBuf::from("schema"));
```

If `GLOWBOT_SCHEMA_DIR` is not set, it defaults to `schema/` relative to the **current working directory**, not the data directory. In Docker, CWD is typically `/`, so it would look for `/schema/`. This file is not present, so `migrate_with_sqldiff` would fail and fall back to `init_direct`. This means database migrations via `sqldiff` never run in production.

**Recommendation:** Make `GLOWBOT_SCHEMA_DIR` required in production, or bundle the schema SQL files into the binary.

---

## 4. MEDIUM: Code Smells

### 4.1 Enormous `dispatch_tool` Match Expression (S1)

**File:** `src/bot_dispatch.rs` — single match with ~25 arms  
**Severity:** MEDIUM — Maintainability

The `dispatch_tool` function is a single ~300-line match expression with 25+ arms. Every new tool requires adding another arm. This function:
- Cannot be tested independently (each arm has different dependencies)
- Has inconsistent error handling patterns
- Has inconsistent return types
- Violates the Open-Closed Principle

**Recommendation:** Use a `HashMap<String, Box<dyn Fn(...)>>` registry pattern where each tool registers itself. Or at minimum, move each tool handler to its own function file (several already exist — `bot_dispatch_memory.rs`, `bot_dispatch_image.rs`, etc.) and reference them from a much thinner dispatch function.

---

### 4.2 Mixed Sync/Async Mutex Usage (S2)

**Multiple files**  
**Severity:** MEDIUM — Deadlock Risk

The codebase uses **both** `std::sync::Mutex` and `tokio::sync::Mutex`:

| Mutex Type | Where Used | Purpose |
|---|---|---|
| `std::sync::Mutex` | `BotState` (inner), `git`, `HashMap` for chat_locks | Fast, non-async |
| `tokio::sync::Mutex` | `GlowBot.state` (outer), per-chat locks | Held across `.await` points |

The DB connection (`rusqlite::Connection`) is wrapped in `std::sync::Mutex`:
```rust
pub conn: Arc<Mutex<Connection>>,
```

Holding a `std::sync::Mutex` across an `.await` point will **panic at runtime** (or deadlock the Tokio worker thread). While the codebase doesn't currently do this (the DB lock is always acquired and released within sync functions), there's no enforcement mechanism. A future refactor could accidentally hold the DB lock across `.await`.

**Recommendation:** Use `tokio::sync::Mutex` consistently, or use `spawn_blocking` for DB operations. Document the constraint at every sync-mutex usage site.

---

### 4.3 `pub(crate)` / `pub` Confusion (S3)

**Multiple files**  
**Severity:** LOW — Maintenance Burden

There's inconsistent visibility in the codebase:
- `pub(crate) mod bot_dispatch_image` in `bot_dispatch.rs` (used from `bot.rs`)
- `pub mod bot_dispatch_config` and `pub mod bot_dispatch_model` in `bot_dispatch.rs` (referred from `main.rs`)
- Internal submodules like `db_embeddings` and `db_migrations` are `mod` (private within `db.rs`)

Some items that should be `pub(crate)` are `pub` (e.g., `BotState` fields), and some that should be `pub` are `pub(crate)`. This reflects the "growing module" pattern without a clear visibility strategy.

---

### 4.4 `println!()` in Server Code (S4)

**File:** `src/bot.rs`  
**Severity:** LOW

```rust
println!("Embedding backfill: {}/{} ({}%) done", done, total, pct);
```

This follows the same `log::info!` call on the next line. The `println!` is redundant in a server context where stdout is captured by Docker. Mixed stdout/stderr output can cause formatting issues in structured logging systems.

**Recommendation:** Remove `println!` calls. Use `log::info!` consistently.

---

### 4.5 `#[allow(dead_code)]` and Dead Functions (S5)

**Files:** `src/bot.rs`  
**Severity:** LOW

```rust
#[allow(dead_code)]
pub async fn new_with_llm(...)
#[allow(dead_code)]
pub async fn fetch_model_metadata(...)
```

`new_with_llm` is indeed used (called from `main.rs`), but `fetch_model_metadata` appears to be dead code — model metadata is fetched in `main.rs` directly, not via this method. The `#[allow(dead_code)]` annotation masks this.

**Recommendation:** Remove `fetch_model_metadata` if unreferenced, or remove the annotation if it's used implicitly.

---

### 4.6 Cloned Tool Definitions on Every LLM Call (S6)

**File:** `src/bot_pipeline.rs` (via `build_tools`)  
**Severity:** LOW — Performance

```rust
let tools: Vec<crate::openrouter::ToolDefinition> = if tools_enabled {
    let s = state.lock().await;
    let bash_enabled = s.config.is_bash_enabled(chat_id);
    s.build_tools(bash_enabled, chat_id)
} else {
    vec![]
};
```

Tool definitions are reconstructed for every LLM call (including every round in the tool loop). The definitions are static (they only change when the config or MCP tools change). A simple cache would reduce allocations and serialization overhead.

**Recommendation:** Cache `ToolDefinition` vectors and invalidate on config/skill/MCP changes.

---

### 4.7 `reqwest_011` Crate Alias (S7)

**File:** `Cargo.toml`  
**Severity:** LOW — Build Complexity

```toml
reqwest_011 = { package = "reqwest", version = "0.11", ... }
reqwest = { version = "0.12", ... }
```

Two different major versions of `reqwest` are pulled in. `reqwest_011` (0.11) is used exclusively for building the Telegram HTTP client with TCP keepalive. `reqwest` (0.12) is used everywhere else. This doubles the compile time and binary size for the HTTP stack.

In `main.rs`, the alias is `reqwest_011`:
```rust
let http_client = reqwest_011::Client::builder()
    .tcp_keepalive(...)
    ...
```

**Recommendation:** Upgrade teloxide to a version that supports `reqwest` 0.12, or build Telegram HTTP client directly with `reqwest` 0.12 (which also supports `tcp_keepalive`).

---

## 5. LOW: Oversights & Design Issues

### 5.1 No User Input Length Limits (O1)

**Files:** `src/bot.rs`, `src/bot_pipeline.rs`  
**Severity:** MEDIUM — Resource Exhaustion

User-submitted message text is not truncated or length-limited before being sent to the LLM. A user could send a message with 100,000+ characters, which would:
- Consume enormous token budgets in a single request
- Potentially overflow the context window with user input, squeezing out system prompt
- Take longer to process, blocking the per-chat mutex

**Recommendation:** Add a configurable max user message length (e.g. 4096 characters) with truncation and a note to the user.

---

### 5.2 No Rate Limiting on LLM API Calls (O2)

**Files:** `src/bot_pipeline.rs`, `src/bot_heartbeat.rs`  
**Severity:** MEDIUM — Cost Explosion

The heartbeat system can spawn multiple parallel LLM agents (one per chat) that all call the OpenRouter API simultaneously. Combined with the main message processing, there's no rate limiting or concurrency cap on LLM API calls. This could lead to:
- Rate limiting from OpenRouter
- Surprise costs (multiple chats firing heartbeats at the same time can send 10+ requests/minute)
- Tokio task starvation from many concurrent LLM calls

**Recommendation:** Add a semaphore (`tokio::sync::Semaphore`) limiting concurrent LLM calls to a configurable value (e.g. 3-5).

---

### 5.3 Heartbeat Loop Processes Dead Chat Directories (O3)

**File:** `src/main.rs`, `run_heartbeat_loop()`  
**Severity:** LOW — Resource Waste

The heartbeat scheduler scans `chats_dir` for all subdirectories and tries to process tasks/reminders for each. If a chat is removed from the config but its directory still exists on disk (orphaned), the scheduler will continue processing it indefinitely.

**Recommendation:** Skip chats that don't have a corresponding `chat_config` or `dm_config` entry. Log a warning about orphaned chat directories.

---

### 5.4 Config YAML With Secrets in Plaintext (O4)

**File:** `src/config.rs`, `config.example.yaml`  
**Severity:** LOW (by design, but worth noting)

The config file (`config.yaml`) stores the Telegram token and OpenRouter API key in **plaintext**. The `redacted()` method only redacts when displaying via `/config` command. If the file is leaked (e.g., via a bash tool that reads it, or via a git push of the data directory to a public repo), credentials are compromised.

**Recommendation:** Support environment variable interpolation in config values (e.g. `${OPENROUTER_API_KEY}`) so secrets can be injected at runtime rather than stored in the file.

---

### 5.5 MCP Tool Name Prefix Could Collide (O5)

**File:** `src/bot_dispatch.rs`, `src/openrouter_tools.rs`  
**Severity:** LOW

MCP tools are named `mcp_<server>_<tool>`. If a built-in tool is ever named starting with `mcp_`, it would be incorrectly classified. More practically, if two MCP servers have a tool with the same name (e.g., `mcp_server_a_get_weather` and `mcp_server_b_get_weather`), the naming works correctly via the server prefix — but the `filter` in tool listing and `format_tools_output` could be confused.

---

### 5.6 `load_messages` Performance on Large Histories (O6)

**File:** `src/db.rs`  
**Severity:** LOW

```rust
let mut msgs = Vec::with_capacity(raws.len());
for raw in raws {
    // serde_json::from_str for each message
    // serde_json::from_str for tool_calls
}
```

For each message, two `serde_json::from_str` calls are made (content + optional tool_calls). With 1000+ messages (possible with RAG search or large windows), this could be ~2000 deserializations per request. While SQLite is fast, the deserialization overhead adds up.

---

### 5.7 `serde_json::from_str` Uses Default on Parse Error (O7)

**File:** `src/bot_dispatch.rs`, line ~14  
**Severity:** LOW — Silent Mistake

```rust
let args: serde_json::Value =
    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
```

If the LLM returns invalid JSON in its tool call arguments (shouldn't happen with a well-formed model, but possible with some models), it silently falls back to an empty `Value::Null`. The tool handler then sees no arguments and may return confusing error messages or do nothing.

**Recommendation:** Log a warning when arguments fail to parse and return an error to the LLM so it retries with corrected arguments.

---

### 5.8 No Column Count Constraint on search_embeddings (O8)

**File:** `src/db_embeddings.rs`

```rust
let mut stored_norm_sq = 0.0f32;
for (i, &v) in stored_vec.iter().enumerate() {
    dot += v * query_embedding[i];  // <-- PANIC if stored_vec.len() > query_embedding.len()
    stored_norm_sq += v * v;
}
```

The function checks `stored_vec.len() != query_embedding.len()` and `continue`s if they differ (skip stale rows), but the check is after a fallible `unpack_embedding` call. If somehow the stored vector is shorter than the query vector, this would panic on OOB access. The else branch (query shorter than stored) would silently compute a partial dot product.

---

## 6. Dependency Issues

### 6.1 `mockall` Listed but Not Used

**File:** `Cargo.toml`

`mockall = "0.13"` is listed in `[dev-dependencies]` but the codebase uses a hand-rolled `MockLlmBackend` instead of `#[automock]`. The mockall crate is unused, adding compile time for no benefit.

---

### 6.2 `sqldiff` External Dependency

**File:** `src/db_migrations.rs`

The migration system depends on the `sqldiff` CLI tool (from `sqlite3` distribution). If it's not installed (common in minimal Docker images), migrations fall back to `init_direct`. This makes the `sqldiff` path untested in many environments.

---

### 6.3 Unused Dependencies

Likely unused: `futures`, `thiserror` (the codebase uses `anyhow` exclusively). Verify with `cargo udeps`.

---

## 7. Testing Coverage Gaps

### 7.1 `bot_dispatch.rs` — Large Untested Surface

The `dispatch_tool` function has ~25 arms, but many are not directly tested:
- `generate_image` — complex path with file I/O, base64, external API calls
- MCP tools (`mcp_*`) — depends on an external MCP server
- `edit_config` — involves Telegram API, diff generation, state management
- `propose_model_change` — involves Telegram API, state management
- `search_conversations` — depends on DB with embeddings
- `send_media` — depends on Telegram API
- `list_media` — file system I/O

Each of these tools should have handler-specific tests.

---

### 7.2 Heartbeat & Reminder Processing Untested

The entire heartbeat system (`run_heartbeat_task`, `run_heartbeat_loop`, `run_chat_heartbeat`, `process_reminder_action`) is **not unit-tested**. These are complex async functions with LLM calls, tool dispatch, and Telegram interaction.

---

### 7.3 `bot_models.rs` — Inline Keyboard Callback Tests Missing

The `handle_model_callback` function handles the full model browsing UI (4+ categories, pagination, detail views, specifier buttons). Each callback path diverges significantly but only `detail_cb` and `select_cb` helper functions are tested.

---

### 7.4 MCP Session Re-Init Not Tested

`reinitialize_mcp_session` and the session-retry logic in `invoke_tool` are not directly tested.

---

## Summary Table

| # | Severity | Category | Title | File(s) |
|---|----------|----------|-------|---------|
| C1 | CRITICAL | Security | `std::process::exit(0)` kills all in-flight work | `bot_dispatch_config.rs` |
| C2 | HIGH | Security | MCP tool invocation without timeout | `mcp_invoke.rs` |
| C3 | HIGH | Security | Credentials in debug logs; unbounded tool logging | `mcp_client.rs`, `bot_dispatch.rs` |
| C4 | MEDIUM | Security | TOCTOU race in MCP blacklist check | `bot_dispatch.rs` |
| D1 | HIGH | Data Loss | Git auto-commit commented out | `bot.rs` |
| D2 | HIGH | Data Loss | YAML parse errors silently swallowed | `reminders.rs`, `tasks.rs` |
| D3 | MEDIUM | Data Loss | Duplicate embeddings on concurrent backfill | `bot.rs` |
| B1 | MEDIUM | Bug | Character limit vs byte limit mismatch | `bot_send.rs` |
| B2 | LOW | Bug | `short_id()` collision within same nanosecond | `bot_dispatch_config/model.rs` |
| B3 | MEDIUM | Bug | `/stop` race with lock acquisition | `main.rs` |
| B4 | MEDIUM | Bug | Orphaned tool results with missing `tool_call_id` | `openrouter_context.rs` |
| B5 | MEDIUM | Bug | Schema dir fallback to CWD | `bot.rs` |
| S1 | MEDIUM | Code Smell | 25-arm dispatch function | `bot_dispatch.rs` |
| S2 | MEDIUM | Code Smell | Mixed sync/async mutex usage | Multiple |
| S3 | LOW | Code Smell | Inconsistent visibility | Multiple |
| S4 | LOW | Code Smell | `println!` in server code | `bot.rs` |
| S5 | LOW | Code Smell | Dead code annotation | `bot.rs` |
| S6 | LOW | Code Smell | Tool definitions cloned on every call | `bot_pipeline.rs` |
| S7 | LOW | Code Smell | Two reqwest major versions | `Cargo.toml` |
| O1 | MEDIUM | Oversight | No user input length limit | `bot_pipeline.rs` |
| O2 | MEDIUM | Oversight | No LLM rate limiting | `bot_pipeline.rs`, `bot_heartbeat.rs` |
| O3 | LOW | Oversight | Orphaned chat directories processed forever | `main.rs` |
| O4 | LOW | Oversight | Secrets in config file plaintext | `config.rs` |
| O5 | LOW | Oversight | MCP naming convention collision risk | `bot_dispatch.rs` |
| O6 | LOW | Oversight | Per-message JSON deserialization scaling | `db.rs` |
| O7 | LOW | Oversight | Tool arg parse failure silently defaults | `bot_dispatch.rs` |
| O8 | LOW | Oversight | Potential OOB access in similarity loop | `db_embeddings.rs` |
