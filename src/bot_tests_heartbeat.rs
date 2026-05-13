// ---------- heartbeat tests ----------

#[tokio::test]
async fn test_heartbeat_no_tasks() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();
    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let tg_bot = teloxide::Bot::new("ignored");
    run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), bot.stop_signals.clone(), "-123", tg_bot).await;
}

#[tokio::test]
async fn test_heartbeat_completes_task() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();
    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm.clone())
        .await
        .unwrap();

    // add a pending task
    let mut list = crate::tasks::TaskList::default();
    let id = list.add("heartbeat task");
    list.save(&data_dir.join("chats"), "-123").unwrap();

    // LLM returns a remove_task call to complete it
    mock_llm.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_rm".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "remove_task".into(),
                        arguments: format!(r##"{{"id":"{}"}}"##, id),
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

    mock_llm.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Done!".into()),
                tool_calls: None,
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let tg_bot = teloxide::Bot::new("ignored");
    run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), bot.stop_signals.clone(), "-123", tg_bot).await;

    let list = crate::tasks::TaskList::load(&data_dir.join("chats"), "-123").unwrap_or_default();
    assert!(list.tasks.is_empty());
}

#[tokio::test]
async fn test_heartbeat_llm_error() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();
    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm.clone())
        .await
        .unwrap();

    let mut list = crate::tasks::TaskList::default();
    list.add("error task");
    list.save(&data_dir.join("chats"), "-123").unwrap();

    // configure mock to error
    mock_llm.set_error(true);

    let tg_bot = teloxide::Bot::new("ignored");
    run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), bot.stop_signals.clone(), "-123", tg_bot).await;

    // task should still be there after error
    let list = crate::tasks::TaskList::load(&data_dir.join("chats"), "-123").unwrap_or_default();
    assert_eq!(list.tasks.len(), 1);
}

#[tokio::test]
async fn test_heartbeat_two_tasks_first_uncompleted() {
    // Bug fix: when the first task is left uncompleted (no remove_task),
    // the heartbeat must still process the second task.
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();
    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm.clone())
        .await
        .unwrap();

    let mut list = crate::tasks::TaskList::default();
    let id1 = list.add("task one — not done yet");
    let id2 = list.add("task two — can complete");
    list.save(&data_dir.join("chats"), "-123").unwrap();

    // First LLM response: no tool calls (task left pending)
    mock_llm.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Still waiting…".into()),
                tool_calls: None,
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    // Second LLM response: remove_task for the second task
    mock_llm.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_rm".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "remove_task".into(),
                        arguments: format!(r#"{{"id":"{}"}}"#, id2),
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

    // Third response: after remove_task, LLM finishes
    mock_llm.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Done!".into()),
                tool_calls: None,
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let tg_bot = teloxide::Bot::new("ignored");
    run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), bot.stop_signals.clone(), "-123", tg_bot).await;

    let list = crate::tasks::TaskList::load(&data_dir.join("chats"), "-123").unwrap_or_default();
    assert_eq!(list.tasks.len(), 1, "task one should still be pending");
    assert_eq!(list.tasks[0].id, id1, "only task one should remain");
}

#[tokio::test]
async fn test_process_message_command_run_no_tg_bot() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // /run via process_message (no tg_bot) should say not available
    let result = bot
        .process_message("-123", "456", "@testuser", "/run", "mybot")
        .await
        .unwrap();
    assert!(result.unwrap().contains("cannot be used in this context"));
}

#[tokio::test]
async fn test_process_message_command_run_unauthorized() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // Default: command_whitelist is empty, so nobody can run commands
    let result = bot
        .process_message("-123", "456", "@testuser", "/run", "mybot")
        .await
        .unwrap();
    assert_eq!(
        result,
        Some("You are not authorized to run bot commands.".into())
    );
}

#[tokio::test]
async fn test_process_message_command_new() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/new", "mybot")
        .await
        .unwrap();
    let text = result.unwrap();
    assert!(text.contains("Context reset"), "got: {}", text);

    // Verify the cutoff was stored
    let cutoff = {
        let state = bot.state.lock().await;
        state.db.get_cutoff("-123").unwrap()
    };
    assert!(cutoff.is_some());
    // Should be recent (within last 5 seconds)
    let now = chrono::Utc::now().timestamp();
    assert!(
        (now - cutoff.unwrap()).abs() < 5,
        "cutoff {} not close to now {}",
        cutoff.unwrap(),
        now
    );
}

#[tokio::test]
async fn test_process_message_command_new_unauthorized() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // Default: command_whitelist is empty, so nobody can run commands
    let result = bot
        .process_message("-123", "456", "@testuser", "/new", "mybot")
        .await
        .unwrap();
    assert_eq!(
        result,
        Some("You are not authorized to run bot commands.".into())
    );
}

#[tokio::test]
async fn test_conversation_history_respects_cutoff() {
    let (bot, _dir, mock_llm) = setup_test_bot_with_whitelisted_chat().await;

    // Set a cutoff in the future so ALL existing messages are filtered
    {
        let state = bot.state.lock().await;
        state
            .db
            .set_cutoff("-123", chrono::Utc::now().timestamp() + 3600)
            .unwrap();
    }

    // Queue a simple LLM response
    mock_llm.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Hello from the future!".into()),
                tool_calls: None,
                reasoning: None,
                role: Some("assistant".into()),
            ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        usage: None,
    });

    // Save an old message first
    {
        let state = bot.state.lock().await;
        state
            .db
            .save_messages("-123", &[ChatMessage::user("old history")])
            .unwrap();
    }

    let result = bot
        .process_message("-123", "456", "@testuser", "Hi!", "mybot")
        .await
        .unwrap();

    // Should still respond (current message is always included)
    assert_eq!(result, Some("Hello from the future!".into()));
}

#[test]
fn test_context_usage_formatting() {
    let mut state = BotState {
        config: crate::config::basic_config(),
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: std::path::PathBuf::from("/tmp"),
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
    };
    // No context cached, no usage -> no token data yet
    assert_eq!(state.context_usage("-123"), "no token data yet");
    // Cache model metadata for the default model
    state
        .model_metadata
        .insert("test/model".into(), ModelInfo {
            id: "test/model".into(),
            name: String::new(),
            created: 0,
            context_length: 200000,
            architecture: Default::default(),
            pricing: Default::default(),
        });
    // Effective limit = 200k * 0.75 = 150k
    assert_eq!(state.context_usage("-123"), "0k/150k (0%)");
    // Set usage
    state.last_usage.insert(
        "-123".into(),
        crate::openrouter::Usage {
            prompt_tokens: 37000,
            completion_tokens: 500,
            total_tokens: 37500,
        },
    );
    assert_eq!(state.context_usage("-123"), "37k/150k (25%)");
}

