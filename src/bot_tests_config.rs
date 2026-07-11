// --- Config tool tests ---

#[tokio::test]
async fn test_process_message_command_config_redacts_sensitive() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // Add an MCP server with an API key to verify it gets redacted
    {
        let mut state = bot.state.lock().await;
        state.config.mcp_servers.push(crate::config::McpServer {
            name: "test-mcp".into(),
            transport: "streamable".into(),
            url: "http://localhost:9999".into(),
            api_key: Some("secret-mcp-key".into()),
        });
        state.config.mcp_servers.push(crate::config::McpServer {
            name: "no-key-mcp".into(),
            transport: "http".into(),
            url: "http://localhost:8888".into(),
            api_key: None,
        });
    }
    let result = bot
        .process_message("-123", "456", "@testuser", "/config", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    // Should contain config YAML but NOT the real token, API key, or MCP keys
    assert!(resp.contains("```yaml"));
    assert!(resp.contains("telegram_token"));
    assert!(resp.contains("[REDACTED]"));
    assert!(!resp.contains("test-token"));
    assert!(!resp.contains("test-key"));
    assert!(!resp.contains("secret-mcp-key"));
    // Should contain other config fields
    assert!(resp.contains("openrouter"));
    assert!(resp.contains("model:"));
    // MCP server details should still be visible (except the key)
    assert!(resp.contains("test-mcp"));
    assert!(resp.contains("no-key-mcp"));
    assert!(resp.contains("http://localhost:9999"));
}

#[tokio::test]
async fn test_process_message_command_config_schema() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/config_schema", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    // Should contain JSON schema with config field names
    assert!(resp.contains("```json"));
    assert!(resp.contains("ChatConfig"));
    assert!(resp.contains("DmConfig"));
    assert!(resp.contains("telegram_token"));
    assert!(resp.contains("openrouter"));
}

#[tokio::test]
async fn test_process_message_command_config_unauthorized() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    let result = bot
        .process_message("-123", "999", "@unknown", "/config", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("not authorized"));
}

#[tokio::test]
async fn test_process_message_command_config_schema_unauthorized() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    let result = bot
        .process_message("-123", "999", "@unknown", "/config_schema", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("not authorized"));
}

#[tokio::test]
async fn test_process_message_command_model_default() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // Set a temporary override first
    {
        let mut s = bot.state.lock().await;
        s.model_overrides.insert("-123".into(), "override/model".into());
    }
    let result = bot
        .process_message("-123", "456", "@testuser", "/model_default", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("Model reset to config default"));
    assert!(resp.contains("test/model"));
    // Verify override was cleared
    let s = bot.state.lock().await;
    assert!(!s.model_overrides.contains_key("-123"));
}

#[tokio::test]
async fn test_process_message_command_model_reset_alias() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    {
        let mut s = bot.state.lock().await;
        s.model_overrides.insert("-123".into(), "override/model".into());
    }
    let result = bot
        .process_message("-123", "456", "@testuser", "/model_reset", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("Model reset to config default"));
}

#[tokio::test]
async fn test_effective_model_respects_override() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    {
        let mut s = bot.state.lock().await;
        s.model_overrides.insert("-123".into(), "override/model".into());
    }
    let s = bot.state.lock().await;
    // effective_model should return the override, not the config default
    assert_eq!(s.effective_model("-123"), "override/model");
    // Without override, returns config default
    assert_eq!(s.effective_model("-999"), "test/model");
}

#[tokio::test]
async fn test_process_message_command_model_no_args() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/model", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    // Without tg_bot, should return text help
    assert!(resp.contains("Current model"));
    assert!(resp.contains("test/model"));
    assert!(resp.contains("Set model"));
    assert!(resp.contains("Switch routing"));
}

#[tokio::test]
async fn test_process_message_command_model_no_args_with_override() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    {
        let mut s = bot.state.lock().await;
        s.model_overrides.insert("-123".into(), "override/model:nitro".into());
    }
    let result = bot
        .process_message("-123", "456", "@testuser", "/model", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("override/model:nitro"));
    assert!(resp.contains("override"));
    assert!(resp.contains("test/model"));
}

#[tokio::test]
async fn test_process_message_command_model_specifier_nitro() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // Set current model with :free specifier
    {
        let mut s = bot.state.lock().await;
        s.model_overrides.insert("-123".into(), "test/model:free".into());
    }
    let result = bot
        .process_message("-123", "456", "@testuser", "/model :nitro", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("Model set to"));
    assert!(resp.contains("test/model:nitro"));
    assert!(resp.contains(":nitro"));
    // Verify override was applied
    let s = bot.state.lock().await;
    assert_eq!(s.model_overrides.get("-123").unwrap(), "test/model:nitro");
}

#[tokio::test]
async fn test_process_message_command_model_specifier_floor() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/model :floor", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("Model set to"));
    assert!(resp.contains("test/model:floor"));
    let s = bot.state.lock().await;
    assert_eq!(s.model_overrides.get("-123").unwrap(), "test/model:floor");
}

#[tokio::test]
async fn test_process_message_command_model_specifier_free() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/model :free", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("test/model:free"));
}

#[tokio::test]
async fn test_process_message_command_model_specifier_invalid() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/model :fast", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("Unknown specifier"));
    assert!(resp.contains(":nitro"));
    assert!(resp.contains(":floor"));
    assert!(resp.contains(":free"));
}

#[tokio::test]
async fn test_process_message_command_model_direct() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/model openai/gpt-4o", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("Model set to"));
    assert!(resp.contains("openai/gpt-4o"));
    let s = bot.state.lock().await;
    assert_eq!(s.model_overrides.get("-123").unwrap(), "openai/gpt-4o");
}

#[tokio::test]
async fn test_process_message_command_model_direct_with_specifier() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/model google/gemini-2.5-pro:nitro", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("Model set to"));
    assert!(resp.contains("google/gemini-2.5-pro:nitro"));
    let s = bot.state.lock().await;
    assert_eq!(s.model_overrides.get("-123").unwrap(), "google/gemini-2.5-pro:nitro");
}

#[tokio::test]
async fn test_codex_model_commands_match_openrouter_controls() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    {
        let mut s = bot.state.lock().await;
        s.config.codex = Some(crate::config::CodexConfig {
            model: "gpt-5.4".into(),
            auth_file: "auth.json".into(),
            reasoning_effort: None,
            base_url: "https://chatgpt.com/backend-api".into(),
        });
        s.config.chats.get_mut("-123").unwrap().provider = Some(crate::config::LlmProvider::Codex);
    }

    let info = bot
        .process_message("-123", "456", "@testuser", "/model", "mybot")
        .await
        .unwrap()
        .unwrap();
    assert!(info.contains("Current Codex model"));
    assert!(info.contains("gpt-5.4"));

    let unsupported = bot
        .process_message("-123", "456", "@testuser", "/model :nitro", "mybot")
        .await
        .unwrap()
        .unwrap();
    assert!(unsupported.contains("only available with OpenRouter"));

    let changed = bot
        .process_message("-123", "456", "@testuser", "/model gpt-5.5", "mybot")
        .await
        .unwrap()
        .unwrap();
    assert!(changed.contains("gpt-5.5"));
    {
        let s = bot.state.lock().await;
        assert_eq!(s.effective_model("-123"), "gpt-5.5");
        assert!(s.model_metadata.contains_key("gpt-5.5"));
    }

    let reset = bot
        .process_message("-123", "456", "@testuser", "/model_default", "mybot")
        .await
        .unwrap()
        .unwrap();
    assert!(reset.contains("gpt-5.4"));
    let status = bot
        .process_message("-123", "456", "@testuser", "/status", "mybot")
        .await
        .unwrap()
        .unwrap();
    assert!(status.contains("Provider: codex"));
    assert!(status.contains("Model: gpt-5.4"));

    // With no Telegram client the picker reports the same limitation as OpenRouter,
    // rather than treating Codex as unsupported.
    let picker = bot
        .process_message("-123", "456", "@testuser", "/models", "mybot")
        .await
        .unwrap()
        .unwrap();
    assert!(picker.contains("no Telegram bot available"));
}

#[tokio::test]
async fn test_process_message_command_model_unauthorized() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // User 789 is not in the command whitelist
    let result = bot
        .process_message("-123", "789", "@testuser", "/model :nitro", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("not authorized"));
}

#[tokio::test]
async fn test_read_config_tool_returns_yaml() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    let result = super::bot_dispatch::bot_dispatch_config::tool_read_config(&state).await;

    // Should contain key config fields
    assert!(result.contains("telegram_token:"));
    assert!(result.contains("test-token"));
    assert!(result.contains("openrouter:"));
    assert!(result.contains("model:"));
}

#[tokio::test]
async fn test_read_config_schema_returns_valid_schema() {
    let result =
        super::bot_dispatch::bot_dispatch_config::tool_read_config_schema().await;

    // Should be valid JSON
    let v: serde_json::Value =
        serde_json::from_str(&result).expect("schema should be valid JSON");

    // Should be a JSON Schema object with properties
    assert_eq!(v["type"], "object");
    let props = &v["properties"];
    assert!(props["telegram_token"].is_object());
    assert!(props["openrouter"].is_object());
    assert!(props["chats"].is_object());
    assert!(props["dms"].is_object());
    assert!(props["mcp_servers"].is_object());
    assert!(props["heartbeat_interval_minutes"].is_object());
    assert!(props["bash_enabled"].is_object());
    assert!(props["media_dir"].is_object());
    assert!(props["embedding"].is_object());
    assert!(props["conversation"].is_object());
    assert!(props["db"].is_object());

    // Verify nested ChatConfig schema is accessible too
    // The schema should describe ChatConfig (used in chats HashMap values)
    let schema_str = result;
    assert!(schema_str.contains("ChatConfig"));
    assert!(schema_str.contains("DmConfig"));
    assert!(schema_str.contains("model"));
    assert!(schema_str.contains("interaction_mode"));
    assert!(schema_str.contains("image_fallback_model"));
    assert!(schema_str.contains("audio_fallback_model"));
    assert!(schema_str.contains("image_gen_model"));
}

#[tokio::test]
async fn test_edit_config_missing_config_yaml() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    let args = serde_json::json!({});
    let result =
        super::bot_dispatch::bot_dispatch_config::tool_edit_config(&state, "-123", &args, None)
            .await;

    assert!(result.starts_with("Error: config_yaml parameter required"));
}

#[tokio::test]
async fn test_edit_config_invalid_yaml() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    let args = serde_json::json!({"config_yaml": "this is not valid yaml: [unclosed"});
    let result =
        super::bot_dispatch::bot_dispatch_config::tool_edit_config(&state, "-123", &args, None)
            .await;

    assert!(result.starts_with("Error: invalid YAML"));
}

#[tokio::test]
async fn test_edit_config_no_changes() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    // Serialize current config back to YAML
    let current_yaml = serde_yaml::to_string(&state.lock().await.config).unwrap();

    let args = serde_json::json!({"config_yaml": current_yaml});
    let result =
        super::bot_dispatch::bot_dispatch_config::tool_edit_config(&state, "-123", &args, None)
            .await;

    assert!(result.contains("Config unchanged"));
}

#[tokio::test]
async fn test_edit_config_requires_tg_bot() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    // Valid yaml with a change, but no tg_bot
    let new_yaml = "telegram_token: test-token\nopenrouter:\n  api_key: test-key\n  model: new/model\n";
    let args = serde_json::json!({"config_yaml": new_yaml});
    let result =
        super::bot_dispatch::bot_dispatch_config::tool_edit_config(&state, "-123", &args, None)
            .await;

    assert!(result.contains("requires Telegram bot context"));
}

#[tokio::test]
async fn test_handle_config_callback_accept() {
    let (bot, dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    // Create a new config YAML with a change
    let new_yaml = "telegram_token: test-token\nopenrouter:\n  api_key: test-key\n  model: changed/model\n";

    // Store a pending change
    let pending_id = "test123";
    {
        let mut s = state.lock().await;
        s.pending_config_changes.insert(
            pending_id.to_string(),
            PendingConfigChange {
                chat_id: "-123".to_string(),
                message_id: 42,
                new_yaml: new_yaml.to_string(),
            },
        );
    }

    let data = format!("cfg:{}:accept", pending_id);
    let result =
        super::bot_dispatch::bot_dispatch_config::handle_config_callback(&state, &data, None).await;

    assert!(result.is_some());
    let (edit_text, followup) = result.unwrap();
    assert!(edit_text.contains("Config Change Applied") || edit_text.contains("restarting"));
    assert!(followup.is_none());

    // Verify the config was saved to disk
    let saved_config =
        crate::config::Config::load(&dir.path().join("glowbot_data").join("config.yaml"))
            .unwrap();
    assert_eq!(saved_config.openrouter.model, "changed/model");
}

#[tokio::test]
async fn test_handle_config_callback_deny() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    let new_yaml = "telegram_token: test-token\nopenrouter:\n  api_key: test-key\n  model: some/model\n";

    let pending_id = "deny_test";
    {
        let mut s = state.lock().await;
        s.pending_config_changes.insert(
            pending_id.to_string(),
            PendingConfigChange {
                chat_id: "-123".to_string(),
                message_id: 42,
                new_yaml: new_yaml.to_string(),
            },
        );
    }

    let data = format!("cfg:{}:deny", pending_id);
    let result =
        super::bot_dispatch::bot_dispatch_config::handle_config_callback(&state, &data, None).await;

    assert!(result.is_some());
    let (edit_text, followup) = result.unwrap();
    assert!(edit_text.contains("Config Change Denied"));
    assert!(followup.is_some());
    assert!(followup.unwrap().contains("user denied the proposed config change"));

    // Verify the pending change was removed
    assert!(state.lock().await.pending_config_changes.is_empty());
}

#[tokio::test]
async fn test_handle_config_callback_unknown_pending_id() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    let data = "cfg:nonexistent:accept";
    let result =
        super::bot_dispatch::bot_dispatch_config::handle_config_callback(&state, data, None).await;

    assert!(result.is_some());
    let (edit_text, followup) = result.unwrap();
    assert!(edit_text.contains("expired") || edit_text.contains("already processed"));
    assert!(followup.is_none());
}

#[tokio::test]
async fn test_handle_config_callback_unknown_action() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    let new_yaml = "telegram_token: test-token\nopenrouter:\n  api_key: test-key\n  model: x/model\n";

    let pending_id = "unknown_action_test";
    {
        let mut s = state.lock().await;
        s.pending_config_changes.insert(
            pending_id.to_string(),
            PendingConfigChange {
                chat_id: "-123".to_string(),
                message_id: 42,
                new_yaml: new_yaml.to_string(),
            },
        );
    }

    let data = format!("cfg:{}:something_else", pending_id);
    let result =
        super::bot_dispatch::bot_dispatch_config::handle_config_callback(&state, &data, None).await;

    assert!(result.is_some());
    let (edit_text, _followup) = result.unwrap();
    assert!(edit_text.contains("Unknown action"));
}

#[tokio::test]
async fn test_handle_config_callback_non_cfg_prefix() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    let result =
        super::bot_dispatch::bot_dispatch_config::handle_config_callback(&state, "something_else", None)
            .await;

    // Non-cfg callbacks return None (handled in main.rs)
    assert!(result.is_none());
}

