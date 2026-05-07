use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    /// Display name for this server.
    #[serde(default)]
    pub name: String,
    /// Transport type: "http" (stateless) or "streamable" (session-based). Default: "streamable".
    #[serde(default = "default_transport")]
    pub transport: String,
    /// HTTP endpoint URL for the MCP server.
    pub url: String,
    /// Optional Bearer token for Authorization header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

fn default_transport() -> String {
    "streamable".into()
}

/// Interaction mode for a chat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum InteractionMode {
    #[serde(rename = "every_message")]
    EveryMessage,
    #[serde(rename = "mention_only")]
    #[default]
    MentionOnly,
}

/// Per-chat configuration override (groups only).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatConfig {
    /// Optional model override for this chat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Interaction mode for this chat.
    #[serde(default)]
    pub interaction_mode: InteractionMode,
    /// User IDs allowed to interact with the bot. Empty = everyone.
    #[serde(default)]
    pub interaction_whitelist: Vec<String>,
    /// User IDs allowed to run bot commands. Empty = nobody can run commands.
    #[serde(default)]
    pub command_whitelist: Vec<String>,
    /// Optional per-chat system prompt appended to the base.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    /// Heartbeat interval in minutes for this chat (0 = disabled).
    /// If unset, falls back to the global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_minutes: Option<u64>,
    /// Override the global bash_enabled setting for this chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bash_enabled: Option<bool>,
}

/// Per-DM configuration override.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DmConfig {
    /// Optional model override for this DM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Whether bot commands (/status, /stop, /tasks, /run) are enabled for this DM.
    #[serde(default)]
    pub commands_enabled: bool,
    /// Optional per-DM system prompt appended to the base.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    /// Heartbeat interval in minutes for this DM (0 = disabled).
    /// If unset, falls back to the global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_minutes: Option<u64>,
    /// Override the global bash_enabled setting for this DM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bash_enabled: Option<bool>,
}

/// Database-related configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatabaseConfig {
    /// Whether to persist LLM reasoning/thinking content in the database.
    /// Reasoning is only captured when `conversation.include_reasoning` is also enabled.
    #[serde(default)]
    pub store_reasoning: bool,
}

/// Conversation context configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationConfig {
    /// Number of recent messages to load from the database as context.
    #[serde(default = "default_recent_messages_window_size")]
    pub recent_messages_window_size: usize,
    /// Whether to include the model's reasoning/thinking content in subsequent requests.
    /// When enabled, reasoning text from assistant messages is captured and sent back
    /// in the next turn so the model can see its previous thinking.
    #[serde(default)]
    pub include_reasoning: bool,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            recent_messages_window_size: default_recent_messages_window_size(),
            include_reasoning: false,
        }
    }
}

/// Embedding configuration for conversation vector search (RAG).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Embedding model name. When set, every message is embedded and stored.
    /// Example: "openai/text-embedding-3-small"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Maximum characters per embedding. 0 means no limit (full text).
    /// When text exceeds this, it is either truncated (allow_split=false)
    /// or split into multiple chunks (allow_split=true).
    #[serde(default)]
    pub max_chars: usize,
    /// When true, split long text into chunks of max_chars, creating separate
    /// embedding entries per chunk. When false, truncate to max_chars.
    #[serde(default)]
    pub allow_split: bool,
    /// Maximum number of embeddings loaded for similarity search.
    #[serde(default = "default_embedding_search_limit")]
    pub search_limit: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: None,
            max_chars: 0,
            allow_split: false,
            search_limit: default_embedding_search_limit(),
        }
    }
}

/// OpenRouter API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterConfig {
    /// OpenRouter API key.
    pub api_key: String,
    /// Model to use (required).
    pub model: String,
}

/// Global application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Telegram bot token.
    pub telegram_token: String,
    /// OpenRouter configuration.
    pub openrouter: OpenRouterConfig,
    /// Conversation context settings (window size, thought inclusion).
    #[serde(default)]
    pub conversation: ConversationConfig,

    /// Per-chat configuration overrides for groups, keyed by chat ID string (negative).
    #[serde(default)]
    pub chats: HashMap<String, ChatConfig>,
    /// Per-DM configuration overrides, keyed by user/chat ID string (positive).
    #[serde(default)]
    pub dms: HashMap<String, DmConfig>,

    /// MCP servers to connect to for additional tools.
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    /// Default heartbeat interval in minutes (default: 90).
    #[serde(default = "default_heartbeat")]
    pub heartbeat_interval_minutes: u64,
    /// Whether the bash tool is enabled globally (default: true).
    /// Used as fallback when no per-chat override is set.
    #[serde(default = "default_bash_enabled")]
    pub bash_enabled: bool,
    /// Embedding configuration for RAG vector search.
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    /// Database-related configuration.
    #[serde(default)]
    pub db: DatabaseConfig,
    /// Directory where media files (images, videos, etc.) are stored,
    /// typically used by MCP tools or bash commands for downloads.
    /// The send_media tool uses this path for finding files.
    #[serde(default = "default_media_dir")]
    pub media_dir: String,
}

fn default_recent_messages_window_size() -> usize {
    20
}

fn default_heartbeat() -> u64 {
    90
}

fn default_bash_enabled() -> bool {
    true
}

fn default_embedding_search_limit() -> usize {
    1000
}

fn default_media_dir() -> String {
    "/media".into()
}

#[path = "config_methods.rs"]
mod config_methods;
#[cfg(test)]
pub(crate) use self::config_methods::basic_config;

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
