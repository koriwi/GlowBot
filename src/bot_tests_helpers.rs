use super::bot_dispatch::{cap_tool_result, dispatch_tool, dispatch_tool_calls, log_tool_call_to};
use super::bot_heartbeat::{background_task_prompt, run_heartbeat_task};
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

