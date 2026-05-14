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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::BotState;
    use crate::llm::mock::MockLlmBackend;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    async fn make_state() -> (Arc<Mutex<BotState>>, TempDir) {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let config = crate::config::basic_config();
        config.save(&data_dir.join("config.yaml")).unwrap();
        let mock_llm: Arc<dyn crate::llm::LlmBackend> = Arc::new(MockLlmBackend::new());
        let state = Arc::new(Mutex::new(BotState {
            config,
            skills: std::collections::HashMap::new(),
            llm: mock_llm,
            data_dir: data_dir.clone(),
            db: crate::db::Database::open_in_memory().unwrap(),
            mcp_tools: vec![],
            _mcp_services: vec![],
            mcp_peers: std::collections::HashMap::new(),
            model_metadata: std::collections::HashMap::new(),
            model_order: vec![],
            last_usage: std::collections::HashMap::new(),
            pending_config_changes: std::collections::HashMap::new(),
            pending_model_changes: std::collections::HashMap::new(),
            model_overrides: std::collections::HashMap::new(),
            last_browse_cb: std::collections::HashMap::new(),
        }));
        (state, dir)
    }

    // ─── read_memory ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_read_memory_missing() {
        let (state, _dir) = make_state().await;
        let args = json!({"user_id": "999"});
        let result = tool_read_memory(&state, "-123", &args).await;
        assert!(result.starts_with("No memory file found"));
    }

    #[tokio::test]
    async fn test_read_memory_exists() {
        let (state, _dir) = make_state().await;
        let mem = crate::memory::Memory::new("456", "@testuser");
        crate::memory::save_memory(
            &state.lock().await.chats_dir(),
            "-123",
            "456",
            &mem,
        )
        .unwrap();

        let args = json!({"user_id": "456"});
        let result = tool_read_memory(&state, "-123", &args).await;
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["user_id"], "456");
        assert_eq!(v["username"], "@testuser");
    }

    // ─── update_memory ───────────────────────────────────────────

    #[tokio::test]
    async fn test_update_memory_create() {
        let (state, _dir) = make_state().await;
        let args = json!({
            "user_id": "789",
            "username": "@newuser",
            "call_name": "Newbie",
            "description": "A new user",
            "log_entry": "First seen"
        });
        let result = tool_update_memory(&state, "-123", &args).await;
        assert!(result.contains("Memory updated"));

        let mem =
            crate::memory::load_memory(&state.lock().await.chats_dir(), "-123", "789");
        assert!(mem.is_some());
        let m = mem.unwrap();
        assert_eq!(m.frontmatter.username, "@newuser");
        assert_eq!(m.frontmatter.call_name, "Newbie");
        assert_eq!(m.frontmatter.description, "A new user");
        assert!(!m.body.is_empty());
    }

    #[tokio::test]
    async fn test_update_memory_partial_fields() {
        let (state, _dir) = make_state().await;
        let mem = crate::memory::Memory::new("456", "@original");
        crate::memory::save_memory(
            &state.lock().await.chats_dir(),
            "-123",
            "456",
            &mem,
        )
        .unwrap();

        let args = json!({"user_id": "456", "call_name": "Updated"});
        let result = tool_update_memory(&state, "-123", &args).await;
        assert!(result.contains("Memory updated"));

        let m = crate::memory::load_memory(&state.lock().await.chats_dir(), "-123", "456")
            .unwrap();
        assert_eq!(m.frontmatter.call_name, "Updated");
        assert_eq!(m.frontmatter.username, "@original");
    }

    #[tokio::test]
    async fn test_update_memory_no_fields() {
        let (state, _dir) = make_state().await;
        let args = json!({"user_id": "456"});
        let result = tool_update_memory(&state, "-123", &args).await;
        assert_eq!(result, "No fields to update.");
    }

    #[tokio::test]
    async fn test_update_memory_empty_user_id() {
        let (state, _dir) = make_state().await;
        let args = json!({"user_id": ""});
        let result = tool_update_memory(&state, "-123", &args).await;
        assert_eq!(result, "Error: user_id is required");
    }

    // ─── read_chat_memory ────────────────────────────────────────

    #[tokio::test]
    async fn test_read_chat_memory_missing() {
        let (state, _dir) = make_state().await;
        let result = tool_read_chat_memory(&state, "-123").await;
        assert_eq!(result, "No chat memory for -123");
    }

    #[tokio::test]
    async fn test_read_chat_memory_exists() {
        let (state, _dir) = make_state().await;
        let mem = crate::memory::Memory::new_chat();
        crate::memory::save_chat_memory(&state.lock().await.chats_dir(), "-123", &mem)
            .unwrap();

        let result = tool_read_chat_memory(&state, "-123").await;
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(v["call_name"].is_string());
        assert!(v["description"].is_string());
    }

    // ─── update_chat_memory ──────────────────────────────────────

    #[tokio::test]
    async fn test_update_chat_memory_create() {
        let (state, _dir) = make_state().await;
        let args = json!({
            "call_name": "Test Chat",
            "description": "A test group",
            "log_entry": "Enlisted"
        });
        let result = tool_update_chat_memory(&state, "-123", &args).await;
        assert_eq!(result, "Chat memory updated");

        let m =
            crate::memory::load_chat_memory(&state.lock().await.chats_dir(), "-123");
        assert!(m.is_some());
        assert_eq!(m.unwrap().frontmatter.call_name, "Test Chat");
    }

    #[tokio::test]
    async fn test_update_chat_memory_no_fields() {
        let (state, _dir) = make_state().await;
        let args = json!({});
        let result = tool_update_chat_memory(&state, "-123", &args).await;
        assert_eq!(result, "No fields to update.");
    }
}
