use crate::bash;
use crate::commands::{can_interact, can_run_command, handle_command, parse_command};
use crate::config::Config;
use crate::git::GitRepo;
use crate::llm::LlmBackend;
use crate::memory::{load_chat_memories, save_memory, Memory};
use crate::openrouter::{bash_tool_definition, ChatCompletionRequest, ChatMessage};
use crate::skills::{load_all_skills, Skill};
use crate::system_prompt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Shared bot state accessible from all handlers.
pub struct BotState {
    pub config: Config,
    pub skills: HashMap<String, Skill>,
    pub llm: Arc<dyn LlmBackend>,
    pub data_dir: std::path::PathBuf,
}

impl BotState {
    /// Get the path to the chats directory.
    pub fn chats_dir(&self) -> std::path::PathBuf {
        self.data_dir.join("chats")
    }

    /// Get the path to the skills directory.
    pub fn skills_dir(&self) -> std::path::PathBuf {
        self.data_dir.join("skills")
    }

    /// Get the path to the config file.
    pub fn config_path(&self) -> std::path::PathBuf {
        self.data_dir.join("config.yaml")
    }
}

/// Main GlowBot orchestrator.
pub struct GlowBot {
    pub state: Arc<Mutex<BotState>>,
    pub git_repo: GitRepo,
}

impl GlowBot {
    /// Create a new GlowBot instance with the given LLM backend.
    pub async fn new_with_llm(data_dir: &Path, llm: Arc<dyn LlmBackend>) -> anyhow::Result<Self> {
        let config_path = data_dir.join("config.yaml");
        let config = Config::load(&config_path)?;
        let skills_dir = data_dir.join("skills");
        let skills = load_all_skills(&skills_dir)?;
        let git_repo = GitRepo::new(data_dir);

        // Initialize git if needed
        if !git_repo.is_repo() {
            git_repo.init()?;
        }

        let state = BotState {
            config,
            skills,
            llm,
            data_dir: data_dir.to_path_buf(),
        };

        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            git_repo,
        })
    }

    /// Reload skills from disk.
    pub async fn reload_skills(&self) -> anyhow::Result<()> {
        let mut state = self.state.lock().await;
        let skills_dir = state.skills_dir();
        state.skills = load_all_skills(&skills_dir)?;
        Ok(())
    }

    /// Save config to disk and auto-commit.
    pub async fn save_config(&self) -> anyhow::Result<()> {
        let state = self.state.lock().await;
        let path = state.config_path();
        state.config.save(&path)?;
        drop(state);
        self.git_repo
            .auto_commit("Update configuration via /command")?;
        Ok(())
    }

    /// Process an incoming message and return the response.
    /// Returns None if no response should be sent.
    pub async fn process_message(
        &self,
        chat_id: &str,
        user_id: &str,
        username: &str,
        text: &str,
        bot_username: &str,
    ) -> anyhow::Result<Option<String>> {
        let is_command = text.trim().starts_with('/');
        let is_mention = text.contains(&format!("@{}", bot_username));

        // Check if it's a bot command
        if let Some(command) = parse_command(text) {
            return self.handle_bot_command(&command, chat_id, user_id).await;
        }

        // Check interaction permissions
        let chat_config = {
            let state = self.state.lock().await;
            state.config.chat_config(chat_id)
        };

        if !can_interact(&chat_config, user_id) {
            return Ok(None); // User not allowed to interact
        }

        // In mention_only mode, only respond to mentions (groups only; DMs always respond)
        let is_dm = !chat_id.starts_with('-');
        if !is_dm
            && matches!(
                chat_config.interaction_mode,
                crate::config::InteractionMode::MentionOnly
            )
            && !is_command
            && !is_mention
        {
            return Ok(None);
        }

        // If it's a plain command (not a bot command), ignore
        if is_command && !is_mention {
            return Ok(None);
        }

        // Process with LLM
        self.process_with_llm(chat_id, user_id, username, text)
            .await
    }

    /// Handle a bot command (/model, /mode, /reload, /status).
    async fn handle_bot_command(
        &self,
        command: &crate::commands::Command,
        chat_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<String>> {
        // Check command permissions
        let allowed = {
            let state = self.state.lock().await;
            let chat_config = state.config.chat_config(chat_id);
            can_run_command(&chat_config, user_id)
        };

        if !allowed {
            return Ok(Some("You are not authorized to run bot commands.".into()));
        }

        let needs_save = matches!(
            command,
            crate::commands::Command::Model(_) | crate::commands::Command::Mode(_)
        );
        let reload_needed = matches!(command, crate::commands::Command::Reload);

        let response = {
            let mut state = self.state.lock().await;
            handle_command(command, &mut state.config, chat_id)
        };

        if needs_save {
            self.save_config().await?;
        }

        if reload_needed {
            self.reload_skills().await?;
        }

        Ok(Some(response))
    }

    /// Process a message through the LLM pipeline.
    async fn process_with_llm(
        &self,
        chat_id: &str,
        user_id: &str,
        username: &str,
        text: &str,
    ) -> anyhow::Result<Option<String>> {
        let (system_prompt, model) = {
            let state = self.state.lock().await;
            let skills = &state.skills;
            let memories = load_chat_memories(&state.chats_dir(), chat_id).unwrap_or_default();
            let chat_config = state.config.chat_config(chat_id);
            let system_prompt =
                system_prompt::assemble(chat_id, &chat_config.system_prompt, skills, &memories);
            let model = state.config.model_for_chat(chat_id).to_string();
            (system_prompt, model)
        };

        // Ensure user has a memory file
        self.ensure_memory_exists(chat_id, user_id, username)
            .await?;

        // Build messages
        let mut messages = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user_with_name(text, username),
        ];

        let tools = vec![bash_tool_definition()];
        let max_tool_rounds = 10;

        for _round in 0..max_tool_rounds {
            let request = ChatCompletionRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: Some(tools.clone()),
                tool_choice: None,
            };

            let response = {
                let state = self.state.lock().await;
                state.llm.chat_completion(&request).await?
            };

            let choice = match response.choices.into_iter().next() {
                Some(c) => c,
                None => return Ok(None),
            };

            // Check for tool calls
            if let Some(tool_calls) = &choice.message.tool_calls {
                if tool_calls.is_empty() {
                    let content = choice.message.content.clone().unwrap_or_default();
                    return Ok(Some(content));
                }

                // Add the assistant message with tool calls
                messages.push(ChatMessage::assistant_tool_calls(tool_calls.clone()));

                // Execute each tool call
                for tool_call in tool_calls {
                    if tool_call.function.name == "bash" {
                        let args: serde_json::Value =
                            serde_json::from_str(&tool_call.function.arguments).unwrap_or_default();
                        let command = args["command"].as_str().unwrap_or("");
                        let data_dir = {
                            let state = self.state.lock().await;
                            state.data_dir.clone()
                        };
                        let result = bash::execute_in_dir(command, &data_dir).await;
                        let result_text = match result {
                            Ok(r) => {
                                let mut out = String::new();
                                if !r.stdout.is_empty() {
                                    out.push_str(&format!("stdout:\n{}", r.stdout));
                                }
                                if !r.stderr.is_empty() {
                                    out.push_str(&format!("stderr:\n{}", r.stderr));
                                }
                                if r.stdout.is_empty() && r.stderr.is_empty() {
                                    out.push_str(&format!("exit code: {}", r.exit_code));
                                }
                                out
                            }
                            Err(e) => format!("Error executing command: {}", e),
                        };
                        messages.push(ChatMessage::tool_result(&tool_call.id, &result_text));
                    }
                }

                // Auto-commit after tool execution (tools may have modified files)
                self.git_repo
                    .auto_commit("Auto-commit after tool execution")?;
                continue;
            }

            // No tool calls — return content
            let content = choice.message.content.clone().unwrap_or_default();
            return Ok(Some(content));
        }

        Ok(Some(
            "I ran into a loop processing your request. Please try again.".into(),
        ))
    }

    /// Ensure a memory file exists for the given user in the given chat.
    async fn ensure_memory_exists(
        &self,
        chat_id: &str,
        user_id: &str,
        username: &str,
    ) -> anyhow::Result<()> {
        let state = self.state.lock().await;
        let existing = crate::memory::load_memory(&state.chats_dir(), chat_id, user_id);
        if existing.is_none() {
            let mem = Memory::new(user_id, username);
            save_memory(&state.chats_dir(), chat_id, user_id, &mem)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::MockLlmBackend;
    use crate::openrouter::{
        AssistantMessage, ChatCompletionResponse, Choice, FunctionCall, ToolCall,
    };
    use tempfile::TempDir;

    fn test_config() -> Config {
        Config {
            telegram_token: "test-token".into(),
            openrouter_api_key: "test-key".into(),
            openrouter_default_model: "test/model".into(),
            chats: HashMap::new(),
        }
    }

    async fn setup_test_bot() -> (GlowBot, TempDir, Arc<MockLlmBackend>) {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let config = test_config();
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

        let mut config = test_config();
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
        test_config().save(&data_dir.join("config.yaml")).unwrap();

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
                },
                finish_reason: Some("stop".into()),
            }],
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
                },
                finish_reason: Some("stop".into()),
            }],
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
        // User "456" is in the command whitelist
        let result = bot
            .process_message("-123", "456", "@testuser", "/status", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("Chat ID:"));
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
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        // Second response: final text after tool result
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("The bash command succeeded!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
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

        mock.add_response(ChatCompletionResponse { choices: vec![] });

        let result = bot
            .process_message("-123", "456", "@testuser", "Hello", "mybot")
            .await
            .unwrap();
        assert!(result.is_none());
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
                },
                finish_reason: Some("stop".into()),
            }],
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
        for _ in 0..10 {
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
                    },
                    finish_reason: Some("tool_calls".into()),
                }],
            });
        }

        let result = bot
            .process_message("-123", "456", "@testuser", "Loop test", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("loop"));
    }

    #[tokio::test]
    async fn test_process_message_command_model() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        let result = bot
            .process_message("-123", "456", "@testuser", "/model custom/model", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("custom/model"));
    }

    #[tokio::test]
    async fn test_process_message_command_mode() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        let result = bot
            .process_message("-123", "456", "@testuser", "/mode every_message", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("EveryMessage"));
    }

    #[tokio::test]
    async fn test_process_message_command_reload() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        let result = bot
            .process_message("-123", "456", "@testuser", "/reload", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("Skills reloaded successfully.".into()));
    }

    #[tokio::test]
    async fn test_process_message_command_invalid_mode() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        let result = bot
            .process_message("-123", "456", "@testuser", "/mode invalid", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("Unknown mode"));
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
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("Command failed, but I handled it.".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
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

        let mut config = test_config();
        config.chats.insert(
            "-123".into(),
            crate::config::ChatConfig {
                interaction_mode: crate::config::InteractionMode::EveryMessage,
                interaction_whitelist: vec![],
                command_whitelist: vec![],
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
                },
                finish_reason: Some("stop".into()),
            }],
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
        test_config().save(&data_dir.join("config.yaml")).unwrap();

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
        let (bot, _dir, mock) = setup_test_bot().await;

        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("DM response!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        // Positive chat ID = DM, default mention_only mode
        let result = bot
            .process_message("123456789", "456", "@testuser", "Hello in DM", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("DM response!".into()));
    }
}
