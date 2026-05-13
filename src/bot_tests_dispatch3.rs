#[tokio::test]
async fn test_dispatch_add_task_empty() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "add_task",
        &serde_json::json!({"description":""}),
        None,
    )
    .await;
    assert_eq!(out, "Error: description required");
}

#[tokio::test]
async fn test_dispatch_list_tasks_non_empty() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: data_dir.clone(),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    // Add a task first
    dispatch_tool(
        &state,
        "-123",
        "add_task",
        &serde_json::json!({"description":"do the thing"}),
        None,
    )
    .await;
    let out = dispatch_tool(&state, "-123", "list_tasks", &serde_json::json!({}), None).await;
    assert!(out.contains("do the thing"));
}

#[tokio::test]
async fn test_dispatch_remove_task_empty_and_not_found() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: data_dir.clone(),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "remove_task",
        &serde_json::json!({"id":""}),
        None,
    )
    .await;
    assert_eq!(out, "Error: id required");
    let out = dispatch_tool(
        &state,
        "-123",
        "remove_task",
        &serde_json::json!({"id":"nope"}),
        None,
    )
    .await;
    assert!(out.contains("not found"));
}

#[tokio::test]
async fn test_dispatch_create_skill_validation() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: data_dir.clone(),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "create_skill",
        &serde_json::json!({"name":"","description":"","body":""}),
        None,
    )
    .await;
    assert!(out.contains("required"));
}

#[tokio::test]
async fn test_dispatch_read_skill_not_found() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "read_skill",
        &serde_json::json!({"name":"ghost"}),
        None,
    )
    .await;
    assert!(out.contains("not found"));
}

#[tokio::test]
async fn test_dispatch_update_skill_not_found() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "update_skill",
        &serde_json::json!({"name":"missing","description":"d","body":"b"}),
        None,
    )
    .await;
    assert!(out.contains("not found"));
}

#[tokio::test]
async fn test_dispatch_unknown_tool() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(&state, "-123", "narnia", &serde_json::json!({}), None).await;
    assert!(out.contains("Unknown tool"));
}

#[tokio::test]
async fn test_dispatch_mcp_tool_not_found() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(&state, "-123", "mcp_no_no", &serde_json::json!({}), None).await;
    assert!(out.contains("MCP tool not found"));
}

#[tokio::test]
async fn test_get_recent_messages_empty_history() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "get_recent_messages",
        &serde_json::json!({"count":5}),
        None,
    )
    .await;
    assert!(out.contains("messages"));
}

#[tokio::test]
async fn test_process_message_plain_command_ignored() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // "/notabotcommand" -> starts with / but isn't a recognised command -> ignored
    let result = bot
        .process_message("-123", "456", "@testuser", "/notabotcommand", "mybot")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_conversation_history_window_trims() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;
    // Seed history with exactly 20 items (default conversation_window=20)
    let msgs: Vec<ChatMessage> = (0..10)
        .flat_map(|i| {
            vec![
                ChatMessage::user(&format!("msg{i}")),
                ChatMessage::assistant(&format!("reply{i}")),
            ]
        })
        .collect();
    {
        let state = bot.state.lock().await;
        state.db.save_messages("-123", &msgs).unwrap();
    }
    // one more exchange will trigger trimming via the query window
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("ok".into()),
                tool_calls: None,
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });
    let _ = bot
        .process_message("-123", "456", "user", "hello", "mybot")
        .await
        .unwrap();
    let h_len = {
        let state = bot.state.lock().await;
        state.db.load_messages("-123", 20, None).unwrap().len()
    };
    assert_eq!(h_len, 20);
}

