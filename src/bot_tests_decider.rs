async fn setup_decider_bot(value: serde_json::Value) -> (GlowBot, TempDir, wiremock::MockServer) {
    use wiremock::{
        matchers::{method, path},
        Mock, ResponseTemplate,
    };
    let (bot, dir, _) = setup_test_bot_with_whitelisted_chat().await;
    let server = wiremock::MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(&server)
        .await;
    {
        let mut s = bot.state.lock().await;
        s.config.chats.get_mut("-123").unwrap().interaction_mode =
            crate::config::InteractionMode::AutoDetect;
        s.config.openrouter.decider_model = Some("test/decider".into());
        let mut metadata = crate::codex::model_info("gpt-5.4");
        metadata.id = "test/decider".into();
        s.decision_metadata.insert(metadata.id.clone(), metadata);
        s.decision_test_url = Some(format!("{}/decisions", server.uri()));
    }
    (bot, dir, server)
}

#[tokio::test]
async fn auto_detect_ignores_but_retains_message_and_context() {
    let (bot, _dir, server) = setup_decider_bot(
        serde_json::json!({"answers":{"should_respond":{"type":"noul","noul":0.59}}}),
    )
    .await;
    assert!(bot
        .process_message(
            "-123",
            "456",
            "alice",
            "He is talking about the bot",
            "mybot"
        )
        .await
        .unwrap()
        .is_none());
    assert!(bot
        .process_message("-123", "456", "alice", "And another thing", "mybot")
        .await
        .unwrap()
        .is_none());
    let history = bot
        .state
        .lock()
        .await
        .db
        .load_messages("-123", 20, None)
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().all(|m| m.role == "user"));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let second: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    let text = second["state"].to_string();
    assert!(text.contains("He is talking about the bot"));
    assert_eq!(text.matches("And another thing").count(), 1);
}

#[tokio::test]
async fn auto_detect_threshold_runs_agent_and_saves_user_once() {
    let (bot, _dir, server) = setup_decider_bot(
        serde_json::json!({"answers":{"should_respond":{"type":"noul","noul":0.6}}}),
    )
    .await;
    assert!(bot
        .process_message("-123", "456", "alice", "Can you help?", "mybot")
        .await
        .unwrap()
        .is_some());
    let history = bot
        .state
        .lock()
        .await
        .db
        .load_messages("-123", 20, None)
        .unwrap();
    assert_eq!(history.iter().filter(|m| m.role == "user").count(), 1);
    assert_eq!(history.iter().filter(|m| m.role == "assistant").count(), 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn auto_detect_mentions_and_commands_bypass_and_whitelist_blocks() {
    let (bot, _dir, server) = setup_decider_bot(serde_json::json!({"answers":{}})).await;
    assert!(bot
        .process_message("-123", "456", "alice", "@mybot hello", "mybot")
        .await
        .unwrap()
        .is_some());
    assert!(bot
        .process_message("-123", "456", "alice", "/status", "mybot")
        .await
        .unwrap()
        .is_some());
    assert!(bot
        .process_message("-123", "999", "eve", "hello", "mybot")
        .await
        .unwrap()
        .is_none());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn auto_detect_invalid_answer_stays_silent_and_saves_message() {
    let (bot, _dir, _server) = setup_decider_bot(
        serde_json::json!({"answers":{"should_respond":{"type":"noul","noul":2.0}}}),
    )
    .await;
    assert!(bot
        .process_message("-123", "456", "alice", "hello", "mybot")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        bot.state
            .lock()
            .await
            .db
            .load_messages("-123", 20, None)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn auto_detect_config_requires_model_and_key() {
    let mut config = crate::config::basic_config();
    config.chats.insert(
        "-123".into(),
        crate::config::ChatConfig {
            interaction_mode: crate::config::InteractionMode::AutoDetect,
            ..Default::default()
        },
    );
    assert!(config.validate().is_err());
    config.openrouter.decider_model = Some("   ".into());
    assert!(config.validate().is_err());
    config.openrouter.decider_model = Some("test/decider".into());
    assert!(config.validate().is_ok());
    let encoded = serde_yaml::to_string(&config).unwrap();
    let decoded: crate::config::Config = serde_yaml::from_str(&encoded).unwrap();
    assert_eq!(
        decoded.chats["-123"].interaction_mode,
        crate::config::InteractionMode::AutoDetect
    );
    config.openrouter.api_key.clear();
    assert!(config.validate().is_err());
}

#[tokio::test]
async fn auto_detect_stop_during_decision_keeps_history_without_agent() {
    use wiremock::{matchers::method, Mock, ResponseTemplate};
    let (bot, _dir, server) = setup_decider_bot(serde_json::json!({"answers":{}})).await;
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(
                    serde_json::json!({"answers":{"should_respond":{"type":"noul","noul":0.9}}}),
                )
                .set_delay(std::time::Duration::from_millis(100)),
        )
        .mount(&server)
        .await;
    let stop = async {
        loop {
            if !server.received_requests().await.unwrap().is_empty() {
                bot.stop_signals
                    .lock()
                    .unwrap()
                    .get("-123")
                    .unwrap()
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    };
    let (result, _) = tokio::join!(
        bot.process_message("-123", "456", "alice", "Can you help?", "mybot"),
        stop
    );
    assert!(result.unwrap().is_none());
    assert_eq!(
        bot.state
            .lock()
            .await
            .db
            .load_messages("-123", 20, None)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn auto_detect_failed_media_stays_silent_without_calling_decider() {
    let (bot, _dir, server) = setup_decider_bot(serde_json::json!({"answers":{}})).await;
    let media = crate::media::IngestedMedia::Voice {
        file_id: "voice".into(),
        duration: 5,
    };
    let result = process_message_impl(
        &bot.state,
        &bot.git_repo,
        &bot.stop_signals,
        "-123",
        "456",
        "alice",
        None,
        None,
        Some(&media),
        None,
        None,
        "mybot",
        None,
    )
    .await
    .unwrap();
    assert!(result.is_none());
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        bot.state
            .lock()
            .await
            .db
            .load_messages("-123", 20, None)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn auto_detect_caption_mention_bypasses_decider() {
    let (bot, _dir, server) = setup_decider_bot(serde_json::json!({"answers":{}})).await;
    let result = process_message_impl(
        &bot.state,
        &bot.git_repo,
        &bot.stop_signals,
        "-123",
        "456",
        "alice",
        None,
        Some("@mybot hello"),
        None,
        None,
        None,
        "mybot",
        None,
    )
    .await
    .unwrap();
    assert!(result.is_some());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn decider_refresh_validates_catalog_and_preserves_cache_on_failure() {
    use wiremock::{matchers::method, Mock, ResponseTemplate};
    let (bot, _dir, server) = setup_decider_bot(serde_json::json!({"answers":{}})).await;
    server.reset().await;
    bot.state.lock().await.decision_metadata.clear();
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[{"id":"test/decider","context_length":10000,"architecture":{"input_modalities":["text","image"]}}]}))).mount(&server).await;
    assert_eq!(
        super::bot_decider::model_for_decision(&bot.state)
            .await
            .unwrap(),
        "test/decider"
    );
    assert!(bot.state.lock().await.decision_metadata["test/decider"].supports_modality("image"));
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    assert!(super::bot_decider::refresh(&bot.state).await.is_err());
    assert!(super::bot_decider::model_for_decision(&bot.state)
        .await
        .is_ok());
    bot.state.lock().await.config.openrouter.decider_model = Some("missing/model".into());
    assert!(bot
        .process_message("-123", "456", "alice", "hello", "mybot")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        bot.state
            .lock()
            .await
            .db
            .load_messages("-123", 20, None)
            .unwrap()
            .len(),
        1
    );
    bot.state.lock().await.config.openrouter.decider_model = None;
    assert!(super::bot_decider::model_for_decision(&bot.state)
        .await
        .is_err());
}

#[tokio::test]
async fn image_capable_decider_receives_native_current_and_history_images() {
    use crate::openrouter::{ContentPart, ImageUrlDetail};
    let (bot, _dir, server) = setup_decider_bot(
        serde_json::json!({"answers":{"should_respond":{"type":"noul","noul":0.7}}}),
    )
    .await;
    let message = ChatMessage::user_multimodal(vec![
        ContentPart::Text {
            text: "What is this?".into(),
        },
        ContentPart::ImageUrl {
            image_url: ImageUrlDetail {
                url: "data:image/png;base64,YQ==".into(),
                detail: None,
            },
        },
    ]);
    assert!(super::bot_decider::judge(
        &bot.state,
        "-123",
        "test/decider",
        "mybot",
        std::slice::from_ref(&message),
        &message
    )
    .await
    .unwrap());
    let requests = server.received_requests().await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        payload["state"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "image_url")
            .count(),
        2
    );
    let for_agent = super::bot_decider::for_agent(&bot.state, "-123", message.clone()).await;
    assert!(for_agent.text_content().contains("What is this?"));
    assert!(!serde_json::to_string(&for_agent)
        .unwrap()
        .contains("image_url"));
    bot.state
        .lock()
        .await
        .model_metadata
        .insert("test/model".into(), crate::codex::model_info("gpt-5.4"));
    let for_agent = super::bot_decider::for_agent(&bot.state, "-123", message).await;
    assert!(serde_json::to_string(&for_agent)
        .unwrap()
        .contains("image_url"));
}
