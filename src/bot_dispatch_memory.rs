use super::BotState;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle the `read_memory` tool — load a user's memory file.
pub(crate) async fn tool_read_memory(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    args: &serde_json::Value,
) -> String {
    let uid = args["user_id"].as_str().unwrap_or("");
    let s = state.lock().await;
    match crate::memory::load_memory(&s.chats_dir(), chat_id, uid) {
        Some(m) => serde_json::json!({
            "user_id": m.frontmatter.user_id,
            "username": m.frontmatter.username,
            "call_name": m.frontmatter.call_name,
            "description": m.frontmatter.description,
            "body": m.body,
        })
        .to_string(),
        None => format!("No memory file found for user_id={} in chat {}", uid, chat_id),
    }
}

/// Handle the `update_memory` tool — create or update a user's memory file.
pub(crate) async fn tool_update_memory(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    args: &serde_json::Value,
) -> String {
    let uid = args["user_id"].as_str().unwrap_or("");
    if uid.is_empty() {
        return "Error: user_id is required".into();
    }
    let s = state.lock().await;
    let chats_dir = s.chats_dir();
    let mut mem = crate::memory::load_memory(&chats_dir, chat_id, uid)
        .unwrap_or_else(|| crate::memory::Memory::new(uid, ""));
    let mut changed = false;
    if let Some(v) = args["username"].as_str() {
        mem.frontmatter.username = v.into();
        changed = true;
    }
    if let Some(v) = args["call_name"].as_str() {
        mem.frontmatter.call_name = v.into();
        changed = true;
    }
    if let Some(v) = args["description"].as_str() {
        mem.frontmatter.description = v.into();
        changed = true;
    }
    if let Some(v) = args["log_entry"].as_str() {
        mem.append_log(v);
        changed = true;
    }
    if changed {
        match crate::memory::save_memory(&chats_dir, chat_id, uid, &mem) {
            Ok(()) => format!("Memory updated for {}", uid),
            Err(e) => format!("Error: {}", e),
        }
    } else {
        "No fields to update.".into()
    }
}

/// Handle the `read_chat_memory` tool — load the chat-level memory file.
pub(crate) async fn tool_read_chat_memory(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
) -> String {
    let s = state.lock().await;
    match crate::memory::load_chat_memory(&s.chats_dir(), chat_id) {
        Some(m) => serde_json::json!({
            "call_name": m.frontmatter.call_name,
            "description": m.frontmatter.description,
            "body": m.body,
        })
        .to_string(),
        None => format!("No chat memory for {}", chat_id),
    }
}

/// Handle the `update_chat_memory` tool — create or update the chat-level memory file.
pub(crate) async fn tool_update_chat_memory(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    args: &serde_json::Value,
) -> String {
    let s = state.lock().await;
    let chats_dir = s.chats_dir();
    let mut mem = crate::memory::load_chat_memory(&chats_dir, chat_id)
        .unwrap_or_else(crate::memory::Memory::new_chat);
    let mut changed = false;
    if let Some(v) = args["call_name"].as_str() {
        mem.frontmatter.call_name = v.into();
        changed = true;
    }
    if let Some(v) = args["description"].as_str() {
        mem.frontmatter.description = v.into();
        changed = true;
    }
    if let Some(v) = args["log_entry"].as_str() {
        mem.append_log(v);
        changed = true;
    }
    if changed {
        match crate::memory::save_chat_memory(&chats_dir, chat_id, &mem) {
            Ok(()) => "Chat memory updated".into(),
            Err(e) => format!("Error: {}", e),
        }
    } else {
        "No fields to update.".into()
    }
}
