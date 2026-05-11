use crate::commands::{can_interact, parse_command};
use crate::config::Config;
use crate::db::Database;
use crate::git::GitRepo;
use crate::llm::LlmBackend;
#[path = "bot_commands.rs"]
mod bot_commands;
#[path = "bot_models.rs"]
pub mod bot_models;
#[path = "bot_dispatch.rs"]
pub mod bot_dispatch;
#[path = "bot_heartbeat.rs"]
mod bot_heartbeat;
#[path = "bot_pipeline.rs"]
mod bot_pipeline;
#[path = "bot_state.rs"]
mod bot_state;
use self::bot_commands::handle_bot_command_impl;
pub use self::bot_heartbeat::run_heartbeat_task;
pub use self::bot_state::{BotState, PendingConfigChange};
use crate::skills::load_all_skills;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

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

        let schema_dir = std::env::var("GLOWBOT_SCHEMA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("schema"));

        let state = BotState {
            config,
            skills,
            llm,
            data_dir: data_dir.to_path_buf(),
            db: Database::new(&data_dir.join("conversations.db"), &schema_dir)?,
            mcp_tools,
            model_metadata: HashMap::new(),
            last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            model_overrides: HashMap::new(),
        };

        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            git_repo,
            stop_signals: Arc::new(std::sync::Mutex::new(HashMap::new())),
        })
    }

    /// Fetch model metadata from OpenRouter and populate the cache.
    #[allow(dead_code)]
    pub async fn fetch_model_metadata(&self) -> anyhow::Result<()> {
        let api_key = {
            let s = self.state.lock().await;
            s.config.openrouter.api_key.clone()
        };

        let client = crate::openrouter::OpenRouterClient::new(api_key);
        let models = client.fetch_models().await?;

        let mut s = self.state.lock().await;
        for m in models {
            s.model_metadata.insert(m.id.clone(), m);
        }
        log::info!(
            "Cached {} model metadata entries from OpenRouter",
            s.model_metadata.len()
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
        process_message_impl(
            &self.state,
            &self.git_repo,
            &self.stop_signals,
            chat_id,
            user_id,
            username,
            Some(text),
            None,
            None,
            bot_username,
            None,
        )
        .await
    }

    /// Clean up mismatched embeddings and start async backfill.
    /// If no embedding model is configured, does nothing.
    pub async fn start_embedding_backfill(&self) {
        let (model, api_key, max_chars, allow_split) = {
            let s = self.state.lock().await;
            match &s.config.openrouter.embedding_model {
                Some(m) => (
                    m.clone(),
                    s.config.openrouter.api_key.clone(),
                    s.config.embedding.max_chars,
                    s.config.embedding.allow_split,
                ),
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

            let chunker =
                |t: &str| self::bot_pipeline::chunk_for_embedding(t, max_chars, allow_split);
            for (idx, (msg_id, text)) in unembedded.iter().enumerate() {
                for chunk in &chunker(text) {
                    let text_preview: String = chunk.chars().take(80).collect();
                    match client.embeddings(&model, chunk).await {
                        Ok(vec) => {
                            if let Err(e) = db.save_embedding(*msg_id, &vec, &model) {
                                log::warn!("Failed to save embedding for message {} (model={}, text=\"{}\"): {}", msg_id, model, text_preview, e);
                            }
                        }
                        Err(e) => {
                            log::warn!(
                                "Failed to embed message {} (model={}, text=\"{}\"): {}",
                                msg_id,
                                model,
                                text_preview,
                                e
                            );
                        }
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
        self::bot_pipeline::ensure_memory_exists_impl(&self.state, chat_id, user_id, username).await
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
    text: Option<&str>,
    caption: Option<&str>,
    media: Option<&crate::media::IngestedMedia>,
    bot_username: &str,
    tg_bot: Option<&teloxide::Bot>,
) -> anyhow::Result<Option<String>> {
    let text = text.unwrap_or("");
    let is_command = text.trim().starts_with('/');
    let is_mention = text.contains(&format!("@{}", bot_username));

    // Check if it's a bot command
    if let Some(command) = parse_command(text) {
        log::info!(
            "bot: parsed command {:?}, dispatching to command handler",
            command
        );
        return handle_bot_command_impl(
            state,
            stop_signals,
            chat_id,
            user_id,
            &command,
            tg_bot,
            _git_repo,
        )
        .await;
    }

    let is_dm = !chat_id.starts_with('-');

    // Check interaction permissions (groups only — DMs don't have interaction_whitelist)
    if !is_dm {
        let chat_config = {
            let s = state.lock().await;
            s.config.chat_config(chat_id)
        };
        if !can_interact(&chat_config, user_id) {
            log::info!(
                "bot: user {} not in interaction whitelist for chat {}, ignoring",
                user_id,
                chat_id
            );
            return Ok(None);
        }
    }

    if !is_dm
        && {
            let s = state.lock().await;
            let chat_config = s.config.chat_config(chat_id);
            matches!(
                chat_config.interaction_mode,
                crate::config::InteractionMode::MentionOnly
            )
        }
        && !is_command
        && !is_mention
    {
        log::info!(
            "bot: chat {} is mention-only and message was not a mention, ignoring",
            chat_id
        );
        return Ok(None);
    }

    if is_command && !is_mention {
        log::info!("bot: message looks like a command but no mention, ignoring");
        return Ok(None);
    }

    let (tools_enabled, dm_blocked) = if is_dm {
        let s = state.lock().await;
        if s.config.dm_config(chat_id).is_some() {
            (true, false)
        } else {
            (false, true) // blocked
        }
    } else {
        (true, false)
    };

    if dm_blocked {
        log::info!(
            "bot: DM from unknown user {} blocked (dms not enabled and no dm config entry)",
            user_id
        );
        return Ok(Some(format!(
            "I don't know you yet. Please ask the bot owner to add your user ID (`{}`) to the `dms` section in the config.",
            user_id
        )));
    }

    log::info!(
        "bot: routing to LLM pipeline (chat={}, user={}, tools_enabled={}, is_dm={}, has_media={})",
        chat_id,
        user_id,
        tools_enabled,
        is_dm,
        media.is_some()
    );

    self::bot_pipeline::process_with_llm_impl(
        state,
        _git_repo,
        stop_signals,
        chat_id,
        user_id,
        username,
        text,
        caption,
        media,
        tools_enabled,
        tg_bot,
    )
    .await
}

#[cfg(test)]
#[path = "bot_tests.rs"]
mod tests;
