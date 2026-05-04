use crate::commands::{can_interact, can_run_command, handle_command, parse_command};
use crate::config::Config;
use crate::db::Database;
use crate::git::GitRepo;
use crate::llm::LlmBackend;
use crate::memory::{save_memory, Memory};
use crate::openrouter::{ChatCompletionRequest, ChatMessage, Usage};
#[path = "bot_dispatch.rs"]
mod bot_dispatch;
#[path = "bot_heartbeat.rs"]
mod bot_heartbeat;
use self::bot_dispatch::dispatch_tool_calls;
pub use self::bot_heartbeat::run_heartbeat_task;
use crate::skills::{load_all_skills, Skill};
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
    /// SQLite-backed conversation history (one row per message).
    pub db: Database,
    /// Tools discovered from MCP servers.
    pub mcp_tools: Vec<crate::mcp::McpTool>,
    /// Cached model context lengths from OpenRouter.
    pub model_context_lengths: HashMap<String, u64>,
    /// Per-chat last token usage from the most recent LLM call.
    pub last_usage: HashMap<String, Usage>,
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

    /// Assemble the full system prompt for a chat, loading memories and skills.
    pub fn assemble_system_prompt(
        &self,
        chat_id: &str,
        tools_enabled: bool,
        user_id: &str,
    ) -> String {
        let skills = &self.skills;
        let memories =
            crate::memory::load_chat_memories(&self.chats_dir(), chat_id).unwrap_or_default();
        let chat_memory = crate::memory::load_chat_memory(&self.chats_dir(), chat_id);
        let chat_config = self.config.chat_config(chat_id);
        let system_prompt = if !chat_id.starts_with('-') {
            self.config
                .dm_config(chat_id)
                .map(|d| d.system_prompt.as_str())
                .unwrap_or(&chat_config.system_prompt)
        } else {
            &chat_config.system_prompt
        };
        crate::system_prompt::assemble(
            chat_id,
            system_prompt,
            skills,
            chat_memory.as_ref(),
            &memories,
            tools_enabled,
            user_id,
        )
    }

    /// Get the effective model for a chat.
    pub fn effective_model(&self, chat_id: &str) -> String {
        self.config.model_for_chat(chat_id).to_string()
    }

    /// Build the full list of tool definitions including MCP tools.
    /// `send_message` is always included — in normal conversations it's for
    /// headsup/intermediate messages; in heartbeat tasks it's for completion reports.
    pub fn build_tools(&self, include_bash: bool) -> Vec<crate::openrouter::ToolDefinition> {
        let mut t = crate::openrouter::all_tool_definitions(
            include_bash,
            self.config.embedding_model.as_deref(),
        );
        for mt in &self.mcp_tools {
            t.push(crate::openrouter::ToolDefinition {
                def_type: "function".into(),
                function: crate::openrouter::FunctionDef {
                    name: format!("mcp_{}_{}", mt.server_name, mt.name),
                    description: format!("[MCP: {}] {}", mt.server_name, mt.description),
                    parameters: mt.input_schema.clone(),
                },
            });
        }
        t
    }

    /// Check if a chat has pending tasks.
    pub fn has_pending_tasks(&self, chat_id: &str) -> bool {
        let list = crate::tasks::TaskList::load(&self.chats_dir(), chat_id).unwrap_or_default();
        list.has_tasks()
    }

    /// Get the formatted context usage string for a chat, e.g. "37k/189k (15%)"
    /// Reports against the *effective* limit (with safety margin applied).
    pub fn context_usage(&self, chat_id: &str) -> String {
        let model = self.effective_model(chat_id);
        let raw_limit = self.model_context_lengths.get(&model).copied().unwrap_or(0);
        let effective_limit = if raw_limit == 0 {
            0
        } else {
            (raw_limit as f64 * crate::openrouter::TOKEN_ESTIMATE_MARGIN) as u64
        };
        let usage = self.last_usage.get(chat_id).cloned().unwrap_or_default();
        crate::openrouter::format_context_usage(usage.prompt_tokens, effective_limit)
    }
}

/// Main GlowBot orchestrator.
pub struct GlowBot {
    pub state: Arc<Mutex<BotState>>,
    pub git_repo: GitRepo,
    /// Per-chat stop signals. When set, ongoing LLM processing for that chat should abort.
    pub stop_signals: Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
}

impl GlowBot {
    // Methods below are tested directly; dispatch_tool is the canonical path.
    #[allow(dead_code)]
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

        // Discover MCP server tools
        let mcp_tools = crate::mcp::discover_all(&config.mcp_servers).await?;
        if !mcp_tools.is_empty() {
            log::info!(
                "Loaded {} MCP tools from {} server(s)",
                mcp_tools.len(),
                config.mcp_servers.len()
            );
        }

        let state = BotState {
            config,
            skills,
            llm,
            data_dir: data_dir.to_path_buf(),
            db: Database::new(&data_dir.join("conversations.db"))?,
            mcp_tools,
            model_context_lengths: HashMap::new(),
            last_usage: HashMap::new(),
        };

        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            git_repo,
            stop_signals: Arc::new(std::sync::Mutex::new(HashMap::new())),
        })
    }

    /// Fetch model context lengths from OpenRouter and populate the cache.
    pub async fn fetch_model_contexts(&self) -> anyhow::Result<()> {
        let api_key = {
            let s = self.state.lock().await;
            s.config.openrouter_api_key.clone()
        };

        let client = crate::openrouter::OpenRouterClient::new(api_key);
        let models = client.fetch_models().await?;

        let mut s = self.state.lock().await;
        for m in models {
            s.model_context_lengths.insert(m.id, m.context_length);
        }
        log::info!(
            "Cached {} model context lengths from OpenRouter",
            s.model_context_lengths.len()
        );
        Ok(())
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
        // self.git_repo
        //     .auto_commit("Update configuration via /command")?;
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
        process_message_impl(&self.state, &self.git_repo, &self.stop_signals, chat_id, user_id, username, text, bot_username, None).await
    }

    /// Clean up mismatched embeddings and start async backfill.
    /// If no embedding model is configured, does nothing.
    pub async fn start_embedding_backfill(&self) {
        let (model, api_key) = {
            let s = self.state.lock().await;
            match &s.config.embedding_model {
                Some(m) => (m.clone(), s.config.openrouter_api_key.clone()),
                None => return,
            }
        };

        // Phase 1: synchronous cleanup of mismatched embeddings
        let cleaned = {
            let s = self.state.lock().await;
            s.db.cleanup_mismatched_embeddings(&model).unwrap_or(0)
        };
        if cleaned > 0 {
            log::info!("Cleaned {} embeddings with old model", cleaned);
        }

        // Phase 2: async backfill in background
        let db = {
            let s = self.state.lock().await;
            s.db.clone()
        };

        tokio::spawn(async move {
            let unembedded = match db.find_unembedded_messages() {
                Ok(u) => u,
                Err(e) => {
                    log::warn!("Failed to find unembedded messages: {}", e);
                    return;
                }
            };

            if unembedded.is_empty() {
                log::info!("All messages are embedded, nothing to backfill.");
                return;
            }

            let total = unembedded.len();
            log::info!("Starting embedding backfill for {} messages...", total);

            let client = crate::openrouter::OpenRouterClient::new(api_key);

            for (idx, (msg_id, text)) in unembedded.iter().enumerate() {
                match client.embeddings(&model, text).await {
                    Ok(vec) => {
                        if let Err(e) = db.save_embedding(*msg_id, &vec, &model) {
                            log::warn!("Failed to save embedding for message {}: {}", msg_id, e);
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to embed message {}: {}", msg_id, e);
                    }
                }

                let done = idx + 1;
                let pct = (done as f64 / total as f64 * 100.0).round() as u32;
                println!("Embedding backfill: {}/{} ({}%) done", done, total, pct);
                log::info!("Embedding backfill: {}/{} ({}%) done", done, total, pct);

                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            log::info!("Embedding backfill complete: {} messages processed.", total);
            println!("Embedding backfill complete: {} messages processed.", total);
        });
    }

    /// Ensure a memory file exists (delegates to free function, used by tests).
    pub async fn ensure_memory_exists(
        &self,
        chat_id: &str,
        user_id: &str,
        username: &str,
    ) -> anyhow::Result<()> {
        ensure_memory_exists_impl(&self.state, chat_id, user_id, username).await
    }
}

/// Process an incoming message (free function, can be called without the GlowBot lock).
pub async fn process_message_impl(
    state: &Arc<Mutex<BotState>>,
    _git_repo: &GitRepo,
    stop_signals: &Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    chat_id: &str,
    user_id: &str,
    username: &str,
    text: &str,
    bot_username: &str,
    tg_bot: Option<&teloxide::Bot>,
) -> anyhow::Result<Option<String>> {
    let is_command = text.trim().starts_with('/');
    let is_mention = text.contains(&format!("@{}", bot_username));

    // Check if it's a bot command
    if let Some(command) = parse_command(text) {
        return handle_bot_command_impl(state, stop_signals, chat_id, user_id, &command, tg_bot, _git_repo).await;
    }

    let is_dm = !chat_id.starts_with('-');

    // Check interaction permissions (groups only — DMs don't have interaction_whitelist)
    if !is_dm {
        let chat_config = {
            let s = state.lock().await;
            s.config.chat_config(chat_id)
        };
        if !can_interact(&chat_config, user_id) {
            return Ok(None);
        }
    }

    if !is_dm
        && {
            let s = state.lock().await;
            let chat_config = s.config.chat_config(chat_id);
            matches!(chat_config.interaction_mode, crate::config::InteractionMode::MentionOnly)
        }
        && !is_command
        && !is_mention
    {
        return Ok(None);
    }

    if is_command && !is_mention {
        return Ok(None);
    }

    let (tools_enabled, dm_blocked) = if is_dm {
        let s = state.lock().await;
        if s.config.dm_config(chat_id).is_some() {
            (true, false)
        } else if s.config.dm_enabled_effective() {
            (false, false) // unknown DM, text-only respond
        } else {
            (false, true) // blocked
        }
    } else {
        (true, false)
    };

    if dm_blocked {
        return Ok(Some(format!(
            "I don't know you yet. Please ask the bot owner to add your user ID (`{}`) to the `dms` section in the config.",
            user_id
        )));
    }

    process_with_llm_impl(state, _git_repo, stop_signals, chat_id, user_id, username, text, tools_enabled, tg_bot).await
}

/// Handle a bot command (free function).
async fn handle_bot_command_impl(
    state: &Arc<Mutex<BotState>>,
    stop_signals: &Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    chat_id: &str,
    _user_id: &str,
    command: &crate::commands::Command,
    tg_bot: Option<&teloxide::Bot>,
    _git_repo: &GitRepo,
) -> anyhow::Result<Option<String>> {
    // /stop sets the stop signal and returns immediately
    if matches!(command, crate::commands::Command::Stop) {
        if let Ok(signals) = stop_signals.lock() {
            if let Some(signal) = signals.get(chat_id) {
                signal.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        return Ok(Some("Stop signal sent. Current operations will be cancelled.".into()));
    }

    let allowed = {
        let s = state.lock().await;
        let is_dm = !chat_id.starts_with('-');
        if is_dm {
            s.config.dm_config(chat_id).map(|d| d.commands_enabled).unwrap_or(false)
        } else {
            let chat_config = s.config.chat_config(chat_id);
            can_run_command(&chat_config)
        }
    };

    if !allowed {
        return Ok(Some("You are not authorized to run bot commands.".into()));
    }

    if matches!(command, crate::commands::Command::Tasks) {
        let s = state.lock().await;
        let list = crate::tasks::TaskList::load(&s.chats_dir(), chat_id).unwrap_or_default();
        let response = if list.tasks.is_empty() {
            "No pending tasks for this chat.".to_string()
        } else {
            let mut lines = vec![format!("*{} pending task(s):*", list.tasks.len())];
            for (i, t) in list.tasks.iter().enumerate() {
                lines.push(format!("{}. `{}` — {}", i + 1, t.id, t.description));
            }
            lines.join("\n")
        };
        return Ok(Some(response));
    }

    // /run triggers the heartbeat task agent immediately for this chat
    if matches!(command, crate::commands::Command::Run) {
        if let Some(bot) = tg_bot {
            let state_clone = Arc::clone(state);
            let git_clone = _git_repo.clone();
            let cid = chat_id.to_string();
            let tg_clone = bot.clone();
            tokio::spawn(async move {
                crate::bot::run_heartbeat_task(state_clone, git_clone, &cid, tg_clone).await;
            });
            return Ok(Some("🔄 Running task agent for this chat now...".into()));
        }
        return Ok(Some("Run command cannot be used in this context.".into()));
    }

    let response = {
        let mut s = state.lock().await;
        let usage = s.context_usage(chat_id);
        handle_command(command, &mut s.config, chat_id, &usage)
    };

    Ok(Some(response))
}

/// Process a message through the LLM pipeline (free function).
async fn process_with_llm_impl(
    state: &Arc<Mutex<BotState>>,
    _git_repo: &GitRepo,
    stop_signals: &Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    chat_id: &str,
    user_id: &str,
    username: &str,
    text: &str,
    tools_enabled: bool,
    tg_bot: Option<&teloxide::Bot>,
) -> anyhow::Result<Option<String>> {
    // Set up stop signal for this chat (clear any previous signal)
    {
        let mut signals = stop_signals.lock().unwrap();
        signals.entry(chat_id.to_string())
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    let check_stopped = || -> bool {
        if let Ok(signals) = stop_signals.lock() {
            signals.get(chat_id).map(|s| s.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(false)
        } else {
            false
        }
    };

    let (system_prompt, model) = {
        let s = state.lock().await;
        (s.assemble_system_prompt(chat_id, tools_enabled, user_id), s.effective_model(chat_id))
    };

    // Ensure user has a memory file
    ensure_memory_exists_impl(state, chat_id, user_id, username).await?;

    // Read existing conversation history upfront
    let (history, include_thoughts) = {
        let s = state.lock().await;
        let win = s.config.conversation.recent_messages_window_size;
        let include = s.config.conversation.include_thoughts;
        let hist = s
            .db
            .load_messages(chat_id, win)
            .unwrap_or_default();
        (hist, include)
    };

    let current_msg = ChatMessage::user_with_name(text, username);
    let mut turn_messages = vec![current_msg.clone()];

    let tools: Vec<crate::openrouter::ToolDefinition> = if tools_enabled {
        let s = state.lock().await;
        let bash_enabled = s.config.is_bash_enabled(chat_id);
        s.build_tools(bash_enabled)
    } else {
        vec![]
    };

    let context_limit = {
        let s = state.lock().await;
        s.model_context_lengths.get(&model).copied().unwrap_or(0)
    };

    let max_tool_rounds = 64;

    let (result, final_reasoning) = {
        let mut final_text = None;
        let mut final_reasoning = None;
        for _round in 0..max_tool_rounds {
            if check_stopped() {
                return Ok(Some("⏹ Stopped.".into()));
            }

            let (messages, _trimmed) = crate::openrouter::build_trimmed_request(
                context_limit,
                &[ChatMessage::system(&system_prompt)],
                &history,
                &turn_messages,
                &tools,
            );

            let request = ChatCompletionRequest {
                model: model.clone(),
                messages,
                tools: Some(tools.clone()),
                tool_choice: None,
            };

            let (response, usage) = {
                let s = state.lock().await;
                let resp = s.llm.chat_completion(&request).await?;
                let usage = resp.usage.clone().unwrap_or_default();
                (resp, usage)
            };
            {
                let mut s = state.lock().await;
                s.last_usage.insert(chat_id.to_string(), usage);
            }

            if check_stopped() {
                return Ok(Some("⏹ Stopped.".into()));
            }

            let choice = match response.choices.into_iter().next() {
                Some(c) => c,
                None => break,
            };

            if let Some(tool_calls) = &choice.message.tool_calls {
                if tool_calls.is_empty() {
                    final_text = Some(choice.message.content.clone().unwrap_or_default());
                    break;
                }

                // Record assistant's tool call message in the turn
                if let (Some(reasoning), true) = (&choice.message.reasoning, include_thoughts) {
                    turn_messages.push(ChatMessage::assistant_tool_calls_with_reasoning(
                        tool_calls.clone(),
                        reasoning.clone(),
                    ));
                } else {
                    turn_messages.push(ChatMessage::assistant_tool_calls(tool_calls.clone()));
                }

                let data_dir = { state.lock().await.data_dir.clone() };
                let results = dispatch_tool_calls(state, chat_id, tool_calls, Some(&data_dir), tg_bot).await;
                turn_messages.extend(results);

                // git_repo.auto_commit("Auto-commit after tool execution")?;

                if check_stopped() {
                    return Ok(Some("⏹ Stopped.".into()));
                }
                continue;
            }

            final_text = Some(choice.message.content.clone().unwrap_or_default());
            final_reasoning = choice.message.reasoning;
            break;
        }

        (final_text.unwrap_or_else(|| "I ran into a loop processing your request. Please try again.".into()), final_reasoning)
    };

    // Record final assistant message in the turn
    if let (Some(reasoning), true) = (&final_reasoning, include_thoughts) {
        turn_messages.push(ChatMessage::assistant_with_reasoning(&result, reasoning.clone()));
    } else {
        turn_messages.push(ChatMessage::assistant(&result));
    }

    // Store the completed turn in conversation history
    let message_ids = {
        let s = state.lock().await;
        s.db.save_messages(chat_id, &turn_messages).unwrap_or_default()
    };

    // Embed messages in the background if embedding model is configured
    {
        let s = state.lock().await;
        if let Some(ref embed_model) = s.config.embedding_model {
            if !message_ids.is_empty() {
                let api_key = s.config.openrouter_api_key.clone();
                let db = s.db.clone();
                let embed_model = embed_model.clone();
                let turn_messages = turn_messages.clone();
                drop(s);

                tokio::spawn(async move {
                    embed_turn(&db, &api_key, &embed_model, &message_ids, &turn_messages).await;
                });
            }
        }
    }

    Ok(Some(result))
}

/// Embed each message in a turn and store the vectors.
/// Runs as a background task — failures are logged but don't affect the user.
async fn embed_turn(
    db: &crate::db::Database,
    api_key: &str,
    embed_model: &str,
    message_ids: &[i64],
    turn_messages: &[ChatMessage],
) {
    let client = crate::openrouter::OpenRouterClient::new(api_key.to_string());
    for (i, msg) in turn_messages.iter().enumerate() {
        if i >= message_ids.len() {
            break;
        }
        let text = msg.text_content();
        if text.is_empty() {
            continue;
        }
        match client.embeddings(embed_model, &text).await {
            Ok(vec) => {
                if let Err(e) = db.save_embedding(message_ids[i], &vec, embed_model) {
                    log::warn!("Failed to save embedding for message {}: {}", message_ids[i], e);
                }
            }
            Err(e) => {
                log::warn!("Failed to embed message {}: {}", message_ids[i], e);
            }
        }
    }
}

async fn ensure_memory_exists_impl(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    user_id: &str,
    username: &str,
) -> anyhow::Result<()> {
    let s = state.lock().await;
    let existing = crate::memory::load_memory(&s.chats_dir(), chat_id, user_id);
    if existing.is_none() {
        let mem = Memory::new(user_id, username);
        save_memory(&s.chats_dir(), chat_id, user_id, &mem)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "bot_tests.rs"]
mod tests;

