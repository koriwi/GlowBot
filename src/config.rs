use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
pub enum InteractionMode {
    #[serde(rename = "every_message")]
    EveryMessage,
    #[serde(rename = "mention_only")]
    #[default]
    MentionOnly,
}

/// Per-chat configuration override (groups only).
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct ChatConfig {
    /// Optional human-readable name for this chat (survives config updates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// MCP server names to blacklist for this chat.
    /// Tools from these servers will not be offered to the LLM.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_blacklist: Vec<String>,
    /// Override the global image fallback model for this chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_fallback_model: Option<String>,
    /// Override the global audio fallback model for this chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_fallback_model: Option<String>,
    /// Override the global image generation model for this chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_gen_model: Option<String>,
    /// Override the global advice model for this chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advice_model: Option<String>,
}

/// Per-DM configuration override.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct DmConfig {
    /// Optional human-readable name for this DM (survives config updates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional model override for this DM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Whether most bot commands are enabled for this DM.
    /// /todos, /tasks, /reminders, and /stop are always allowed regardless of this setting.
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
    /// Override the global image fallback model for this DM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_fallback_model: Option<String>,
    /// Override the global audio fallback model for this DM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_fallback_model: Option<String>,
    /// Override the global image generation model for this DM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_gen_model: Option<String>,
    /// Override the global advice model for this DM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advice_model: Option<String>,
}

/// Database-related configuration (placeholder for future options).
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct DatabaseConfig {}

/// Conversation context configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConversationConfig {
    /// Number of recent messages to load from the database as context.
    #[serde(default = "default_recent_messages_window_size")]
    pub recent_messages_window_size: usize,
    /// Number of recent messages to load from the database as context for heartbeat/background tasks.
    /// When set, heartbeat tasks use this value instead of `recent_messages_window_size`.
    /// When unset (default), heartbeat tasks use `recent_messages_window_size`.
    /// Set to `Some(0)` to give heartbeat tasks no conversation history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_recent_messages_window_size: Option<usize>,
    /// Number of recent messages (including tool calls and results) sent to the advice model
    /// when the LLM calls the `ask_advisor` tool. Default: 5.
    #[serde(default = "default_advice_recent_messages_window_size")]
    pub advice_recent_messages_window_size: usize,
    /// Maximum character length for tool call results. When set, tool results exceeding
    /// this limit are replaced with an error message telling the LLM to reduce the
    /// response size (via jq, grep, head, narrowing the query, etc.).
    /// When None (default), there is no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_result_chars: Option<usize>,
}

fn default_advice_recent_messages_window_size() -> usize {
    5
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            recent_messages_window_size: default_recent_messages_window_size(),
            heartbeat_recent_messages_window_size: None,
            advice_recent_messages_window_size: default_advice_recent_messages_window_size(),
            max_tool_result_chars: None,
        }
    }
}

/// Embedding configuration for conversation vector search (RAG).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingConfig {
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
            max_chars: 0,
            allow_split: false,
            search_limit: default_embedding_search_limit(),
        }
    }
}

/// OpenRouter API configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRouterConfig {
    /// OpenRouter API key.
    pub api_key: String,
    /// Model to use (required).
    pub model: String,
    /// Model used to describe images when the conversation model
    /// doesn't natively support image input.
    /// This model should support the `image` input modality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_fallback_model: Option<String>,
    /// Model used to transcribe audio/voice when the conversation model
    /// doesn't natively support audio input.
    /// This model should support the `audio` input modality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_fallback_model: Option<String>,
    /// Model used for image generation via the `generate_image` tool.
    /// Must be an image generation model available on OpenRouter
    /// (e.g. "google/imagen-4", "black-forest-labs/flux-1.1-pro").
    /// If unset, the `generate_image` tool is not exposed to the LLM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_gen_model: Option<String>,
    /// Embedding model name for conversation vector search (RAG).
    /// When set, every message is embedded and stored.
    /// Example: "openai/text-embedding-3-small"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    /// Model used for the `ask_advisor` tool. When set, the LLM can call this tool
    /// to query a larger/smarter model for advice. When None, the tool is disabled.
    /// No default — must be explicitly configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advice_model: Option<String>,
}

/// Global application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    /// Telegram bot token.
    pub telegram_token: String,
    /// OpenRouter configuration.
    pub openrouter: OpenRouterConfig,
    /// Conversation context settings (window sizes for history and advice).
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
