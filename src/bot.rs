use crate::bash;
use crate::commands::{can_interact, can_run_command, handle_command, parse_command};
use crate::config::Config;
use crate::git::GitRepo;
use crate::llm::LlmBackend;
use crate::memory::{load_chat_memories, save_memory, Memory};
use crate::openrouter::{ChatCompletionRequest, ChatMessage};
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
    /// Per-chat conversation history (sliding window of recent messages).
    pub conversation_history: HashMap<String, Vec<ChatMessage>>,
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
            conversation_history: HashMap::new(),
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
            let chat_memory = crate::memory::load_chat_memory(&state.chats_dir(), chat_id);
            let chat_config = state.config.chat_config(chat_id);
            let system_prompt = system_prompt::assemble(
                chat_id,
                &chat_config.system_prompt,
                skills,
                chat_memory.as_ref(),
                &memories,
            );
            let model = state.config.model_for_chat(chat_id).to_string();
            (system_prompt, model)
        };

        // Ensure user has a memory file
        self.ensure_memory_exists(chat_id, user_id, username)
            .await?;

        // Build messages: system prompt + history + current message
        let history = {
            let state = self.state.lock().await;
            state
                .conversation_history
                .get(chat_id)
                .cloned()
                .unwrap_or_default()
        };

        let current_msg = ChatMessage::user_with_name(text, username);
        let mut messages = vec![ChatMessage::system(&system_prompt)];
        messages.extend(history);
        messages.push(current_msg.clone());

        let tools = crate::openrouter::all_tool_definitions();
        let max_tool_rounds = 10;

        // Run the LLM tool-use loop, capturing the final response.
        let result = {
            let mut result = None;
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
                    None => break,
                };

                // Check for tool calls
                if let Some(tool_calls) = &choice.message.tool_calls {
                    if tool_calls.is_empty() {
                        result = Some(choice.message.content.clone().unwrap_or_default());
                        break;
                    }

                    // Add the assistant message with tool calls
                    messages.push(ChatMessage::assistant_tool_calls(tool_calls.clone()));

                    // Execute each tool call
                    for tool_call in tool_calls {
                        let args: serde_json::Value =
                            serde_json::from_str(&tool_call.function.arguments).unwrap_or_default();

                        let result_text = match tool_call.function.name.as_str() {
                            "bash" => self.execute_bash_tool(&args).await,
                            "read_memory" => self.execute_read_memory(chat_id, &args).await,
                            "update_memory" => self.execute_update_memory(chat_id, &args).await,
                            "read_chat_memory" => self.execute_read_chat_memory(chat_id).await,
                            "update_chat_memory" => {
                                self.execute_update_chat_memory(chat_id, &args).await
                            }
                            "create_skill" => self.execute_create_skill(&args).await,
                            "update_skill" => self.execute_update_skill(&args).await,
                            _ => format!("Unknown tool: {}", tool_call.function.name),
                        };

                        // Log the tool call
                        self.log_tool_call(
                            &tool_call.function.name,
                            &tool_call.function.arguments,
                            &result_text,
                        )
                        .await;

                        messages.push(ChatMessage::tool_result(&tool_call.id, &result_text));
                    }

                    // Auto-commit after tool execution (tools may have modified files)
                    self.git_repo
                        .auto_commit("Auto-commit after tool execution")?;
                    continue;
                }

                // No tool calls — final response
                result = Some(choice.message.content.clone().unwrap_or_default());
                break;
            }

            // If we exhausted rounds, give a loop error
            result.unwrap_or_else(|| {
                "I ran into a loop processing your request. Please try again.".into()
            })
        };

        // Store in conversation history
        {
            let mut state = self.state.lock().await;
            let window = state.config.conversation_window;
            let history = state
                .conversation_history
                .entry(chat_id.to_string())
                .or_default();
            history.push(current_msg);
            history.push(ChatMessage::assistant(&result));
            while history.len() > window {
                history.remove(0);
            }
        }

        Ok(Some(result))
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

    /// Execute a bash tool call.
    async fn execute_bash_tool(&self, args: &serde_json::Value) -> String {
        let command = args["command"].as_str().unwrap_or("");
        let data_dir = {
            let state = self.state.lock().await;
            state.data_dir.clone()
        };
        match bash::execute_in_dir(command, &data_dir).await {
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
        }
    }

    /// Execute a read_memory tool call. Returns memory as JSON.
    async fn execute_read_memory(&self, chat_id: &str, args: &serde_json::Value) -> String {
        let user_id = args["user_id"].as_str().unwrap_or("");
        let state = self.state.lock().await;
        match crate::memory::load_memory(&state.chats_dir(), chat_id, user_id) {
            Some(mem) => serde_json::json!({
                "user_id": mem.frontmatter.user_id,
                "username": mem.frontmatter.username,
                "call_name": mem.frontmatter.call_name,
                "description": mem.frontmatter.description,
                "body": mem.body,
            })
            .to_string(),
            None => format!(
                "No memory file found for user_id={} in chat {}",
                user_id, chat_id
            ),
        }
    }

    /// Execute an update_memory tool call. Only overwrites provided fields.
    async fn execute_update_memory(&self, chat_id: &str, args: &serde_json::Value) -> String {
        let user_id = args["user_id"].as_str().unwrap_or("");
        if user_id.is_empty() {
            return "Error: user_id is required".into();
        }

        let state = self.state.lock().await;
        let chats_dir = state.chats_dir();

        let mut mem = crate::memory::load_memory(&chats_dir, chat_id, user_id)
            .unwrap_or_else(|| Memory::new(user_id, ""));

        let mut changed = false;

        if let Some(v) = args["username"].as_str() {
            mem.frontmatter.username = v.to_string();
            changed = true;
        }
        if let Some(v) = args["call_name"].as_str() {
            mem.frontmatter.call_name = v.to_string();
            changed = true;
        }
        if let Some(v) = args["description"].as_str() {
            mem.frontmatter.description = v.to_string();
            changed = true;
        }
        if let Some(v) = args["log_entry"].as_str() {
            mem.append_log(v);
            changed = true;
        }

        if changed {
            match crate::memory::save_memory(&chats_dir, chat_id, user_id, &mem) {
                Ok(()) => format!(
                    "Memory updated for user_id={}. Current state: {}",
                    user_id,
                    serde_json::json!({
                        "user_id": mem.frontmatter.user_id,
                        "username": mem.frontmatter.username,
                        "call_name": mem.frontmatter.call_name,
                        "description": mem.frontmatter.description,
                    })
                ),
                Err(e) => format!("Failed to save memory: {}", e),
            }
        } else {
            "No fields to update. Provide at least one of: username, call_name, description, log_entry.".into()
        }
    }

    /// Execute a read_chat_memory tool call.
    async fn execute_read_chat_memory(&self, chat_id: &str) -> String {
        let state = self.state.lock().await;
        match crate::memory::load_chat_memory(&state.chats_dir(), chat_id) {
            Some(mem) => serde_json::json!({
                "call_name": mem.frontmatter.call_name,
                "description": mem.frontmatter.description,
                "body": mem.body,
            })
            .to_string(),
            None => format!("No chat memory yet for chat {}.", chat_id),
        }
    }

    /// Execute an update_chat_memory tool call.
    async fn execute_update_chat_memory(&self, chat_id: &str, args: &serde_json::Value) -> String {
        let state = self.state.lock().await;
        let chats_dir = state.chats_dir();

        let mut mem =
            crate::memory::load_chat_memory(&chats_dir, chat_id).unwrap_or_else(Memory::new_chat);

        let mut changed = false;

        if let Some(v) = args["call_name"].as_str() {
            mem.frontmatter.call_name = v.to_string();
            changed = true;
        }
        if let Some(v) = args["description"].as_str() {
            mem.frontmatter.description = v.to_string();
            changed = true;
        }
        if let Some(v) = args["log_entry"].as_str() {
            mem.append_log(v);
            changed = true;
        }

        if changed {
            match crate::memory::save_chat_memory(&chats_dir, chat_id, &mem) {
                Ok(()) => format!(
                    "Chat memory updated. Current state: {}",
                    serde_json::json!({
                        "call_name": mem.frontmatter.call_name,
                        "description": mem.frontmatter.description,
                    })
                ),
                Err(e) => format!("Failed to save chat memory: {}", e),
            }
        } else {
            "No fields to update. Provide at least one of: call_name, description, log_entry."
                .into()
        }
    }

    /// Execute a create_skill tool call.
    async fn execute_create_skill(&self, args: &serde_json::Value) -> String {
        let name = args["name"].as_str().unwrap_or("");
        let description = args["description"].as_str().unwrap_or("");
        let body = args["body"].as_str().unwrap_or("");

        if name.is_empty() || description.is_empty() || body.is_empty() {
            return "Error: name, description, and body are all required.".into();
        }

        let state = self.state.lock().await;
        let fm = crate::skills::SkillFrontmatter {
            name: name.to_string(),
            description: description.to_string(),
        };
        match crate::skills::write_skill(&state.skills_dir(), name, &fm, body) {
            Ok(_path) => {
                drop(state);
                // Reload skills so the new one is available immediately
                if let Err(e) = self.reload_skills().await {
                    log::error!("Failed to reload skills after create: {}", e);
                }
                format!("Skill '{}' created successfully.", name)
            }
            Err(e) => format!("Failed to create skill: {}", e),
        }
    }

    /// Execute an update_skill tool call.
    async fn execute_update_skill(&self, args: &serde_json::Value) -> String {
        let name = args["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return "Error: name is required.".into();
        }

        let state = self.state.lock().await;
        let skill_path = state.skills_dir().join(name).join("skill.md");

        let mut skill = match crate::skills::load_skill(&skill_path) {
            Ok(s) => s,
            Err(_) => return format!("Skill '{}' not found at {}.", name, skill_path.display()),
        };

        let mut changed = false;
        if let Some(v) = args["description"].as_str() {
            skill.frontmatter.description = v.to_string();
            changed = true;
        }
        let body = args["body"].as_str();
        if let Some(v) = body {
            skill.body = v.to_string();
            changed = true;
        }

        if !changed {
            return "No fields to update. Provide at least one of: description, body.".into();
        }

        // Reconstruct the file content and write
        let yaml = serde_yaml::to_string(&skill.frontmatter).unwrap_or_default();
        let content = format!("---\n{}---\n{}", yaml, skill.body);
        match std::fs::write(&skill_path, &content) {
            Ok(()) => {
                drop(state);
                if let Err(e) = self.reload_skills().await {
                    log::error!("Failed to reload skills after update: {}", e);
                }
                format!("Skill '{}' updated successfully.", name)
            }
            Err(e) => format!("Failed to update skill: {}", e),
        }
    }

    /// Log a tool call to the tool_calls.log file in the data directory.
    async fn log_tool_call(&self, tool_name: &str, args: &str, result: &str) {
        let state = self.state.lock().await;
        let log_path = state.data_dir.join("tool_calls.log");
        drop(state);

        let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let result_summary: String = result.chars().take(200).collect();
        let args_summary: String = args.chars().take(200).collect();
        let line = format!(
            "[{}] {} | args: {} | result: {}\n",
            timestamp, tool_name, args_summary, result_summary
        );

        // Best-effort append — don't block on log write errors
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(line.as_bytes())
            });

        log::info!("tool {}: {}", tool_name, args_summary);
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
            conversation_window: 20,
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

    #[tokio::test]
    async fn test_execute_read_memory_existing() {
        let (bot, _dir, _mock) = setup_test_bot().await;

        // First ensure a memory file exists
        bot.ensure_memory_exists("-123", "456", "@testuser")
            .await
            .unwrap();

        let result = bot
            .execute_read_memory("-123", &serde_json::json!({"user_id": "456"}))
            .await;
        assert!(result.contains("456"));
        assert!(result.contains("@testuser"));
    }

    #[tokio::test]
    async fn test_execute_read_memory_nonexistent() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        let result = bot
            .execute_read_memory("-123", &serde_json::json!({"user_id": "nonexistent"}))
            .await;
        assert!(result.contains("No memory file found"));
    }

    #[tokio::test]
    async fn test_execute_update_memory_new() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        let result = bot
            .execute_update_memory(
                "-123",
                &serde_json::json!({
                    "user_id": "789",
                    "call_name": "TestUser",
                    "description": "A test user",
                    "log_entry": "first interaction"
                }),
            )
            .await;
        assert!(result.contains("Memory updated"));
        assert!(result.contains("TestUser"));

        // Verify the file was written
        let state = bot.state.lock().await;
        let mem = crate::memory::load_memory(&state.chats_dir(), "-123", "789").unwrap();
        assert_eq!(mem.frontmatter.call_name, "TestUser");
        assert_eq!(mem.frontmatter.description, "A test user");
        assert!(mem.body.contains("first interaction"));
    }

    #[tokio::test]
    async fn test_execute_update_memory_partial() {
        let (bot, _dir, _mock) = setup_test_bot().await;

        // Create initial memory
        bot.execute_update_memory(
            "-123",
            &serde_json::json!({
                "user_id": "111",
                "call_name": "Original",
                "description": "Original desc"
            }),
        )
        .await;

        // Partial update — only change call_name
        let result = bot
            .execute_update_memory(
                "-123",
                &serde_json::json!({
                    "user_id": "111",
                    "call_name": "Updated"
                }),
            )
            .await;
        assert!(result.contains("Updated"));

        let state = bot.state.lock().await;
        let mem = crate::memory::load_memory(&state.chats_dir(), "-123", "111").unwrap();
        assert_eq!(mem.frontmatter.call_name, "Updated");
        assert_eq!(mem.frontmatter.description, "Original desc"); // unchanged
    }

    #[tokio::test]
    async fn test_execute_update_memory_no_fields() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        let result = bot
            .execute_update_memory("-123", &serde_json::json!({"user_id": "999"}))
            .await;
        assert!(result.contains("No fields to update"));
    }

    #[tokio::test]
    async fn test_execute_update_memory_empty_user_id() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        let result = bot
            .execute_update_memory("-123", &serde_json::json!({"user_id": ""}))
            .await;
        assert!(result.contains("Error"));
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
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        // Then LLM responds after reading memory
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("I remember you, @testuser!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
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
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        // LLM confirms
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("I've noted that!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
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
    async fn test_execute_read_chat_memory() {
        let (bot, _dir, _mock) = setup_test_bot().await;

        // Save some chat memory first
        let state = bot.state.lock().await;
        let mut mem = Memory::new_chat();
        mem.frontmatter.call_name = "Test Group".into();
        mem.frontmatter.description = "A test group".into();
        crate::memory::save_chat_memory(&state.chats_dir(), "-123", &mem).unwrap();
        drop(state);

        let result = bot.execute_read_chat_memory("-123").await;
        assert!(result.contains("Test Group"));
        assert!(result.contains("A test group"));
    }

    #[tokio::test]
    async fn test_execute_read_chat_memory_nonexistent() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        let result = bot.execute_read_chat_memory("-none").await;
        assert!(result.contains("No chat memory"));
    }

    #[tokio::test]
    async fn test_execute_update_chat_memory() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        let result = bot
            .execute_update_chat_memory(
                "-123",
                &serde_json::json!({
                    "call_name": "My Group",
                    "description": "We talk about Rust",
                    "log_entry": "first interaction"
                }),
            )
            .await;
        assert!(result.contains("Chat memory updated"));
        assert!(result.contains("My Group"));

        let state = bot.state.lock().await;
        let mem = crate::memory::load_chat_memory(&state.chats_dir(), "-123").unwrap();
        assert_eq!(mem.frontmatter.call_name, "My Group");
        assert!(mem.body.contains("first interaction"));
    }

    #[tokio::test]
    async fn test_execute_update_chat_memory_partial() {
        let (bot, _dir, _mock) = setup_test_bot().await;

        // First update
        bot.execute_update_chat_memory(
            "-123",
            &serde_json::json!({"call_name": "Original", "description": "Original desc"}),
        )
        .await;

        // Partial update — only description
        bot.execute_update_chat_memory("-123", &serde_json::json!({"description": "New desc"}))
            .await;

        let state = bot.state.lock().await;
        let mem = crate::memory::load_chat_memory(&state.chats_dir(), "-123").unwrap();
        assert_eq!(mem.frontmatter.call_name, "Original"); // unchanged
        assert_eq!(mem.frontmatter.description, "New desc");
    }

    #[tokio::test]
    async fn test_execute_create_skill() {
        let (bot, dir, _mock) = setup_test_bot().await;
        let result = bot
            .execute_create_skill(&serde_json::json!({
                "name": "my-skill",
                "description": "Does things",
                "body": "Run: echo hello"
            }))
            .await;
        assert!(result.contains("created successfully"));

        // Verify file exists
        let skills_dir = dir.path().join("glowbot_data").join("skills");
        let skill_file = skills_dir.join("my-skill").join("skill.md");
        assert!(skill_file.exists());

        // Verify skill was loaded
        let state = bot.state.lock().await;
        assert!(state.skills.contains_key("my-skill"));
        assert_eq!(
            state.skills["my-skill"].frontmatter.description,
            "Does things"
        );
    }

    #[tokio::test]
    async fn test_execute_create_skill_missing_fields() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        let result = bot
            .execute_create_skill(&serde_json::json!({"name": "test"}))
            .await;
        assert!(result.contains("Error"));
    }

    #[tokio::test]
    async fn test_execute_update_skill() {
        let (bot, _dir, _mock) = setup_test_bot().await;

        // Create a skill first
        bot.execute_create_skill(&serde_json::json!({
            "name": "updatable",
            "description": "Original",
            "body": "Original body"
        }))
        .await;

        // Update it
        let result = bot
            .execute_update_skill(&serde_json::json!({
                "name": "updatable",
                "description": "Updated desc"
            }))
            .await;
        assert!(result.contains("updated successfully"));

        let state = bot.state.lock().await;
        let skill = &state.skills["updatable"];
        assert_eq!(skill.frontmatter.description, "Updated desc");
        assert_eq!(skill.body, "Original body"); // unchanged
    }

    #[tokio::test]
    async fn test_execute_update_skill_not_found() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        let result = bot
            .execute_update_skill(&serde_json::json!({"name": "nonexistent"}))
            .await;
        assert!(result.contains("not found"));
    }

    #[tokio::test]
    async fn test_log_tool_call() {
        let (bot, dir, _mock) = setup_test_bot().await;
        bot.log_tool_call("bash", r#"{"command":"echo hi"}"#, "stdout: hi\n")
            .await;

        let log_path = dir.path().join("glowbot_data").join("tool_calls.log");
        assert!(log_path.exists());
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("bash"));
        assert!(content.contains("echo hi"));
    }
}
