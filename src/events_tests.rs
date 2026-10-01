use super::*;
use crate::llm::LlmBackend;
use crate::openrouter::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct RecordingLlm {
    requests: std::sync::Mutex<Vec<ChatCompletionRequest>>,
    rounds: AtomicUsize,
    fail: bool,
}
#[async_trait::async_trait]
impl LlmBackend for RecordingLlm {
    async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> anyhow::Result<ChatCompletionResponse> {
        self.requests.lock().unwrap().push(ChatCompletionRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            tool_choice: None,
            modalities: None,
            image_config: None,
        });
        anyhow::ensure!(!self.fail, "test failure");
        let round = self.rounds.fetch_add(1, Ordering::SeqCst);
        let message = if round == 0 {
            AssistantMessage {
                tool_calls: Some(vec![ToolCall {
                    id: "read".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "read_chat_memory".into(),
                        arguments: "{}".into(),
                    },
                }]),
                reasoning: Some("Consult memory".into()),
                ..Default::default()
            }
        } else {
            AssistantMessage {
                content: Some("No action needed".into()),
                ..Default::default()
            }
        };
        Ok(ChatCompletionResponse {
            choices: vec![Choice {
                message,
                finish_reason: None,
            }],
            ..Default::default()
        })
    }
    async fn embeddings(&self, _: &str, _: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![])
    }
}

async fn setup(fail: bool) -> (EventContext, tempfile::TempDir, Arc<RecordingLlm>) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data).unwrap();
    let mut config = crate::config::basic_config();
    config.bash_enabled = false;
    config.chats.insert(
        "-123".into(),
        crate::config::ChatConfig {
            system_prompt: "Put finished files in the archive".into(),
            ..Default::default()
        },
    );
    config
        .dms
        .insert("456".into(), crate::config::DmConfig::default());
    config.save(&data.join("config.yaml")).unwrap();
    let llm = Arc::new(RecordingLlm {
        requests: Default::default(),
        rounds: AtomicUsize::new(0),
        fail,
    });
    let bot = GlowBot::new_with_llm(&data, llm.clone()).await.unwrap();
    {
        let state = bot.state.lock().await;
        state
            .db
            .save_messages(
                "-123",
                &[ChatMessage::user("Previous archive instructions")],
            )
            .unwrap();
        let mut memory = crate::memory::Memory::new_chat();
        memory.frontmatter.description = "Chat memory archive rule".into();
        crate::memory::save_memory(&state.chats_dir(), "-123", "_chat", &memory).unwrap();
    }
    (
        EventContext {
            bot: Arc::new(Mutex::new(bot)),
            telegram: teloxide::Bot::new("ignored"),
            chat_locks: Default::default(),
        },
        dir,
        llm,
    )
}

#[tokio::test]
async fn event_context_tools_and_history_are_reused_and_saved() {
    let (context, _dir, llm) = setup(false).await;
    let bot = context.bot.lock().await;
    crate::bot::run_event(
        bot.state.clone(),
        bot.stop_signals.clone(),
        "-123",
        &serde_json::json!({"kind":"finished"}),
        &context.telegram,
    )
    .await
    .unwrap();
    let requests = llm.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let serialized = serde_json::to_string(&requests[0].messages).unwrap();
    for expected in [
        "Put finished files",
        "Chat memory archive rule",
        "Previous archive instructions",
        "Background Event",
        "finished",
    ] {
        assert!(serialized.contains(expected), "missing {expected}");
    }
    assert!(!requests[0]
        .tools
        .as_ref()
        .unwrap()
        .iter()
        .any(|t| t.function.name == "bash"));
    assert!(requests[1].messages.iter().any(|m| m.role == "tool"));
    let state = bot.state.lock().await;
    let history = state.db.load_messages("-123", 20, None).unwrap();
    let saved = serde_json::to_string(&history).unwrap();
    assert!(saved.contains("Background Event"));
    assert!(saved.contains("No action needed"));
    assert!(state.last_usage.contains_key("-123"));
}

#[tokio::test]
async fn rejects_destinations_atomically_and_accepts_repeated_callbacks() {
    let (context, _dir, llm) = setup(false).await;
    for ids in [
        vec![],
        vec!["bad"],
        vec!["0"],
        vec!["0456"],
        vec!["-123", "999"],
        vec!["456"; 101],
    ] {
        let request = EventRequest {
            chat_ids: ids.into_iter().map(str::to_string).collect(),
            event: serde_json::json!("test"),
        };
        assert_eq!(
            receive(State(context.clone()), Json(request)).await,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        receive(
            State(context.clone()),
            Json(EventRequest {
                chat_ids: vec!["456".into()],
                event: serde_json::Value::Null
            })
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert!(llm.requests.lock().unwrap().is_empty());
    let lock = Arc::new(Mutex::new(()));
    context
        .chat_locks
        .lock()
        .unwrap()
        .insert("456".into(), lock.clone());
    let guard = lock.lock().await;
    for _ in 0..2 {
        assert_eq!(
            receive(
                State(context.clone()),
                Json(EventRequest {
                    chat_ids: vec!["456".into()],
                    event: serde_json::json!("test")
                })
            )
            .await,
            StatusCode::ACCEPTED
        );
    }
    assert!(llm.requests.lock().unwrap().is_empty());
    drop(guard);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let bot = context.bot.lock().await;
            if bot
                .state
                .lock()
                .await
                .db
                .load_messages("456", 20, None)
                .unwrap()
                .len()
                >= 6
            {
                break;
            }
            drop(bot);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn failures_and_stops_preserve_event_history_without_retry() {
    let (context, _dir, llm) = setup(true).await;
    let bot = context.bot.lock().await;
    let signals = bot.stop_signals.clone();
    assert!(crate::bot::run_event(
        bot.state.clone(),
        signals.clone(),
        "-123",
        &serde_json::json!("failure"),
        &context.telegram
    )
    .await
    .is_err());
    signals.lock().unwrap().insert(
        "-123".into(),
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );
    crate::bot::run_event(
        bot.state.clone(),
        signals,
        "-123",
        &serde_json::json!("stopped"),
        &context.telegram,
    )
    .await
    .unwrap();
    assert_eq!(llm.requests.lock().unwrap().len(), 1);
    let state = bot.state.lock().await;
    assert!(
        serde_json::to_string(&state.db.load_messages("-123", 20, None).unwrap())
            .unwrap()
            .contains("stopped")
    );
}

#[tokio::test]
async fn http_route_accepts_json_and_rejects_malformed_body() {
    let (context, _dir, _) = setup(false).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(context)).await.unwrap();
    });
    let client = reqwest::Client::new();
    let url = format!("http://{address}/events");
    assert_eq!(
        client
            .post(&url)
            .json(&serde_json::json!({"chat_ids":["-123"],"event":"test"}))
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    assert_eq!(
        client
            .post(&url)
            .header("content-type", "application/json")
            .body("broken")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(client.get(&url).send().await.unwrap().status(), 405);
    server.abort();
}

#[tokio::test]
async fn event_respects_history_window_model_and_mcp_restrictions() {
    let (context, _dir, llm) = setup(false).await;
    let bot = context.bot.lock().await;
    {
        let mut state = bot.state.lock().await;
        state
            .config
            .conversation
            .heartbeat_recent_messages_window_size = Some(0);
        state
            .model_overrides
            .insert("-123".into(), "custom-model".into());
        state.config.chats.get_mut("-123").unwrap().mcp_blacklist = vec!["blocked".into()];
        for server in ["allowed", "blocked"] {
            state.mcp_tools.push(crate::mcp::McpToolInfo {
                server_name: server.into(),
                name: "curl".into(),
                description: "HTTP requests".into(),
                input_schema: serde_json::json!({"type":"object"}),
            });
        }
    }
    crate::bot::run_event(
        bot.state.clone(),
        bot.stop_signals.clone(),
        "-123",
        &serde_json::json!("event"),
        &context.telegram,
    )
    .await
    .unwrap();
    let requests = llm.requests.lock().unwrap();
    assert_eq!(requests[0].model, "custom-model");
    assert!(!serde_json::to_string(&requests[0].messages)
        .unwrap()
        .contains("Previous archive instructions"));
    let tools = requests[0].tools.as_ref().unwrap();
    assert!(tools.iter().any(|t| t.function.name == "mcp_allowed_curl"));
    assert!(!tools.iter().any(|t| t.function.name == "mcp_blocked_curl"));
}

#[tokio::test]
async fn accepted_event_failure_is_saved_without_retry_and_clears_old_stop() {
    let (context, _dir, llm) = setup(true).await;
    {
        let bot = context.bot.lock().await;
        bot.stop_signals.lock().unwrap().insert(
            "456".into(),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
    }
    assert_eq!(
        receive(
            State(context.clone()),
            Json(EventRequest {
                chat_ids: vec!["456".into()],
                event: serde_json::json!("failure"),
            })
        )
        .await,
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let bot = context.bot.lock().await;
            if !bot
                .state
                .lock()
                .await
                .db
                .load_messages("456", 20, None)
                .unwrap()
                .is_empty()
            {
                break;
            }
            drop(bot);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(llm.requests.lock().unwrap().len(), 1);
}
