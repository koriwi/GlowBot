# Feature: Clear Message History

## Status
Planned / Deferred

## Overview
Add a way for users to delete the persisted SQLite conversation history for a chat. This is a natural companion to the SQLite persistence feature — now that messages survive restarts, users need a way to wipe them.

## Motivation
- Privacy: users may want to erase their conversation trail
- Debugging: clearing history can help isolate issues
- Fresh context: starting a "new" conversation without old baggage

## Proposed Design

### User-facing command
```
/clear
```

Deletes all messages from the database for the **current chat**.

### Permissions
- Respect the existing `commands_enabled` setting — same as `/status` and `/tasks`
- Disabled by default = nobody can run it (secure by default)

### Implementation sketch

`src/commands.rs`:
- Add `"clear"` to the command parser
- `handle_clear_command(state, chat_id)` calls `state.db.clear_messages(chat_id)`
- Returns a confirmation like `Cleared 42 messages.` or `Nothing to clear.`

`src/bot.rs`:
- Wire the command through `handle_telegram_message()`
- Log the action to `tool_calls.log` for audit

`src/db.rs` (already exists):
- `db.clear_messages(chat_id)` already implemented

### Edge cases
- `clear_messages` is idempotent — running twice is fine
- Does NOT affect per-user `.md` memory files or chat-level `_chat.md` — only the transient conversation table
- Does NOT restart the bot or change config
- Should be undoable only via git history (since DB is not versioned separately)

### Open questions
- Should there be a global `/clearall` for admin-level wiping across all chats?
- Should clearing also reset `last_usage` context stats for that chat?
- Should we add a "Are you sure?" confirmation step?

## Files to touch
| File | Change |
|------|--------|
| `src/commands.rs` | Add `parse_command("clear")`, `handle_clear_command(...)` |
| `src/bot.rs` | Route `/clear` to the handler, update `setMyCommands` registration |
| `spec.md` | Add `/clear` to the commands table in §4.11 |

## Related work
- SQLite conversation persistence (completed)
- `Database::clear_messages()` method already exists and tested
