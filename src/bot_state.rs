use crate::config::Config;
use crate::db::Database;
use crate::llm::LlmBackend;
use crate::openrouter::Usage;
use crate::skills::Skill;
use std::collections::HashMap;
use std::sync::Arc;

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

    /// Get the effective model for a chat.
    pub fn effective_model(&self, chat_id: &str) -> String {
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
            self.config.embedding.model.as_deref(),
            &self.config.media_dir,
        );
        for mt in &self.mcp_tools {
            if !self.config.is_mcp_server_allowed(chat_id, &mt.server_name) {
                log::info!(
                    "MCP server '{}' blacklisted for chat {}, skipping tools",
                    mt.server_name,
                    chat_id
                );
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
        let raw_limit = self.model_context_lengths.get(crate::openrouter::normalize_model_id(&model)).copied().unwrap_or(0);
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
