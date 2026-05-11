use super::bot_dispatch::{dispatch_tool, log_tool_call_to};
use super::bot_heartbeat::run_heartbeat_task;
use super::*;
use crate::llm::mock::MockLlmBackend;
use crate::openrouter::{
    AssistantMessage, ChatCompletionResponse, ChatMessage, Choice, FunctionCall, ModelInfo, ToolCall,
};
use tempfile::TempDir;

async fn setup_test_bot() -> (GlowBot, TempDir, Arc<MockLlmBackend>) {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let config = crate::config::basic_config();
    let config_path = data_dir.join("config.yaml");
    config.save(&config_path).unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm.clone())
        .await
        .unwrap();
    (bot, dir, mock_llm)
}

async fn setup_test_bot_with_whitelisted_chat() -> (GlowBot, TempDir, Arc<MockLlmBackend>) {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let mut config = crate::config::basic_config();
    config.chats.insert(
        "-123".into(),
        crate::config::ChatConfig {
            interaction_mode: crate::config::InteractionMode::EveryMessage,
            command_whitelist: vec!["456".into()],
            interaction_whitelist: vec!["456".into()],
            ..Default::default()
        },
    );
    let config_path = data_dir.join("config.yaml");
    config.save(&config_path).unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm.clone())
        .await
        .unwrap();
    (bot, dir, mock_llm)
}

#[tokio::test]
async fn test_bot_creation() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let state = bot.state.lock().await;
    assert_eq!(state.config.telegram_token, "test-token");
}

#[tokio::test]
async fn test_bot_creation_nonexistent_config() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mock_llm = Arc::new(MockLlmBackend::new());
    let result = GlowBot::new_with_llm(&data_dir, mock_llm).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ensure_memory_exists() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    bot.ensure_memory_exists("-123", "456", "@testuser")
        .await
        .unwrap();

    let state = bot.state.lock().await;
    let mem = crate::memory::load_memory(&state.chats_dir(), "-123", "456");
    assert!(mem.is_some());
    assert_eq!(mem.unwrap().frontmatter.username, "@testuser");
}

#[tokio::test]
async fn test_reload_skills() {
    let (bot, dir, _mock) = setup_test_bot().await;

    use crate::skills::{write_skill, SkillFrontmatter};
    let skills_dir = dir.path().join("glowbot_data").join("skills");
    let fm = SkillFrontmatter {
        name: "test-skill".into(),
        description: "A test".into(),
    };
    write_skill(&skills_dir, "test-skill", &fm, "body text").unwrap();

    bot.reload_skills().await.unwrap();
    let state = bot.state.lock().await;
    assert!(state.skills.contains_key("test-skill"));
}

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
            crate::mcp::McpTool {
                server_name: "test-server".into(),
                name: "greet".into(),
                description: "Say hello".into(),
                input_schema: serde_json::json!({}),
                server_url: "http://localhost:9999".into(),
                api_key: None,
                session_id: None,
                transport: "streamable".into(),
            },
            crate::mcp::McpTool {
                server_name: "test-server".into(),
                name: "calculate".into(),
                description: "Do math".into(),
                input_schema: serde_json::json!({}),
                server_url: "http://localhost:9999".into(),
                api_key: None,
                session_id: None,
                transport: "streamable".into(),
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

#[tokio::test]
async fn test_save_config() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    // Saving should work without errors
    bot.save_config().await.unwrap();
}

#[tokio::test]
async fn test_process_message_bash_tool_error() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_err".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "bash".into(),
                        arguments: r#"{"command":"nonexistent_command_xyz"}"#.into(),
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

    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Command failed, but I handled it.".into()),
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
        .process_message("-123", "456", "@testuser", "Run bad command", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("Command failed, but I handled it.".into()));
}

#[tokio::test]
async fn test_process_message_with_chat_system_prompt() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let mut config = crate::config::basic_config();
    config.chats.insert(
        "-123".into(),
        crate::config::ChatConfig {
            interaction_mode: crate::config::InteractionMode::EveryMessage,
            interaction_whitelist: vec![],
            system_prompt: "Custom system prompt".into(),
            ..Default::default()
        },
    );
    config.save(&data_dir.join("config.yaml")).unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    mock_llm.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Response with custom prompt".into()),
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
        .process_message("-123", "456", "@testuser", "Hello", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("Response with custom prompt".into()));
}

#[tokio::test]
async fn test_new_with_llm_with_skills_dir_with_empty_subdirs() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();

    // Create a skills dir with a subdirectory that has no skill.md
    let skills_dir = data_dir.join("skills");
    std::fs::create_dir_all(skills_dir.join("empty_skill")).unwrap();
    // Also create a file directly (not a directory)
    std::fs::write(skills_dir.join("some_file.txt"), "not a skill").unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let state = bot.state.lock().await;
    assert!(state.skills.is_empty());
}

#[tokio::test]
async fn test_dm_always_responds_even_in_mention_only_mode() {
    // DMs have positive chat IDs (not starting with '-')
    // They should always respond, even with mention_only default
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut config = crate::config::basic_config();
    config.dms.insert(
        "123456789".into(),
        crate::config::DmConfig::default(),
    );
    config.save(&data_dir.join("config.yaml")).unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    mock_llm.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("DM response!".into()),
                tool_calls: None,
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    // Positive chat ID = DM, default mention_only mode
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let result = bot
        .process_message("123456789", "456", "@testuser", "Hello in DM", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("DM response!".into()));
}

#[tokio::test]
async fn test_process_message_with_read_memory_tool() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    // First ensure memory exists
    bot.ensure_memory_exists("-123", "456", "@testuser")
        .await
        .unwrap();

    // LLM calls read_memory
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_read".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "read_memory".into(),
                        arguments: r#"{"user_id":"456"}"#.into(),
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

    // Then LLM responds after reading memory
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("I remember you, @testuser!".into()),
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
        .process_message("-123", "456", "@testuser", "Who am I?", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("I remember you, @testuser!".into()));
}

#[tokio::test]
async fn test_process_message_with_update_memory_tool() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    // LLM calls update_memory
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_update".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "update_memory".into(),
                        arguments: r#"{"user_id":"456","call_name":"Learned","log_entry":"user said hello"}"#.into(),
                    },
                }]),
                role: Some("assistant".into()),
                reasoning: None,
            ..Default::default()
            },
            finish_reason: Some("tool_calls".into()),
        }],
    ..Default::default()});

    // LLM confirms
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("I've noted that!".into()),
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
        .process_message("-123", "456", "@testuser", "My name is Learned", "mybot")
        .await
        .unwrap();
    assert_eq!(result, Some("I've noted that!".into()));

    // Verify memory was actually updated
    let state = bot.state.lock().await;
    let mem = crate::memory::load_memory(&state.chats_dir(), "-123", "456").unwrap();
    assert_eq!(mem.frontmatter.call_name, "Learned");
    assert!(mem.body.contains("user said hello"));
}

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
    assert_eq!(tools.len(), 21);
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
    assert_eq!(tools.len(), 22);
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
    assert_eq!(tools.len(), 21); // same as without MCP
    assert!(!tools.iter().any(|t| t.function.name.starts_with("mcp_")));

    // Chat "-123" not blacklisted — MCP tool should be included
    let tools = state.build_tools(true, "-123");
    assert_eq!(tools.len(), 22);
    assert!(tools
        .iter()
        .any(|t| t.function.name == "mcp_test-srv_test_tool"));

    // DM chats are never blacklisted
    let tools = state.build_tools(true, "12345");
    assert_eq!(tools.len(), 22);
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
    assert_eq!(tools.len(), 20); // 17 base + 3 config tools
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

// ---------- dispatch_tool edge-case tests ----------

#[tokio::test]
async fn test_dispatch_send_message_empty_text() {
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_message",
        &serde_json::json!({"text":""}),
        None,
    )
    .await;
    assert_eq!(out, "Error: text required");
}

#[tokio::test]
async fn test_dispatch_send_message_no_tg_bot() {
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_message",
        &serde_json::json!({"text":"hi"}),
        None,
    )
    .await;
    assert_eq!(out, "Error: send_message not available in this context.");
}

#[tokio::test]
async fn test_dispatch_send_media_empty_file_path() {
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_media",
        &serde_json::json!({"file_path":""}),
        None,
    )
    .await;
    assert_eq!(out, "Error: file_path required");
}

#[tokio::test]
async fn test_dispatch_send_media_file_not_found() {
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_media",
        &serde_json::json!({"file_path":"nonexistent.png"}),
        None,
    )
    .await;
    assert!(out.starts_with("Error: file not found:"));
}

#[tokio::test]
async fn test_dispatch_send_media_no_tg_bot() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    // Create a dummy file so the file-exists check passes before the tg_bot check
    std::fs::write(data_dir.join("test.png"), b"fake png").unwrap();
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_media",
        &serde_json::json!({"file_path":"test.png"}),
        None,
    )
    .await;
    assert_eq!(out, "Error: send_media not available in this context.");
}

#[tokio::test]
async fn test_dispatch_send_media_original_quality() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("test.png"), b"fake png").unwrap();
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_media",
        &serde_json::json!({"file_path":"test.png", "original_quality": true}),
        None,
    )
    .await;
    // Without a tg_bot, still returns not available, but param is parsed correctly
    assert_eq!(out, "Error: send_media not available in this context.");
}

// ─── list_media dispatch tests ─────────────────────────────────────

fn setup_state_with_media_dir(media_dir: &std::path::Path) -> Arc<Mutex<BotState>> {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: Config {
            media_dir: media_dir.to_string_lossy().to_string(),
            ..crate::config::basic_config()
        },
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    state
}

#[tokio::test]
async fn test_dispatch_list_media_root_empty() {
    let media_dir = TempDir::new().unwrap();
    let state = setup_state_with_media_dir(media_dir.path());
    let out = dispatch_tool(&state, "-123", "list_media", &serde_json::json!({}), None).await;
    assert!(out.starts_with("Media directory listing for"));
    assert!(out.contains("(empty)"));
}

#[tokio::test]
async fn test_dispatch_list_media_with_files_and_dirs() {
    let media_dir = TempDir::new().unwrap();
    let media_path = media_dir.path();

    // Create some files
    std::fs::write(media_path.join("photo.jpg"), b"fake jpeg").unwrap();
    std::fs::write(media_path.join("notes.txt"), b"hello").unwrap();
    // Create a subdirectory with files
    std::fs::create_dir_all(media_path.join("images")).unwrap();
    std::fs::write(media_path.join("images/cat.png"), b"cat png").unwrap();
    std::fs::write(media_path.join("images/dog.jpg"), b"dog jpg").unwrap();
    // Another subdirectory (empty)
    std::fs::create_dir_all(media_path.join("videos")).unwrap();

    let state = setup_state_with_media_dir(media_path);
    let out = dispatch_tool(&state, "-123", "list_media", &serde_json::json!({}), None).await;
    assert!(out.starts_with("Media directory listing for"));
    assert!(out.contains("photo.jpg"));
    assert!(out.contains("notes.txt"));
    assert!(out.contains("images/"));
    assert!(out.contains("videos/"));
    assert!(out.contains("cat.png"));
    assert!(out.contains("dog.jpg"));
    // Verify size formatting
    assert!(out.contains("9 B")); // "fake jpeg" is 9 bytes
}

#[tokio::test]
async fn test_dispatch_list_media_subpath() {
    let media_dir = TempDir::new().unwrap();
    let media_path = media_dir.path();
    std::fs::create_dir_all(media_path.join("images/cats")).unwrap();
    std::fs::write(media_path.join("images/cats/fluffy.jpg"), b"meow").unwrap();
    std::fs::write(media_path.join("images/logo.png"), b"logo").unwrap();
    std::fs::write(media_path.join("root.txt"), b"top").unwrap();

    let state = setup_state_with_media_dir(media_path);
    let out = dispatch_tool(
        &state,
        "-123",
        "list_media",
        &serde_json::json!({"subpath": "images"}),
        None,
    )
    .await;
    assert!(out.contains("cats/"));
    assert!(out.contains("fluffy.jpg"));
    assert!(out.contains("logo.png"));
    assert!(
        !out.contains("root.txt"),
        "subpath should not show root files"
    );
}

#[tokio::test]
async fn test_dispatch_list_media_subpath_not_found() {
    let media_dir = TempDir::new().unwrap();
    let state = setup_state_with_media_dir(media_dir.path());
    let out = dispatch_tool(
        &state,
        "-123",
        "list_media",
        &serde_json::json!({"subpath": "nonexistent"}),
        None,
    )
    .await;
    assert!(out.starts_with("Error: directory not found:"));
}

#[tokio::test]
async fn test_dispatch_list_media_subpath_is_file() {
    let media_dir = TempDir::new().unwrap();
    let media_path = media_dir.path();
    std::fs::write(media_path.join("readme.md"), b"# media").unwrap();

    let state = setup_state_with_media_dir(media_path);
    let out = dispatch_tool(
        &state,
        "-123",
        "list_media",
        &serde_json::json!({"subpath": "readme.md"}),
        None,
    )
    .await;
    assert!(out.contains("is a file, not a directory"));
}

#[tokio::test]
async fn test_dispatch_list_media_path_traversal_blocked() {
    let media_dir = TempDir::new().unwrap();
    let state = setup_state_with_media_dir(media_dir.path());
    // Try to escape with ..
    let out = dispatch_tool(
        &state,
        "-123",
        "list_media",
        &serde_json::json!({"subpath": ".."}),
        None,
    )
    .await;
    assert!(out.starts_with("Error: invalid subpath"));
}

#[tokio::test]
async fn test_dispatch_list_media_root_missing() {
    let state = setup_state_with_media_dir(std::path::Path::new("/nonexistent/media/dir"));
    let out = dispatch_tool(&state, "-123", "list_media", &serde_json::json!({}), None).await;
    assert!(out.starts_with("Error: directory not found:"));
}

#[tokio::test]
async fn test_dispatch_bash_empty_command() {
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "bash",
        &serde_json::json!({"command":""}),
        None,
    )
    .await;
    assert!(out.contains("exit code"));
}

#[tokio::test]
async fn test_dispatch_bash_disabled() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut cfg = crate::config::basic_config();
    cfg.bash_enabled = false;
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "bash",
        &serde_json::json!({"command":"echo hi"}),
        None,
    )
    .await;
    assert!(
        out.contains("disabled"),
        "expected disabled message, got: {}",
        out
    );
}

#[tokio::test]
async fn test_dispatch_read_memory_missing() {
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "read_memory",
        &serde_json::json!({"user_id":"999"}),
        None,
    )
    .await;
    assert!(out.contains("No memory file found"));
}

#[tokio::test]
async fn test_dispatch_update_memory_no_fields() {
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "update_memory",
        &serde_json::json!({"user_id":"999"}),
        None,
    )
    .await;
    assert_eq!(out, "No fields to update.");
}

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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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
    run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), "-123", tg_bot).await;
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
    run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), "-123", tg_bot).await;

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
    run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), "-123", tg_bot).await;

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
    run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), "-123", tg_bot).await;

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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
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

// ─── embedding dispatch tests ───────────────────────────────

#[tokio::test]
async fn test_dispatch_search_conversations_no_model() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config(); // no embedding_model
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: data_dir.clone(),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": "hello"}),
        None,
    )
    .await;
    assert!(out.contains("not configured"));
}

#[tokio::test]
async fn test_dispatch_search_conversations_empty_query() {
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.embedding_model = Some("test-embed-model".into());
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: std::path::PathBuf::from("/tmp"),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": ""}),
        None,
    )
    .await;
    assert_eq!(out, "Error: query required");
}

#[tokio::test]
async fn test_dispatch_search_conversations_no_results() {
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.embedding_model = Some("test-embed-model".into());
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: std::path::PathBuf::from("/tmp"),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": "nothing here"}),
        None,
    )
    .await;
    assert_eq!(out, "No similar messages found.");
}

#[tokio::test]
async fn test_dispatch_search_conversations_with_results() {
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.embedding_model = Some("test-embed-model".into());
    cfg.embedding.search_limit = 5;

    let db = crate::db::Database::open_in_memory().unwrap();
    // Store a message and its embedding
    let ids = db
        .save_messages(
            "-123",
            &[ChatMessage::user("Alice talked about Rust programming")],
        )
        .unwrap();
    db.save_embedding(ids[0], &[1.0f32, 0.0, 0.0, 0.0], "test-embed-model")
        .unwrap();

    // Mock LLM returns a matching embedding query
    let mock_llm = Arc::new(MockLlmBackend::new());
    mock_llm.add_embedding(vec![1.0f32, 0.0, 0.0, 0.0]);

    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: mock_llm,
        data_dir: std::path::PathBuf::from("/tmp"),
        db,
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": "rust"}),
        None,
    )
    .await;
    assert!(out.contains("similarity"));
    assert!(out.contains("Rust"));
}

#[tokio::test]
async fn test_dispatch_search_conversations_embedding_error() {
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.embedding_model = Some("test-embed-model".into());

    let mock_llm = Arc::new(MockLlmBackend::new());
    mock_llm.set_error(true);

    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: mock_llm,
        data_dir: std::path::PathBuf::from("/tmp"),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": "test"}),
        None,
    )
    .await;
    assert!(out.contains("Error embedding"));
}

// ─── generate_image dispatch tests ──────────────────────────────────

#[tokio::test]
async fn test_dispatch_generate_image_no_model() {
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
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "generate_image",
        &serde_json::json!({"prompt": "a cat"}),
        None,
    )
    .await;
    assert!(out.contains("image generation model not configured"));
}

#[tokio::test]
async fn test_dispatch_generate_image_empty_prompt() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.image_gen_model = Some("test/image-model".into());
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "generate_image",
        &serde_json::json!({}),
        None,
    )
    .await;
    assert!(out.contains("prompt required"));
}

// ─── generate_image helper function tests ──────────────────────────

#[test]
fn test_base64_decode_standard() {
    let decoded =
        super::bot_dispatch::bot_dispatch_image::tests::base64_decode_for_test("aGVsbG8=").unwrap();
    assert_eq!(decoded, b"hello");
}

#[test]
fn test_base64_decode_with_data_url_prefix() {
    let decoded = super::bot_dispatch::bot_dispatch_image::tests::base64_decode_for_test(
        "data:image/png;base64,aGVsbG8=",
    )
    .unwrap();
    assert_eq!(decoded, b"hello");
}

#[test]
fn test_base64_decode_invalid() {
    let result = super::bot_dispatch::bot_dispatch_image::tests::base64_decode_for_test(
        "!!!not-base64!!!",
    );
    assert!(result.is_err());
}

#[test]
fn test_detect_image_format_png() {
    let data = [0x89, 0x50, 0x4E, 0x47, 0x00, 0x00];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        Some("png")
    );
}

#[test]
fn test_detect_image_format_jpg() {
    let data = [0xFF, 0xD8, 0xFF, 0x00, 0x00];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        Some("jpg")
    );
}

#[test]
fn test_detect_image_format_webp() {
    let mut data = vec![b'R', b'I', b'F', b'F', 0x00, 0x00, 0x00, 0x00];
    data.extend_from_slice(b"WEBP");
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        Some("webp")
    );
}

#[test]
fn test_detect_image_format_gif() {
    let data = [b'G', b'I', b'F', b'8', b'9', b'a'];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        Some("gif")
    );
}

#[test]
fn test_detect_image_format_unknown() {
    let data = [0x00, 0x00, 0x00, 0x00];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        None
    );
}

#[test]
fn test_image_gen_model_id() {
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::image_gen_model_id_for_test(
            "provider/model"
        ),
        "model"
    );
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::image_gen_model_id_for_test(
            "black-forest-labs/flux-1.1-pro",
        ),
        "flux-1_1-pro"
    );
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::image_gen_model_id_for_test("simple"),
        "simple"
    );
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::image_gen_model_id_for_test(
            "provider/model:route",
        ),
        "model_route"
    );
}

#[test]
fn test_guess_mime_png() {
    let data = [0x89, 0x50, 0x4E, 0x47];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::guess_mime_for_test(&data),
        "image/png"
    );
}

#[test]
fn test_guess_mime_jpeg() {
    let data = [0xFF, 0xD8, 0xFF];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::guess_mime_for_test(&data),
        "image/jpeg"
    );
}

#[test]
fn test_guess_mime_default() {
    let data = [0x00, 0x00, 0x00];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::guess_mime_for_test(&data),
        "image/png"
    );
}

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
