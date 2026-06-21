#[tokio::test]
async fn test_process_message_mention_only_ignores() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // Default is MentionOnly, so non-mention messages should be ignored
    let result = bot
        .process_message("-123", "456", "@testuser", "Hello world", "mybot")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_process_message_mention_responds() {
    let (bot, _dir, mock) = setup_test_bot().await;

    // Set up mock to return a simple response
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Hello, I'm GlowBot!".into()),
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
        .process_message("-123", "456", "@testuser", "@mybot Hello!", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("Hello, I'm GlowBot!".into()));
}

#[tokio::test]
async fn test_process_message_every_message_mode() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Got your message!".into()),
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
        .process_message("-123", "456", "@testuser", "Hello", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("Got your message!".into()));
}

#[tokio::test]
async fn test_process_message_includes_sender_and_sent_time_metadata() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("metadata seen".into()),
                tool_calls: None,
                role: Some("assistant".into()),
                reasoning: None,
                ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let sent_at = chrono::DateTime::parse_from_rfc3339("2026-06-21T12:34:56Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let result = process_message_impl(
        &bot.state,
        &bot.git_repo,
        &bot.stop_signals,
        "-123",
        "456",
        "@testuser",
        Some("Hello with metadata"),
        None,
        None,
        Some("Test User"),
        Some(sent_at),
        "mybot",
        None,
    )
    .await
    .unwrap();
    assert_eq!(result, Some("metadata seen".into()));

    let state = bot.state.lock().await;
    let messages = state.db.load_messages("-123", 10, None).unwrap();
    let user_message = messages.iter().find(|m| m.role == "user").unwrap();
    let content = user_message.text_content();
    assert!(content.contains("[Telegram message metadata]"));
    assert!(content.contains("Sent at: 2026-06-21T12:34:56+00:00"));
    assert!(content.contains("Sender ID: 456"));
    assert!(content.contains("Sender name: Test User"));
    assert!(content.contains("Sender username: @testuser"));
    assert!(content.contains("Message:\nHello with metadata"));
}

#[tokio::test]
async fn test_process_message_interaction_whitelist_blocks() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // User "789" is not in interaction_whitelist
    let result = bot
        .process_message("-123", "789", "@other", "Hello", "mybot")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_process_message_command_unauthorized() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // Default: command_whitelist is empty, so nobody can run commands
    let result = bot
        .process_message("-123", "456", "@testuser", "/status", "mybot")
        .await
        .unwrap();
    assert_eq!(
        result,
        Some("You are not authorized to run bot commands.".into())
    );
}

#[tokio::test]
async fn test_process_message_command_authorized() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // User "456" is in the command_whitelist
    let result = bot
        .process_message("-123", "456", "@testuser", "/status", "mybot")
        .await
        .unwrap();
    assert!(result.unwrap().contains("Chat ID:"));
}

#[tokio::test]
async fn test_process_message_command_blocked_by_command_whitelist() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // User "789" is not in the command_whitelist → blocked
    let result = bot
        .process_message("-123", "789", "@otheruser", "/status", "mybot")
        .await
        .unwrap();
    assert_eq!(
        result,
        Some("You are not authorized to run bot commands.".into())
    );
}

#[tokio::test]
async fn test_process_message_command_stop() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/stop", "mybot")
        .await
        .unwrap();
    assert!(result.unwrap().contains("Stop signal sent"));
}

#[tokio::test]
async fn test_process_message_command_tasks_empty() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/tasks", "mybot")
        .await
        .unwrap();
    assert!(result.unwrap().contains("No pending tasks"));
}

#[tokio::test]
async fn test_process_message_command_tasks_with_tasks() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // Add a task first
    {
        let state = bot.state.lock().await;
        let mut list = crate::tasks::TaskList::load(&state.chats_dir(), "-123").unwrap_or_default();
        list.add("Test task one");
        list.add("Test task two");
        list.save(&state.chats_dir(), "-123").unwrap();
    }
    let result = bot
        .process_message("-123", "456", "@testuser", "/tasks", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("2 pending task"));
    assert!(resp.contains("Test task one"));
    assert!(resp.contains("Test task two"));
}

#[tokio::test]
async fn test_process_message_command_prompt() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/prompt", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    // Should contain the system prompt
    assert!(resp.contains("GlowBot"));
    assert!(!resp.contains("CONVERSATION HISTORY"));
}

#[tokio::test]
async fn test_process_message_command_tools_basic() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let result = bot
        .process_message("-123", "456", "@testuser", "/tools", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("Available Tools"));
    assert!(resp.contains("Built-in"));
    // Built-in tools should be listed
    assert!(resp.contains("`bash`"));
    assert!(resp.contains("`read_memory`"));
    assert!(resp.contains("`update_memory`"));
    assert!(resp.contains("`send_message`"));
    assert!(resp.contains("`add_task`"));
    // No MCP servers section when none configured
    assert!(!resp.contains("MCP Servers"));
    assert!(!resp.contains("MCP:"));
}

#[tokio::test]
async fn test_process_message_command_tools_with_mcp() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // Inject MCP tools into state
    {
        let mut s = bot.state.lock().await;
        s.mcp_tools = vec![
            crate::mcp::McpToolInfo {
                server_name: "test-server".into(),
                name: "greet".into(),
                description: "Say hello".into(),
                input_schema: serde_json::json!({}),
            },
            crate::mcp::McpToolInfo {
                server_name: "test-server".into(),
                name: "calculate".into(),
                description: "Do math".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        s.config.mcp_servers = vec![crate::config::McpServer {
            name: "test-server".into(),
            transport: "streamable".into(),
            url: "http://localhost:9999".into(),
            api_key: None,
        }];
    }
    let result = bot
        .process_message("-123", "456", "@testuser", "/tools", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("Available Tools"));
    assert!(resp.contains("MCP Servers"));
    assert!(resp.contains("test-server"));
    assert!(resp.contains("http://localhost:9999"));
    assert!(resp.contains("streamable"));
    assert!(resp.contains("2 tool(s)"));
    assert!(resp.contains("Built-in"));
    assert!(resp.contains("MCP: test-server"));
    assert!(resp.contains("`mcp_test\u{2d}server_greet`"));
    assert!(resp.contains("`mcp_test\u{2d}server_calculate`"));
}

#[tokio::test]
async fn test_process_message_command_tools_unauthorized() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    // user 789 is not in command_whitelist
    let result = bot
        .process_message("-123", "789", "@other", "/tools", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("not authorized"));
}

#[tokio::test]
async fn test_process_message_command_tools_dm_not_authorized() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // DM from user without DM config — commands not enabled
    let result = bot
        .process_message("12345", "12345", "@rando", "/tools", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(resp.contains("not authorized"));
}

#[tokio::test]
async fn test_process_message_command_todos_group_bypasses_whitelist() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // Group chat, command_whitelist empty → nobody normally runs commands,
    // but /todos is exempt from all authorization checks.
    let result = bot
        .process_message("-123", "456", "@testuser", "/todos", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(!resp.contains("not authorized"));
}

#[tokio::test]
async fn test_process_message_command_todos_dm_always_allowed() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // DM from user without DM config — /todos is always allowed
    let result = bot
        .process_message("12345", "12345", "@rando", "/todos", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(!resp.contains("not authorized"));
}

#[tokio::test]
async fn test_process_message_command_tasks_dm_always_allowed() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // DM from user without DM config — /tasks is always allowed
    let result = bot
        .process_message("12345", "12345", "@rando", "/tasks", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(!resp.contains("not authorized"));
    assert!(resp.contains("No pending tasks"));
}

#[tokio::test]
async fn test_process_message_command_reminders_dm_always_allowed() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // DM from user without DM config — /reminders is always allowed
    let result = bot
        .process_message("12345", "12345", "@rando", "/reminders", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(!resp.contains("not authorized"));
}

#[tokio::test]
async fn test_process_message_command_stop_dm_always_allowed() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // DM from user without DM config — /stop is always allowed
    let result = bot
        .process_message("12345", "12345", "@rando", "/stop", "mybot")
        .await
        .unwrap();
    let resp = result.unwrap();
    assert!(!resp.contains("not authorized"));
    assert!(resp.contains("Stop signal sent"));
}

#[tokio::test]
async fn test_process_message_with_tool_call() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    // First response: tool call
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "bash".into(),
                        arguments: r#"{"command":"echo hello from bash"}"#.into(),
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

    // Second response: final text after tool result
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("The bash command succeeded!".into()),
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
        .process_message("-123", "456", "@testuser", "Run echo", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("The bash command succeeded!".into()));
}

#[tokio::test]
async fn test_process_message_empty_choices() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    mock.add_response(ChatCompletionResponse {
        choices: vec![],
        ..Default::default()
    });

    let result = bot
        .process_message("-123", "456", "@testuser", "Hello", "mybot")
        .await
        .unwrap();
    // Empty choices falls through to the loop error message
    assert!(result.unwrap().contains("loop"));
}

#[tokio::test]
async fn test_process_message_empty_tool_calls() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Final answer".into()),
                tool_calls: Some(vec![]),
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let result = bot
        .process_message("-123", "456", "@testuser", "Hello", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("Final answer".into()));
}

#[tokio::test]
async fn test_process_message_loop_limit() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    // Continuously return tool calls to trigger the loop limit
    for _ in 0..64 {
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_x".into(),
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: "bash".into(),
                            arguments: r#"{"command":"echo loop"}"#.into(),
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
    }

    let result = bot
        .process_message("-123", "456", "@testuser", "Loop test", "mybot")
        .await
        .unwrap();
    assert!(result.unwrap().contains("loop"));
}

