use crate::config::Config;
use crate::db::Database;
use crate::llm::LlmBackend;
use crate::openrouter::{ModelInfo, Usage};
use crate::skills::Skill;
use std::collections::HashMap;
use std::sync::Arc;

/// A pending config change awaiting user approval via inline keyboard.
#[derive(Debug, Clone)]
pub struct PendingConfigChange {
    pub chat_id: String,
    pub message_id: i32,
    pub new_yaml: String,
}

/// A pending model change proposal awaiting user approval via inline keyboard.
#[derive(Debug, Clone)]
pub struct PendingModelChange {
    pub chat_id: String,
    pub message_id: i32,
    pub proposed_model: String,
}

/// Shared bot state accessible from all handlers.
pub struct BotState {
    pub config: Config,
    pub skills: HashMap<String, Skill>,
    pub llm: Arc<dyn LlmBackend>,
    pub data_dir: std::path::PathBuf,
    /// SQLite-backed conversation history (one row per message).
    pub db: Database,
    /// Tools discovered from MCP servers (for LLM tool definitions).
    pub mcp_tools: Vec<crate::mcp::McpToolInfo>,
    /// Live MCP connections — must be kept alive for peers to function.
    /// Not accessed directly; cloned Peers from `mcp_peers` are used instead.
    pub _mcp_services: Vec<crate::mcp::McpConnection>,
    /// Cloned peer handles for MCP tool invocation, keyed by server name.
    pub mcp_peers:
        std::collections::HashMap<String, rmcp::service::Peer<rmcp::service::RoleClient>>,
    /// Cached model metadata from OpenRouter (includes context lengths and input modalities).
    pub model_metadata: HashMap<String, ModelInfo>,
    /// Model IDs in the order they were returned by the API (for "popular" sort).
    pub model_order: Vec<String>,
    /// Per-chat last token usage from the most recent LLM call.
    pub last_usage: HashMap<String, Usage>,
    /// Pending config changes awaiting user approval (keyed by pending_id).
    pub pending_config_changes: HashMap<String, PendingConfigChange>,
    /// Pending model change proposals awaiting user approval (keyed by pending_id).
    pub pending_model_changes: HashMap<String, PendingModelChange>,
    /// Per-chat temporary model overrides set via /models (keyed by chat_id string).
    /// Cleared on /model_default or restart.
    pub model_overrides: HashMap<String, String>,
    /// Per-chat temporary provider overrides set via /model or /models.
    /// Cleared on /model_default or restart.
    pub provider_overrides: HashMap<String, crate::config::LlmProvider>,
    /// Provider currently being browsed by the interactive model picker.
    pub picker_providers: HashMap<String, crate::config::LlmProvider>,
    /// Per-chat last browse callback data, used for "Back" navigation
    /// from the model detail view back to the originating browse page.
    pub last_browse_cb: HashMap<String, String>,
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
        let bash_enabled = if tools_enabled {
            self.config.is_bash_enabled(chat_id)
        } else {
            false
        };
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
            bash_enabled,
            user_id,
            &self.config.media_dir,
        )
    }

    /// Get the effective provider for a chat, respecting any temporary override.
    pub fn effective_provider(&self, chat_id: &str) -> crate::config::LlmProvider {
        self.provider_overrides
            .get(chat_id)
            .copied()
            .unwrap_or_else(|| self.config.provider_for_chat(chat_id))
    }

    /// Get the effective model for a chat, respecting temporary overrides.
    pub fn effective_model(&self, chat_id: &str) -> String {
        if let Some(override_model) = self.model_overrides.get(chat_id) {
            return override_model.clone();
        }
        if self.provider_overrides.contains_key(chat_id) {
            return self
                .config
                .default_model_for_provider(self.effective_provider(chat_id))
                .to_string();
        }
        self.config.model_for_chat(chat_id).to_string()
    }

    /// Build the full list of tool definitions including MCP tools.
    /// Filters out MCP servers blacklisted for the given chat.
    /// `send_message` is always included — in normal conversations it's for
    /// headsup/intermediate messages; in heartbeat tasks it's for completion reports.
    pub fn build_tools(
        &self,
        include_bash: bool,
        chat_id: &str,
    ) -> Vec<crate::openrouter::ToolDefinition> {
        let mut t = crate::openrouter::all_tool_definitions(
            include_bash,
            self.config.openrouter.embedding_model.as_deref(),
            &self.config.media_dir,
            self.config.image_gen_model_for_chat(chat_id),
            self.config.image_fallback_model_for_chat(chat_id),
            self.config.advice_model_for_chat(chat_id),
        );
        let mut blacklisted_counts: HashMap<&str, usize> = HashMap::new();
        for mt in &self.mcp_tools {
            if !self.config.is_mcp_server_allowed(chat_id, &mt.server_name) {
                *blacklisted_counts.entry(&mt.server_name).or_insert(0) += 1;
                continue;
            }
            t.push(crate::openrouter::ToolDefinition {
                def_type: "function".into(),
                function: crate::openrouter::FunctionDef {
                    name: format!("mcp_{}_{}", mt.server_name, mt.name),
                    description: format!("[MCP: {}] {}", mt.server_name, mt.description),
                    parameters: mt.input_schema.clone(),
                },
            });
        }
        for (server_name, count) in &blacklisted_counts {
            log::info!(
                "{} tools from MCP server '{}' blacklisted for chat {}, skipping",
                count,
                server_name,
                chat_id
            );
        }
        t
    }

    /// Check if a chat has pending tasks.
    pub fn has_pending_tasks(&self, chat_id: &str) -> bool {
        let list = crate::tasks::TaskList::load(&self.chats_dir(), chat_id).unwrap_or_default();
        list.has_tasks()
    }

    /// Check if a chat has any due reminders (trigger_at in the past).
    pub fn has_due_reminders(&self, chat_id: &str) -> bool {
        let list =
            crate::reminders::ReminderList::load(&self.chats_dir(), chat_id).unwrap_or_default();
        !list.due().is_empty()
    }

    /// Get the formatted context usage string for a chat, e.g. "37k/189k (15%)"
    /// Reports against the *effective* limit (with safety margin applied).
    pub fn context_usage(&self, chat_id: &str) -> String {
        let model = self.effective_model(chat_id);
        let raw_limit = self
            .model_metadata
            .get(crate::openrouter::normalize_model_id(&model))
            .map(|m| m.context_length)
            .unwrap_or(0);
        let effective_limit = if raw_limit == 0 {
            log::warn!(
                "Model '{}' not found in context length cache; context usage will be limited",
                model
            );
            0
        } else {
            (raw_limit as f64 * crate::openrouter::TOKEN_ESTIMATE_MARGIN) as u64
        };
        let usage = self.last_usage.get(chat_id).cloned().unwrap_or_default();
        crate::openrouter::format_context_usage(usage.prompt_tokens, effective_limit)
    }
}
