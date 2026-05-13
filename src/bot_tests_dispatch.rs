#[tokio::test]
async fn test_log_tool_call() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    log_tool_call_to(
        &data_dir,
        "bash",
        r#"{"command":"echo hi"}"#,
        "stdout: hi\n",
    );

    let log_path = data_dir.join("tool_calls.log");
    assert!(log_path.exists());
    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(content.contains("bash"));
    assert!(content.contains("echo hi"));
}

#[tokio::test]
async fn test_dm_blocked_by_default() {
    let (bot, _dir, _mock) = setup_test_bot().await;

    // DM (positive chat ID), no dms config entry -> blocked
    let result = bot
        .process_message("123", "456", "@test", "Hello", "mybot")
        .await
        .unwrap();
    assert!(result.unwrap().contains("I don't know you"));
}

#[tokio::test]
async fn test_dm_blocked_when_dms_nonempty_and_no_entry() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let mut config = crate::config::basic_config();
    // Having ANY dms entries implies control → unknown DMs blocked
    config.dms.insert(
        "999".into(),
        crate::config::DmConfig {
            commands_enabled: true,
            ..Default::default()
        },
    );
    config.save(&data_dir.join("config.yaml")).unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();

    // User 456 is NOT in dms -> blocked
    let result = bot
        .process_message("123", "456", "@test", "Hello", "mybot")
        .await
        .unwrap();
    assert!(result.unwrap().contains("I don't know you"));
}

#[tokio::test]
async fn test_dm_allowed_when_in_dms() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let mut config = crate::config::basic_config();
    config.dms.insert(
        "123".into(),
        crate::config::DmConfig {
            commands_enabled: true,
            ..Default::default()
        },
    );
    config.save(&data_dir.join("config.yaml")).unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    mock_llm.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Full access!".into()),
                tool_calls: None,
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let result = bot
        .process_message("123", "456", "@test", "Hello", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("Full access!".into()));
}

#[tokio::test]
async fn test_dm_blocked_message_contains_user_id() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let config = crate::config::basic_config();
    config.save(&data_dir.join("config.yaml")).unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();

    let result = bot
        .process_message("123", "789012", "@stranger", "Hello", "mybot")
        .await
        .unwrap();
    let msg = result.unwrap();
    assert!(msg.contains("I don't know you"));
    assert!(
        msg.contains("789012"),
        "message should include user_id: {}",
        msg
    );
}

#[tokio::test]
async fn test_heartbeat_disabled_when_zero() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut config = crate::config::basic_config();
    config.chats.insert(
        "-123".into(),
        crate::config::ChatConfig {
            heartbeat_interval_minutes: Some(0),
            ..Default::default()
        },
    );
    config.save(&data_dir.join("config.yaml")).unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let state = bot.state.lock().await;
    assert_eq!(state.config.heartbeat_interval("-123"), None);
}

#[tokio::test]
async fn test_heartbeat_has_pending_tasks() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();

    let state = bot.state.lock().await;
    assert!(!state.has_pending_tasks("-123"));

    let mut list = crate::tasks::TaskList::default();
    list.add("test task");
    list.save(&state.chats_dir(), "-123").unwrap();

    assert!(state.has_pending_tasks("-123"));
}

#[tokio::test]
async fn test_build_tools_includes_mcp() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let mut state = bot.state.lock().await;

    // No MCP tools yet — all tool definitions with bash
    let tools = state.build_tools(true, "-123");
    assert_eq!(tools.len(), 23);
    assert!(tools.iter().any(|t| t.function.name == "send_message"));
    assert!(tools.iter().any(|t| t.function.name == "bash"));

    // Add a fake MCP tool
    state.mcp_tools.push(crate::mcp::McpTool {
        server_name: "test-srv".into(),
        name: "test_tool".into(),
        description: "A test".into(),
        input_schema: serde_json::json!({"type": "object"}),
        server_url: "https://example.com".into(),
        api_key: None,
        session_id: None,
        transport: "streamable".into(),
    });

    let tools = state.build_tools(true, "-123");
    assert_eq!(tools.len(), 24);
    assert!(tools
        .iter()
        .any(|t| t.function.name == "mcp_test-srv_test_tool"));
}

#[tokio::test]
async fn test_build_tools_mcp_blacklist() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Create a config with a blacklisted MCP server for chat "-456"
    let mut config = crate::config::basic_config();
    config.chats.insert(
        "-456".into(),
        crate::config::ChatConfig {
            mcp_blacklist: vec!["test-srv".into()],
            ..Default::default()
        },
    );
    config.save(&data_dir.join("config.yaml")).unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let mut state = bot.state.lock().await;

    // Add a fake MCP tool
    state.mcp_tools.push(crate::mcp::McpTool {
        server_name: "test-srv".into(),
        name: "test_tool".into(),
        description: "A test".into(),
        input_schema: serde_json::json!({"type": "object"}),
        server_url: "https://example.com".into(),
        api_key: None,
        session_id: None,
        transport: "streamable".into(),
    });

    // Chat "-456" has the server blacklisted — MCP tool should be excluded
    let tools = state.build_tools(true, "-456");
    assert_eq!(tools.len(), 23); // same as without MCP
    assert!(!tools.iter().any(|t| t.function.name.starts_with("mcp_")));

    // Chat "-123" not blacklisted — MCP tool should be included
    let tools = state.build_tools(true, "-123");
    assert_eq!(tools.len(), 24);
    assert!(tools
        .iter()
        .any(|t| t.function.name == "mcp_test-srv_test_tool"));

    // DM chats are never blacklisted
    let tools = state.build_tools(true, "12345");
    assert_eq!(tools.len(), 24);
    assert!(tools
        .iter()
        .any(|t| t.function.name == "mcp_test-srv_test_tool"));
}

#[tokio::test]
async fn test_build_tools_without_bash() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let state = bot.state.lock().await;

    let tools = state.build_tools(false, "-123");
    assert_eq!(tools.len(), 22); // 17 base + 3 config + 2 model tools
    assert!(!tools.iter().any(|t| t.function.name == "bash"));
    assert!(tools.iter().any(|t| t.function.name == "send_message"));
}

#[tokio::test]
async fn test_get_recent_messages_tool() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    // Pre-seed some conversation history
    {
        let state = bot.state.lock().await;
        let msgs = vec![
            ChatMessage::user_with_name("Hello bot", "Alice"),
            ChatMessage::assistant("Hi Alice!"),
            ChatMessage::user_with_name("What's my name?", "Alice"),
            ChatMessage::assistant("Your name is Alice."),
        ];
        state.db.save_messages("-123", &msgs).unwrap();
    }

    // LLM calls get_recent_messages(count: 2)
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_recent".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "get_recent_messages".into(),
                        arguments: r#"{"count":2}"#.into(),
                    },
                }]),
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("tool_calls".into()),
        }],
        ..Default::default()
    });

    // After reading context, LLM responds
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("I recall our conversation!".into()),
                tool_calls: None,
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let result = bot
        .process_message("-123", "456", "@alice", "Recall what I said", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("I recall our conversation!".into()));
}

