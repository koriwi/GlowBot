# GlowBot — Security & Quality Audit Report

**Date:** 2026-05-12  
**Scope:** Full source code audit (`src/`)  
**Classification:** Vulnerabilities, footguns, code smells, bugs, and oversights  

> **Severity levels:** 🔴 CRITICAL | 🟠 HIGH | 🟡 MEDIUM | 🔵 LOW | ⚪ CODE SMELL

---

## 🔴 CRITICAL

### C1. `tool_read_config` Leaks API Keys to the LLM

- **File:** `src/bot_dispatch_config.rs` — lines 32–36
- **Function:** `tool_read_config()`

```rust
pub(crate) async fn tool_read_config(state: &Arc<Mutex<BotState>>) -> String {
    let s = state.lock().await;
    match serde_yaml::to_string(&s.config) {
        Ok(yaml) => yaml,  // ← returns UNREDACTED YAML
        Err(e) => format!("Error serializing config: {}", e),
    }
}
```

The `read_config` tool definition in `openrouter_tools.rs` (line ~296) lets the LLM read the full config. The function returns `s.config` directly — **without calling `.redacted()`**. This means:

- **`telegram_token`** — leaked to the LLM, sent back to OpenRouter in subsequent requests
- **`openrouter.api_key`** — same leak vector
- **All MCP server `api_key` values** — also leaked

The `config_yaml` field on `edit_config` is a **write** path — the LLM is supposed to provide the full config — but that's only dangerous if a user's prompt injection tricks the LLM into rewriting secrets they shouldn't have access to. The `read` path is the primary leak: the LLM now **has** the keys and they travel through every API request.

**Impact:** Any prompt injection attack (or even a normal conversation) can exfiltrate the API keys and Telegram token. The `redacted()` method exists and is used by `/config` for the user, but not for the LLM.

**Fix:** `s.config.redacted()` instead of `s.config` in `tool_read_config`.

---

### C2. `std::process::exit(0)` Brutally Kills the Process

- **File:** `src/bot_dispatch_config.rs` — line 173
- **Function:** `handle_config_callback()` → accept branch

```rust
tokio::spawn(async {
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    std::process::exit(0);  // ← kills everything in-flight
});
```

When a user accepts a config change, the bot calls `exit(0)` to trigger Docker restart. This is **not a graceful shutdown**:

- **All in-flight LLM calls** are aborted mid-response — OpenRouter may still bill
- **Active SQLite transactions** are interrupted — risk of database corruption
- **Active embedding backfill** is lost mid-way
- **Active tool calls** (bash, MCP) are killed — bash commands running via `tokio::process::Command` are orphaned
- **No cleanup** of pending config changes, stop signals, or other state
- **Messages being processed** across all chats are dropped

`exit(0)` does not run `Drop` implementations. The `MutexGuard` on `state` was dropped (the scope ended), but tasks spawned with `tokio::spawn` are not awaited.

A Docker health-check restart is cleaner than this — but if the goal is to pick up config changes, the bot could reload config while running (already supported via `/reload` pattern) or at minimum use `tokio::signal` to request a graceful shutdown.

**Fix:** Use a proper shutdown mechanism (signal → drain → exit), or hot-reload config without restarting (the config struct is already behind `Arc<Mutex<>>` so the state just needs swapping).

---

### C3. `std::sync::Mutex` on SQLite Connection in Async Context

- **File:** `src/db.rs` — line 32
- **Type:** `pub(crate) conn: Arc<Mutex<Connection>>`

The SQLite `rusqlite::Connection` is wrapped in `std::sync::Mutex` (not `tokio::sync::Mutex`), and used inside async functions with `.lock().unwrap()`. This blocks the **entire tokio worker thread** while the lock is held.

- Every `Database::save_messages()` and `Database::load_messages()` call blocks the reactor
- If the lock is contended, the worker thread is wasted doing nothing
- Under load, this causes head-of-line blocking: a slow DB query delays message handling for **all chats**

In a Docker container typically limited to 1–2 CPU cores, the tokio runtime has few worker threads. Blocking them is a performance disaster waiting to happen.

**Affected functions (non-exhaustive):**
- `load_messages` (db.rs:52)
- `save_messages` (db.rs:135)
- `clear_messages` (db.rs:172)
- `set_cutoff` / `get_cutoff`
- `save_embedding` / `search_embeddings` (db_embeddings.rs)
- `find_unembedded_messages`
- `cleanup_mismatched_embeddings`

**Fix:** Use `tokio::sync::Mutex` for brief-held async mutexes, or better: use `tokio::task::spawn_blocking()` for all SQLite operations.

---

### C4. Bash Tool Has No Allowlist or Sandboxing

- **File:** `src/bash.rs` — all functions
- **File:** `src/bot_dispatch.rs` — lines 70–86 (bash dispatch)

The `bash` tool executes arbitrary shell commands via `bash -c`. The only protection is the system-prompt-level instruction: *"Never run destructive commands (rm -rf, format, etc.) unless explicitly asked."*

**This is a text-level suggestion, not a security boundary.** Against:

- **Prompt injection:** A malicious user message embedded in a forwarded document, a URL in a caption, or adversarial text anywhere in the conversation can instruct the LLM to call `bash("rm -rf /")`. The LLM is notorious for following such instructions.
- **Indirect injection:** MCP server output, memory file contents, or search results fed back into the LLM can contain injection payloads.
- **Tool hallucination:** LLMs sometimes call tools with parameters the user never intended.

The AGENTS.md says *"safe by container isolation"* — but:
- Docker volumes bind-mount the data directory, which contains the entire conversation history, memory files, config (including MCP API keys), and git history
- Many MCP servers run network-accessible services (databases, file systems)
- If the container has access to Docker socket (common in CI/dev setups), escape is possible

**Fix:** Implement a command allowlist, block known-dangerous patterns (`rm -rf`, `> /dev/`, `dd`, `mkfs`, `:(){ :|:& };:`), or integrate a tool-use policy validator.

---

## 🟠 HIGH

### H1. Heartbeat Tasks Ignore Stop Signals

- **File:** `src/bot_heartbeat.rs` — both `run_heartbeat_task()` and `process_reminder_action()`

The heartbeat loop (`run_heartbeat_task`) and the reminder action processor (`process_reminder_action`) run up to **10 LLM rounds** each without checking for stop signals. The `/stop` command only sets a stop signal in `main.rs`/`handle_message`, but:

- Background tasks spawned by `run_heartbeat_loop` in `main.rs` don't receive the `stop_signals` reference
- `run_heartbeat_task` takes only `state` and `git_repo` (line 11), not `stop_signals`
- `process_reminder_action` similarly has no stop mechanism

**Impact:** If a heartbeat task enters an expensive LLM loop (e.g. a bash command that hangs, an MCP call that retries, an LLM that cycles tool calls), the `/stop` command is powerless. The only way out is `std::process::exit(0)` from a config change — which is C2.

**Fix:** Thread the stop signal through to heartbeat tasks and check it at every tool-round boundary, same as the main pipeline.

---

### H2. Image Filename Collision Risk

- **File:** `src/bot_dispatch_image.rs` — line 109
- **Format:** `generated/{model}_{timestamp}_{index}.{ext}`

Generated images use `chrono::Utc::now().format("%Y%m%dT%H%M%S")` which is **second-level granularity**. Two concurrent image generations in the same second (or even a single generation that produces multiple images) will collide.

This is a real risk: OpenRouter image generation can return fast (<1s), and multiple users could trigger generation simultaneously. The `index` suffix helps within a single response, but across responses the second boundary is the same.

**Fix:** Include milliseconds or microseconds in the timestamp, or prepend a UUID.

---

### H3. YAML Deserialization Silently Ignores Errors with `unwrap_or_default()`

Three occurrences of silent data loss:

1. **`src/reminders.rs` line 23** — `ReminderList::load`:
   ```rust
   let list: Self = serde_yaml::from_str(&data).unwrap_or_default();
   ```
2. **`src/tasks.rs` line 17** — `TaskList::load`:
   ```rust
   let list: Self = serde_yaml::from_str(&data).unwrap_or_default();
   ```
3. **`src/db_migrations.rs` line 95** — ALTER TABLE failure:
   ```rust
   let _ = conn.execute("ALTER TABLE ...", []);
   ```

If a `reminders.yaml` or `tasks.yaml` becomes corrupt (disk error, manual edit, bot crash mid-write), the entire list silently resets to empty — **all pending reminders or tasks are lost**, without any warning log.

The ALTER TABLE migration silently fails if the column already exists (which is the expected case), but also if there's a real error.

**Fix:** Log a warning when deserialization fails. Use `anyhow::Context` to surface the error rather than silently swallowing it.

---

### H4. `unwrap()` on `std::sync::Mutex` Locks Everywhere — Poison = Cascade Failure

Across the codebase, the pattern is:

```rust
let conn = self.conn.lock().unwrap();           // db.rs
let chats_dir = s.chats_dir();                   // after state.lock().await
let mut s = state.lock().await;                  // fine (tokio Mutex)
let mut locks = chat_locks.lock().unwrap();      // main.rs
```

Any `std::sync::Mutex` that has been **poisoned** (a panic occurred while the guard was held) will immediately panic again on the next `.lock().unwrap()`. Since the SQLite connection is behind a `std::sync::Mutex`:

- A panic in any DB operation (e.g. a malformed SQL query, an OOM during serialization, or a bug in `rusqlite`) poisons the lock
- **Every subsequent DB operation** panics — messages can't be saved, loaded, embedded, or searched
- The bot enters a crash loop of panics

Similarly, `chat_locks` in `main.rs` uses `std::sync::Mutex` and panics on poison.

**Fix:** Use `.lock().map_err(...)` or `.lock().expect("...")` with a clear message. Better: use `tokio::sync::Mutex` consistently, which doesn't poison. For the DB, use `std::sync::PoisonError` recovery or a connection pool.

---

### H5. MCP Tool Results Not Size-Limited

- **File:** `src/mcp_invoke.rs` — the `invoke_tool_once()` and `invoke_tool()` functions return full raw JSON-RPC results as strings
- **File:** `src/bot_dispatch.rs` — the `mcp_` dispatch at lines 146–195

MCP tools like Playwright's `screenshot` or browser automation can return **megabytes** of HTML, screenshot base64 data, or file contents. These get stuffed into `tool_result` messages:

1. Bloating the LLM context window (costing money, potentially hitting context limits)
2. Triggering OOM if the response is large enough
3. Taking up Tokio working thread time during serialization/deserialization

The number of characters logged to `tool_calls.log` is truncated to 300, but the **result returned to the LLM is not truncated** anywhere.

**Fix:** In `dispatch_tool_calls` or `invoke_tool`, truncate MCP tool results to a reasonable limit (e.g. 4000 chars) with a `... (truncated)` suffix.

---

### H6. No Rate Limiting — API Cost Amplification

There is **no rate limiting** at any layer:

- **Per-user:** A user can paste a 10,000-word document into a group chat, triggering a single LLM call — fair. But they could also send 100 rapid messages, each triggering a separate LLM call queue.
- **Per-chat:** The per-chat lock prevents parallel processing **within** a chat, but a single user in 10 chats triggers 10 concurrent LLM calls.
- **Tool loop:** Up to 64 rounds (pipeline.rs) or 10 rounds (heartbeat) per message — each round is an API call
- **Embedding backfill:** Calls OpenRouter embedding API synchronously for **every unembedded message**, with only 500ms delay between calls
- **Heartbeat:** Scans all chats every 60s, re-parsing YAML files each time

**Impact:** Prompt injection or buggy tool loops can rapidly deplete API budget. The max 64 tool rounds × repeated LLM calls per round could consume thousands of tokens in seconds.

**Fix:** Add per-user, per-chat, and global rate limits; cap LLM spending; limit concurrent embedding calls.

---

## 🟡 MEDIUM

### M1. Duplicated `short_id()` with Timestamp Collisions

Two identical functions in:
- `src/bot_dispatch_config.rs` lines 11–16
- `src/bot_dispatch_model.rs` lines 11–16

```rust
fn short_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();                    // nanosecond-precision
    format!("{:x}", ts)
}
```

While nanosecond precision sounds safe, this has issues:
- **Not monotonic:** `SystemTime::now()` can go backward (NTP adj, leap seconds, daylight saving)
- **Collision under concurrency:** Two `propose_model_change` and `edit_config` calls on the same nanosecond produce the same ID — one overwrites the other's `pending_*_changes` entry
- **64-byte callback data limit overlap:** The hex string from `as_nanos()` can be up to 15 hex chars (60 bits), which combined with `cfg:` or `mdl:` prefix plus action suffix stays under 64 bytes — but it's fragile

**Fix:** Extract to a shared utility function and include a random component (e.g. `format!("{:x}{:04x}", ts, rand::random::<u16>())`).

---

### M2. Invalid `chat_id.parse().unwrap_or_default()` Becomes `ChatId(0)`

- **File:** `src/main.rs` — line 93:
  ```rust
  let chat = ChatId(chat_id.parse().unwrap_or_default());
  ```
- Also: `src/bot_dispatch.rs:73`, `src/bot_dispatch_config.rs:88`, `src/bot_dispatch_model.rs:94`

When `chat_id` fails to parse as `i64`, it becomes `ChatId(0)`. This is a non-existent chat. Messages sent to `ChatId(0)` silently disappear into Telegram's void with no error reported to the caller. The bot has no way to know something went wrong.

**Impact:** A malformed chat_id in config, a database value, or a Telegram callback causes messages to be silently dropped with no diagnostics.

**Fix:** Parse once and propagate errors explicitly, or at minimum log a critical error when the parse fails.

---

### M3. `build_trimmed_request` Sends Over-Limit Requests When Fixed Cost Exceeds Limit

- **File:** `src/openrouter_context.rs` — lines 100–106

```rust
if fixed_cost >= effective_limit {
    // Still try to send head + turn only, history is impossible
    let mut msgs = head.to_vec();
    msgs.extend(turn.iter().cloned());
    return (msgs, true);
}
```

When the system prompt + tools + turn + response reserve exceed the context limit, the function still sends the request. This will either:
1. Be rejected by the API as over context limit (wasted API call)
2. Be truncated by OpenRouter silently (losing tool definitions or the user message)
3. Cost money for a guaranteed failure

This is particularly dangerous with small-context models. The 8K token response reserve is hardcoded to `8192`, which is larger than some models' entire context (e.g., some older models have 4K context).

**Fix:** Return an error instead of sending a known-over-limit request. Make `RESPONSE_RESERVE_TOKENS` model-aware.

---

### M4. Config Save Does Not Git Auto-Commit

- **File:** `src/bot.rs` — `save_config()` method, lines 60–69

```rust
pub async fn save_config(&self) -> anyhow::Result<()> {
    let state = self.state.lock().await;
    let path = state.config_path();
    state.config.save(&path)?;
    drop(state);
    // self.git_repo
    //     .auto_commit("Update configuration via /command")?;  // ← COMMENTED OUT
    Ok(())
}
```

The git auto-commit is commented out, which means:
1. The config can be out of sync with git state
2. The audit trail for config changes is lost
3. Manual intervention is needed to commit config changes

By contrast, `start_embedding_backfill`'s `embed_turn` function (in `bot_pipeline.rs`) does try to call `git_repo.auto_commit`, but it lacks access to the git repo (it only has `db`, `api_key`, etc.).

**Fix:** Re-enable auto-commit, or remove the `_git_repo` parameter and commented code if the feature is intentionally disabled.

---

### M5. No Message Retention / Cleanup — Unbounded SQLite Growth

- **File:** `src/db.rs` — there is no cleanup mechanism

Every message in every chat is stored forever in the `messages` table. For active chats:
- A busy group can produce thousands of messages per day
- Each message stores full JSON content, optional tool calls JSON, and optional reasoning text
- Embeddings table grows proportionally
- No `DELETE` or `VACUUM` strategy exists

The `load_messages` query uses `ORDER BY id DESC LIMIT ?`, which does benefit from the index — but the table itself continues to grow, eventually impacting:
- Backup time and size
- Embedding search speed (`search_embeddings` scans all embeddings for the chat model)
- Storage on constrained Docker volumes

**Fix:** Add configurable retention policy (e.g., keep last N days or N messages per chat). Implement periodic VACUUM or offline compaction.

---

### M6. `due()` Reparses All Reminder Timestamps Every Call

- **File:** `src/reminders.rs` — lines 86–93

```rust
pub fn due(&self) -> Vec<&Reminder> {
    let now = chrono::Utc::now();
    self.reminders.iter().filter(|r| {
        chrono::DateTime::parse_from_rfc3339(&r.trigger_at)  // ← parsed every check
            .map(|dt| dt < now)
            .unwrap_or(false)
    }).collect()
}
```

This function is called every 60 seconds per chat (from `run_heartbeat_loop` in `main.rs`). Each call re-parses every single reminder's `trigger_at` string as an RFC 3339 datetime. For an active chat with 50 reminders, that's 50 string parses per scan cycle.

**Fix:** Parse `trigger_at` at load time and store as a cached `DateTime` field alongside the string.

---

### M7. YAML Files Re-parsed Every Heartbeat Cycle (Every 60 Seconds)

- **File:** `src/bot_dispatch.rs` — `add_task`, `list_tasks`, `remove_task`, `list_reminders`, `remove_reminder`, `create_reminder` — all call `TaskList::load()` or `ReminderList::load()` which reads and deserializes the YAML file from disk
- **File:** `src/bot_heartbeat.rs` — calls `TaskList::load` and `ReminderList::load` inside the loop
- **File:** `src/bot_state.rs` — `has_pending_tasks()` and `has_due_reminders()` also call `load`
- **File:** `src/main.rs` — `run_heartbeat_loop` calls `has_pending_tasks` and `has_due_reminders` every 60 seconds scanning **every chat dir**

For N chats, each heartbeat cycle:
1. Reads dir listing for chats
2. For each chat with tasks: reads YAML → deserializes → checks `has_tasks()` (3 × reads for `has_pending_tasks` + `has_due_reminders`)
3. Inside heartbeat: reads YAML again → deserializes again

**Fix:** Add an in-memory cache with file-mtime checking, or at minimum batch the reads together.

---

## 🔵 LOW

### L1. Logging User Message Content

- **File:** `src/bot_pipeline.rs` — line 27

```rust
log::info!(
    "pipeline: starting LLM processing for chat={}, user={}, text=\"{}\", has_media={}",
    chat_id, user_id,
    text.chars().take(100).collect::<String>(),   // ← logs first 100 chars of user text
    media.is_some()
);
```

- **File:** `src/main.rs` — line 90

```rust
log::info!(
    "Message from {} ({}) in chat {}: {}",
    username, user_id, chat_id,
    text.as_deref().unwrap_or("(media)")           // ← logs full user message
);
```

`main.rs` logs the **full user message text** with no truncation. This is a privacy concern:
- In a shared log environment (Docker, journald, log aggregation), all user conversations are visible in plain text
- If `RUST_LOG=debug`, many more internal details are logged
- Some users may share sensitive information (passwords, tokens in conversations) — this ends up in logs

`bot_pipeline.rs` at least truncates to 100 chars.

**Fix:** Truncate user text in all log statements. Consider a short ID approach. Also ensure `log::info!` in `bot.rs` is truncated (which it's not — `text.as_deref().unwrap_or("(media)")` passes the full message).

---

### L2. MCP Session Race Condition on Concurrent Re-init

- **File:** `src/mcp_invoke.rs` — `invoke_tool()` function, lines 97–118
- **File:** `src/bot_dispatch.rs` — MCP dispatch, lines 146–195

When a session expires (HTTP 500 "Session not found"):
1. `invoke_tool` calls `reinitialize_mcp_session()` which creates a new `McpClient` and gets a new session ID
2. The session ID is propagated to **all tools from the same server** via `t.session_id = tc.session_id.clone()`
3. But another concurrent tool call may be doing the same thing, and the writes to `session_id` are not synchronized

The propagation loop in `bot_dispatch.rs`:
```rust
if tc.session_id.is_some() {
    for t in &mut s.mcp_tools {
        if t.server_name == server {
            t.session_id = tc.session_id.clone();
        }
    }
}
```

This mutates `s.mcp_tools` while holding the state lock—but other concurrent paths may hold a reference to the old `t` through `s.mcp_tools` clones. If two tool calls expire simultaneously, they can race:
- Call A re-inits, gets session X, starts propagating
- Call B re-inits, gets session Y, overwrites session X
- Call A's subsequent retry uses session Y (wrong session? or maybe fine)
- Worse: Call A's retry and the propagation happen in interleaved order

**Fix:** Use a per-server `RwLock<Option<String>>` for the session ID rather than mutating all tool structs.

---

### L3. Media Directory Not Validated at Startup

The `media_dir` config value (defaulting to `/media`) is used extensively:
- As a download target for `curl` in the system prompt
- As ingest directory for Telegram files
- As the directory for generated images
- As the base for `list_media` and `send_media`

But it's **never checked** at startup:
- Does the directory exist?
- Is it writable?
- Is it a symlink to an unexpected location?
- Does `media_dir/ingest` and `media_dir/pw-media` exist?

If `/media` doesn't exist:
- File downloads silently fail
- Generated images silently fail to save
- `list_media` returns empty
- MCP Playwright screenshots fail

**Fix:** Validate `media_dir` at startup, create subdirectories (`ingest`, `generated`, `pw-media`), and abort with a clear error if the path is unusable.

---

### L4. Long-Polling Instead of Webhooks

- **File:** `src/main.rs` — the polling loop is a manual `get_updates` implementation

The bot uses long-polling (`get_updates`) instead of Telegram webhooks. This has several operational downsides:
- **Connection churn:** Opens a new TCP connection every 30 seconds (even with keepalive)
- **No push delivery:** Messages are delivered with up to 30s latency
- **Error surface:** The restart loop adds complexity (lines 122–153)
- **No multi-instance support:** Can't run multiple replicas behind a load balancer
- **Callback data handling:** CallbackQueries arrive in the same polling loop, so a slow message handler can delay callback responses

The 30-second timeout, TCP keepalive, and restart loop are all workarounds for limitations inherent to polling. Webhooks are the recommended approach for production Telegram bots.

---

## ⚪ CODE SMELLS

### S1. Massive `dispatch_tool` Match — 200+ Lines

- **File:** `src/bot_dispatch.rs` — `dispatch_tool()` function spans ~200 lines with ~30 arms

The `match tool_name` block has grown organically. It's already been partially split into submodules (`bot_dispatch_config`, `bot_dispatch_image`, `bot_dispatch_media`, `bot_dispatch_memory`, `bot_dispatch_skills`, `bot_dispatch_model`), but bash and many MCP-related tools remain inline.

Each new tool definition in `openrouter_tools.rs` needs a corresponding handler here. The signal-to-noise ratio makes it hard to audit security (C1, C4, H5 all manifest in this function).

**Suggestion:** Continue the submodule pattern. Each tool family (bash, tasks, reminders, search, etc.) gets its own file with a `dispatch_<tool>(...)` function.

---

### S2. `allow(dead_code)` on Core Initialization Method

- **File:** `src/bot.rs` — line 33

```rust
#[allow(dead_code)]
pub async fn new_with_llm(data_dir: &Path, llm: Arc<dyn LlmBackend>) -> anyhow::Result<Self> {
```

`new_with_llm` is the primary constructor used by `main.rs`. The `#[allow(dead_code)]` attribute suggests the compiler doesn't detect it as used — which is odd since `main.rs` calls it. This might be because `GlowBot` is behind `Arc<Mutex<>>` and the tests don't call it directly.

Regardless, suppressing this lint on the **primary constructor** suggests that the lint was added to silence something else. If the function is genuinely unused (tests bypass it entirely), this is a maintenance smell.

---

### S3. Dual `reqwest` Dependencies (0.11 and 0.12)

- **File:** `Cargo.toml` — lines 7–10

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
# reqwest 0.11 for configuring teloxide's HTTP client with TCP keepalive
reqwest_011 = { package = "reqwest", version = "0.11", default-features = false, features = ["rustls-tls"] }
```

Two major versions of `reqwest` are compiled into the binary, doubling TLS-related compilation time and binary size. The 0.11 dependency exists solely to configure TCP keepalive on the `teloxide` bot client.

- `teloxide 0.13` may have updated its reqwest dependency — check if 0.11 is still needed
- Even if needed, this adds ~15 seconds to build time and ~3MB to binary size for a single feature

---

### S4. `db_embeddings.rs` Does Linear Scan for Similarity Search

- **File:** `src/db_embeddings.rs` — `search_embeddings()`, lines 95–150

The function loads up to `search_limit` (default 1000) embeddings into memory, then computes cosine similarity against each one in a linear loop with **triple pointer chasing**: `stored_vec[i] * query_embedding[i]`.

This is:
- **O(n × d)** where `d` is the embedding dimension (typically 768–3072)
- The entire loop runs while holding the DB mutex (C3)
- All rows are loaded as blobs, deserialized, compared, then sorted
- No SIMD, no approximation, no index

For 1000 embeddings at 1536 dimensions, this is ~1.5M float operations per query — fast on CPU, but all under the std mutex in an async context. And as the database grows, this search limit is the only thing preventing O(n×d×messages) scans.

---

### S5. Embedding Backfill Uses `println!` to stderr/stdout

- **File:** `src/bot.rs` — lines 117, 123

```rust
println!("Embedding backfill: {}/{} ({}%) done", done, total, pct);
```

Inside a background Tokio task, `println!` is used alongside `log::info!`. `println!` goes to stdout without the structured logging framework's formatting, timestamp, or log level. In Docker, this mixes `println!` output with log output in unpredictable ways (depending on whether the app logs to stdout or stderr).

**Fix:** Remove `println!` calls; use only `log::info!`.

---

### S6. `base64_encode` Duplicated in Two Files

- **File:** `src/media.rs` — lines 108–111
- **File:** `src/bot_dispatch_image.rs` — lines 213–216

Identical `base64_encode` function in two places. Both use `base64::engine::general_purpose::STANDARD.encode(data)`.

**Fix:** Move to a shared utility module.

---

### S7. `detail_cb()` / `select_cb()` Truncation Trimming Prefix Strings

- **File:** `src/bot_models.rs` — lines 18–35

```rust
fn detail_cb(model_id: &str) -> String {
    let full = format!("model:detail:{}", model_id);
    if full.len() <= MAX_CALLBACK_BYTES {
        full
    } else {
        format!("d:{}", model_id)  // ← just shorten the prefix
    }
}
```

When the callback would exceed 64 bytes, the prefix is shortened from `model:detail:` (13 chars) to `d:` (2 chars). This saves ~11 bytes but:
- Model IDs like `openai/gpt-4o-2024-11-20:nitro:floor:free` are well under 64 bytes
- But a provider name like `very-long-provider-name-that-exceeds-telegram-limit` would still hit 64 bytes
- The short-prefix callback handling in `handle_model_callback` has a split logic mismatch (`splitn(3)` vs `splitn(2)` for `d:` vs `model:detail:`) — prone to bugs

The existing tests cover this, which is good, but the split logic is fragile.

---

### S8. Permissions: "Linked" Telegram Commands Accepted Without Mention

- **File:** `src/main.rs` — `handle_message()` does not check `msg.text().unwrap_or("").contains("@botname")` before routing to `process_message_impl`
- **File:** `src/bot.rs` — `process_message_impl()` at line 119 checks if `is_command` but `parse_command` on line 109 is checked **before** the `is_mention` gate

In detail: `bot.rs:109` runs `parse_command(text)` regardless of mention status. If the text is `/model gpt-4` without `@bot_username`, `handle_bot_command_impl` is called directly (line 112). The `is_mention` check at line 132 only applies to **non-command** messages.

This means: any message starting with `/` that matches a known command triggers the command handler **even without being mentioned**, in **any chat the bot is in** — including large groups where the bot otherwise waits for `@mention`.

However, `can_run_command` and `can_interact` gate on whitelists. In practice, if the command whitelist is empty (nobody), the command is rejected before execution. The real risk: `/model` default replies with inline buttons (with or without `tg_bot`), and `/prompt` dumps the system prompt.

---

## Summary Table

| # | Severity | Issue | File(s) | Fix Complexity |
|---|----------|-------|---------|----------------|
| C1 | 🔴 CRITICAL | `tool_read_config` leaks API keys to LLM | `bot_dispatch_config.rs:32-36` | 1 line |
| C2 | 🔴 CRITICAL | `exit(0)` kills in-flight work, corrupts DB | `bot_dispatch_config.rs:173` | Replace with graceful shutdown |
| C3 | 🔴 CRITICAL | `std::sync::Mutex` on DB in async context | `db.rs:32` | Use `spawn_blocking` |
| C4 | 🔴 CRITICAL | Bash tool has no allowlist/sandboxing | `bash.rs`, `bot_dispatch.rs:70-86` | Add command filter |
| H1 | 🟠 HIGH | Heartbeat ignores stop signals | `bot_heartbeat.rs` | Thread signal to heartbeat |
| H2 | 🟠 HIGH | Image filename collision (second-level timestamp) | `bot_dispatch_image.rs:109` | Add UUID/ms |
| H3 | 🟠 HIGH | YAML parse failures silently reset data | `reminders.rs:23`, `tasks.rs:17` | Log + preserve data |
| H4 | 🟠 HIGH | Mutex poisoning cascade failure | Multiple files | Handle poison errors |
| H5 | 🟠 HIGH | MCP tool results untruncated | `mcp_invoke.rs` | Truncate results |
| H6 | 🟠 HIGH | No rate limiting — API cost amplifier | All | Add rate limits |
| M1 | 🟡 MEDIUM | Duplicated `short_id()`, nanosecond collision risk | `bot_dispatch_config.rs:11-16`, `bot_dispatch_model.rs:11-16` | Extract + add random |
| M2 | 🟡 MEDIUM | Invalid chat_id becomes ChatId(0), silent drop | `main.rs:93` and others | Parse and propagate error |
| M3 | 🟡 MEDIUM | Over-limit requests sent when fixed_cost exceeds limit | `openrouter_context.rs:100-106` | Return error, don't send |
| M4 | 🟡 MEDIUM | Config save git auto-commit commented out | `bot.rs:65-67` | Re-enable or remove |
| M5 | 🟡 MEDIUM | No message retention policy — unbounded growth | `db.rs` | Add retention + cleanup |
| M6 | 🟡 MEDIUM | Reminder timestamps re-parsed every check | `reminders.rs:86-93` | Cache parsed time |
| M7 | 🟡 MEDIUM | YAML files re-read every heartbeat cycle | Multiple files | Add in-memory cache |
| L1 | 🔵 LOW | User message content logged in plain text | `main.rs:90`, `bot_pipeline.rs:27` | Truncate all log lines |
| L2 | 🔵 LOW | MCP session ID race on concurrent re-init | `mcp_invoke.rs`, `bot_dispatch.rs` | Use per-server RwLock |
| L3 | 🔵 LOW | Media directory not validated at startup | — | Validate at boot |
| L4 | 🔵 LOW | Long-polling instead of webhooks | `main.rs` | Switch to webhook |
| S1 | ⚪ SMELL | 200+ line match in `dispatch_tool` | `bot_dispatch.rs` | Continue submodule split |
| S2 | ⚪ SMELL | `#[allow(dead_code)]` on primary constructor | `bot.rs:33` | Remove lint override |
| S3 | ⚪ SMELL | Dual reqwest 0.11/0.12 dependency | `Cargo.toml:7-10` | Unify versions |
| S4 | ⚪ SMELL | Linear scan for embedding similarity search | `db_embeddings.rs` | Consider ANN index |
| S5 | ⚪ SMELL | `println!` mixed with structured logging | `bot.rs:117,123` | Use `log::info!` only |
| S6 | ⚪ SMELL | Duplicated `base64_encode` function | `media.rs`, `bot_dispatch_image.rs` | Shared utility |
| S7 | ⚪ SMELL | Fragile callback split logic for long model IDs | `bot_models.rs:18-35` | Safer encoding |
| S8 | ⚪ SMELL | Commands accepted without `@mention` in some paths | `bot.rs:109-114` | Add mention check |
