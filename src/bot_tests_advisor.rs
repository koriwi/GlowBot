// --- ask_advisor dispatch tests ---

#[tokio::test]
async fn test_dispatch_ask_advisor_no_advice_model() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    let args = serde_json::json!({"query": "What should I do?"});
    let result = dispatch_tool(&state, "-123", "ask_advisor", &args, None).await;
    assert!(result.contains("advice model not configured"));
}

#[tokio::test]
async fn test_dispatch_ask_advisor_empty_query() {
    let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
    let state = bot.state.clone();

    let args = serde_json::json!({"query": ""});
    let result = dispatch_tool(&state, "-123", "ask_advisor", &args, None).await;
    assert!(result.contains("query required"));
}

#[tokio::test]
async fn test_dispatch_ask_advisor_with_model() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    // Configure advice model
    {
        let mut s = bot.state.lock().await;
        s.config.openrouter.advice_model = Some("advisor/model".into());
    }

    // Add a response to the mock LLM for the advisor call
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Here is my advice...".into()),
                ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let state = bot.state.clone();
    let args = serde_json::json!({"query": "What do you think about this?"});
    let result = dispatch_tool(&state, "-123", "ask_advisor", &args, None).await;
    assert!(result.contains("Advisor response:"));
    assert!(result.contains("Here is my advice..."));
}

#[tokio::test]
async fn test_dispatch_ask_advisor_with_chat_override() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let mut config = crate::config::basic_config();
    config.openrouter.advice_model = Some("global-advice".into());
    config.chats.insert(
        "-456".into(),
        crate::config::ChatConfig {
            advice_model: Some("chat-advice".into()),
            ..Default::default()
        },
    );
    config.save(&data_dir.join("config.yaml")).unwrap();

    let mock = Arc::new(MockLlmBackend::new());
    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Chat-specific advice".into()),
                ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });
    let bot = GlowBot::new_with_llm(&data_dir, mock).await.unwrap();

    let state = bot.state.clone();
    let args = serde_json::json!({"query": "test"});
    let result = dispatch_tool(&state, "-456", "ask_advisor", &args, None).await;
    assert!(result.contains("Advisor response:"));
    assert!(result.contains("Chat-specific advice"));
}

#[tokio::test]
async fn test_dispatch_ask_advisor_with_recent_messages() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    // Configure advice model and pre-seed conversation history
    {
        let mut s = bot.state.lock().await;
        s.config.openrouter.advice_model = Some("advisor/model".into());
        s.config.conversation.advice_recent_messages_window_size = 3;
        // Pre-seed some conversation messages
        let msgs = vec![
            ChatMessage::user_with_name("Hello", "Alice"),
            ChatMessage::assistant("Hi Alice!"),
            ChatMessage::user_with_name("What is Rust?", "Alice"),
        ];
        s.db.save_messages("-123", &msgs).unwrap();
    }

    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Rust is a systems programming language...".into()),
                ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let state = bot.state.clone();
    let args = serde_json::json!({"query": "Explain Rust"});
    let result = dispatch_tool(&state, "-123", "ask_advisor", &args, None).await;
    assert!(result.contains("Advisor response:"));
    assert!(result.contains("Rust is a systems programming language"));
}

#[tokio::test]
async fn test_dispatch_ask_advisor_with_reasoning() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    {
        let mut s = bot.state.lock().await;
        s.config.openrouter.advice_model = Some("advisor/model".into());
        s.config.conversation.advice_recent_messages_window_size = 2;
        // Reasoning is always included — seed messages with reasoning
        let msgs = vec![
            ChatMessage::user_with_name("Question", "Alice"),
            ChatMessage::assistant_with_reasoning("Answer", "I think this is because...".into()),
        ];
        s.db.save_messages("-123", &msgs).unwrap();
    }

    mock.add_response(ChatCompletionResponse {
        choices: vec![Choice {
            message: AssistantMessage {
                content: Some("Good reasoning!".into()),
                ..Default::default()
            },
            finish_reason: Some("stop".into()),
        }],
        ..Default::default()
    });

    let state = bot.state.clone();
    let args = serde_json::json!({"query": "Is this correct?"});
    let result = dispatch_tool(&state, "-123", "ask_advisor", &args, None).await;
    assert!(result.contains("Advisor response:"));
    assert!(result.contains("Good reasoning!"));
}

#[tokio::test]
async fn test_dispatch_ask_advisor_llm_error() {
    let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

    {
        let mut s = bot.state.lock().await;
        s.config.openrouter.advice_model = Some("advisor/model".into());
    }

    // Add an error response (empty choices)
    mock.add_response(ChatCompletionResponse {
        choices: vec![],
        ..Default::default()
    });

    let state = bot.state.clone();
    let args = serde_json::json!({"query": "What should I do?"});
    let result = dispatch_tool(&state, "-123", "ask_advisor", &args, None).await;
    // Should return advisor response with empty content (choices is empty)
    assert!(result.contains("Advisor response:"));
}
