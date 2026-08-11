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

#[derive(Default)]
struct RecordingTextReplySender {
    messages: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl crate::bot_send::TextReplySender for RecordingTextReplySender {
    async fn send_text(&self, text: &str) -> anyhow::Result<()> {
        self.messages.lock().unwrap().push(text.to_string());
        Ok(())
    }
}

#[tokio::test]
async fn test_sms_and_telegram_share_dm_conversation_history() {
    let (bot, _dir, mock) = setup_test_bot().await;
    bot.state.lock().await.config.dms.insert(
        "123".into(),
        crate::config::DmConfig {
            name: Some("Alice".into()),
            phone_number: Some("+49123".into()),
            ..Default::default()
        },
    );
    for response in ["telegram reply", "sms “reply” 😀"] {
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some(response.into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                    reasoning: None,
                    ..Default::default()
                },
                finish_reason: Some("stop".into()),
            }],
            ..Default::default()
        });
    }

    bot.process_message("123", "123", "@alice", "from telegram", "mybot")
        .await
        .unwrap();
    let sender = RecordingTextReplySender::default();
    let response = process_sms_message_impl(
        &bot.state,
        &bot.git_repo,
        &bot.stop_signals,
        "123",
        "+49 123",
        "from sms",
        "2026-08-11 12:00:00",
        &sender,
    )
    .await
    .unwrap();
    assert_eq!(response.as_deref(), Some("sms \"reply\" "));

    let state = bot.state.lock().await;
    let messages = state.db.load_messages("123", 10, None).unwrap();
    assert_eq!(messages.len(), 4);
    let user_messages: Vec<_> = messages
        .iter()
        .filter(|message| message.role == "user")
        .map(ChatMessage::text_content)
        .collect();
    assert!(user_messages[0].contains("[Telegram message metadata]"));
    assert!(user_messages[0].contains("from telegram"));
    assert!(user_messages[1].contains("[SMS message metadata]"));
    assert!(user_messages[1].contains("Sender phone: +49 123"));
    assert!(user_messages[1].contains("Modem date: 2026-08-11 12:00:00"));
    assert!(user_messages[1].contains("Mapped Telegram chat ID: 123"));
    assert!(user_messages[1].contains("from sms"));
    assert_eq!(messages.last().unwrap().text_content(), "sms \"reply\" ");
}

#[tokio::test]
async fn test_sms_send_message_tool_uses_sms_reply_sender() {
    let (bot, _dir, mock) = setup_test_bot().await;
    bot.state
        .lock()
        .await
        .config
        .dms
        .insert("123".into(), crate::config::DmConfig::default());
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "sms_tool".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "send_message".into(),
                        arguments: r#"{"text":"Working on it"}"#.into(),
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
                content: Some("Done".into()),
                tool_calls: None,
                role: Some("assistant".into()),
                reasoning: None,
                ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let sender = RecordingTextReplySender::default();
    let response = process_sms_message_impl(
        &bot.state,
        &bot.git_repo,
        &bot.stop_signals,
        "123",
        "+49123",
        "do something",
        "2026-08-11 12:00:00",
        &sender,
    )
    .await
    .unwrap();
    assert_eq!(response.as_deref(), Some("Done"));
    assert_eq!(
        sender.messages.lock().unwrap().as_slice(),
        &["Working on it"]
    );
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

