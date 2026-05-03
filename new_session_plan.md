# Session Plan: Heartbeat Fixes & Cleanup

## Context
Auditing Deepseek's heartbeat PR. Several bugs found in the per-chat timer loop system.

---

## Done (from previous session)

1. **MCP tools in heartbeat** — `BotState::build_tools()` shared between normal chat and heartbeat
2. **UUID task IDs** — replaced incrementing IDs with `uuid::Uuid::new_v4()`
3. **Agent messaging policy** — spec updated: silent by default, notifications allowed on errors + when explicitly requested in task description
4. **Per-chat scheduling** — each chat gets its own `tokio::spawn` timer loop with individual interval
5. **Heartbeat tests** — `test_heartbeat_interval_secs`, `test_heartbeat_disabled_when_zero`, `test_heartbeat_has_pending_tasks`, `test_build_tools_includes_mcp`

---

## TODO (this session)

### 1. Fix scheduler to skip disabled chats
**Bug:** `heartbeat_interval_secs()` returned `Option<u64>` but the `run_heartbeat_loop` scheduler was written when it returned `u64`. After the type change, the scheduler still pushes disabled chats into the loop because it maps Option<u64> → (String, u64) silently by collecting. Actually, looking at the code more carefully, the issue is that `heartbeat_interval_secs` used `unwrap_or(global_default)` — so a disabled chat (interval=0) got the global default (90min).

**Fix applied:** Changed `heartbeat_interval_secs` to return `Option<u64>` (no fallback).
**Still needed:** Update `run_heartbeat_loop` to only spawn for `Some(interval_secs)`.

### 2. Fix per-chat loops to exit when no tasks remain
**User request:** "spawned loops with no tasks left should exit"

**Current behavior:** `run_chat_heartbeat` runs once, then sleeps the full interval regardless of whether there are tasks.

**Expected behavior:** After `run_heartbeat_task` completes, if no tasks remain, the loop should exit immediately (unregistering from `active` HashSet) so the chat becomes eligible for re-discovery when new tasks are added.

**Implementation:** `run_chat_heartbeat` should check `has_pending_tasks()` after running the task processor. If false, `break`.

### 3. Write `todo.md` for live config behavior
**Item:** Document that `/commands` mutate config live, but manual `config.yaml` edits require restart. No file watcher exists.

**Skip for now** — user explicitly said "write that into a todo file and skip it for now."

---

## Files to touch

| File | Changes |
|------|---------|
| `src/main.rs` | Update scheduler for `Option<u64>`, use `heartbeat_scan_interval_seconds` config field, make loops exit on empty task list |
| `src/bot.rs` | `heartbeat_interval_secs` already changed to `Option<u64>`, test may need update |
| `src/config.rs` | `heartbeat_scan_interval_seconds` already added |
| `todo.md` | NEW — write live config behavior docs, defer implementation |
| `spec.md` | May need update for loop-exit behavior |

---

## Commits planned

1. `fix: heartbeat scheduler skips disabled chats, uses configurable scan interval`
2. `fix: per-chat heartbeat loops exit when no tasks remain`
3. `docs: write todo.md for live config reload behavior`
