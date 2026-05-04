use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

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
    /// Whether bot commands (/status, /stop, /tasks, /run) are enabled for this chat.
    #[serde(default)]
    pub commands_enabled: bool,
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
    /// Reasoning is only captured when `conversation.include_thoughts` is also enabled.
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
    pub include_thoughts: bool,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            recent_messages_window_size: default_recent_messages_window_size(),
            include_thoughts: false,
        }
    }
}

/// Global application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Telegram bot token.
    pub telegram_token: String,
    /// OpenRouter API key.
    pub openrouter_api_key: String,
    /// Default OpenRouter model.
    #[serde(default = "default_model")]
    pub openrouter_default_model: String,
    /// Conversation context settings (window size, thought inclusion).
    #[serde(default)]
    pub conversation: ConversationConfig,

    /// Per-chat configuration overrides for groups, keyed by chat ID string (negative).
    #[serde(default)]
    pub chats: HashMap<String, ChatConfig>,
    /// Per-DM configuration overrides, keyed by user/chat ID string (positive).
    #[serde(default)]
    pub dms: HashMap<String, DmConfig>,
    /// Control whether unknown DMs (not in `dms`) get a response.
    /// - `None` + `dms` is empty → text-only respond (backward-compatible).
    /// - `None` + `dms` is non-empty → block with "I don't know you" message.
    /// - `Some(true)` → text-only respond to unknown DMs.
    /// - `Some(false)` → block with "I don't know you" message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dm_enabled: Option<bool>,
    /// MCP servers to connect to for additional tools.
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    /// Default heartbeat interval in minutes (default: 90).
    #[serde(default = "default_heartbeat")]
    pub heartbeat_interval_minutes: u64,
    /// How often (in seconds) the scheduler scans for chats with pending tasks.
    /// Default: 60s. Increase this if you have many chats and want less filesystem churn.
    #[serde(default = "default_heartbeat_scan_interval")]
    pub heartbeat_scan_interval_seconds: u64,
    /// Whether the bash tool is enabled globally (default: true).
    /// Used as fallback when no per-chat override is set.
    #[serde(default = "default_bash_enabled")]
    pub bash_enabled: bool,
    /// Embedding model for conversation vector search.
    /// When set, every message is embedded and stored for RAG retrieval.
    /// Example: "openai/text-embedding-3-small"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    /// Maximum number of recent embeddings loaded for similarity search.
    #[serde(default = "default_embedding_search_limit")]
    pub embedding_search_limit: usize,
    /// Database-related configuration.
    #[serde(default)]
    pub db: DatabaseConfig,
}

fn default_model() -> String {
    "anthropic/claude-sonnet-4".to_string()
}

fn default_recent_messages_window_size() -> usize {
    20
}

fn default_heartbeat() -> u64 {
    90
}

fn default_heartbeat_scan_interval() -> u64 {
    60
}

fn default_bash_enabled() -> bool {
    true
}

fn default_embedding_search_limit() -> usize {
    1000
}

impl Config {
    /// Load configuration from a YAML file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: Self = serde_yaml::from_str(&data)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        Ok(config)
    }

    /// Save configuration to a YAML file.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let data = serde_yaml::to_string(self)?;
        std::fs::write(path, data)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;
        Ok(())
    }

    /// Get the effective chat config for a given chat ID (groups only).
    pub fn chat_config(&self, chat_id: &str) -> ChatConfig {
        self.chats.get(chat_id).cloned().unwrap_or_default()
    }

    /// Get the DM config for a given chat ID, if any.
    pub fn dm_config(&self, chat_id: &str) -> Option<&DmConfig> {
        self.dms.get(chat_id)
    }

    /// Resolve the effective `dm_enabled` with the implicit default:
    /// `None` + `dms` is empty → true (backward-compatible).
    /// `None` + `dms` is non-empty → false (presence of entries implies control).
    pub fn dm_enabled_effective(&self) -> bool {
        self.dm_enabled.unwrap_or(self.dms.is_empty())
    }

    /// Get the effective model for a given chat ID.
    /// For DMs, checks the `dms` entry first.
    pub fn model_for_chat(&self, chat_id: &str) -> &str {
        if !chat_id.starts_with('-') {
            if let Some(dm) = self.dms.get(chat_id) {
                if let Some(ref m) = dm.model {
                    return m;
                }
            }
        }
        self.chats
            .get(chat_id)
            .and_then(|c| c.model.as_deref())
            .unwrap_or(&self.openrouter_default_model)
    }

    /// Check whether the bash tool is enabled for a given chat.
    /// Per-chat override takes precedence; falls back to global default.
    /// For DMs, checks the `dms` entry first.
    pub fn is_bash_enabled(&self, chat_id: &str) -> bool {
        if !chat_id.starts_with('-') {
            if let Some(dm) = self.dms.get(chat_id) {
                if let Some(b) = dm.bash_enabled {
                    return b;
                }
            }
        }
        self.chats
            .get(chat_id)
            .and_then(|c| c.bash_enabled)
            .unwrap_or(self.bash_enabled)
    }

    /// Get the effective heartbeat interval for a chat (global default if not overridden).
    /// Returns None if disabled (set to 0).
    /// For DMs, checks the `dms` entry first.
    pub fn heartbeat_interval(&self, chat_id: &str) -> Option<u64> {
        let interval = if !chat_id.starts_with('-') {
            self.dms
                .get(chat_id)
                .and_then(|d| d.heartbeat_interval_minutes)
                .or_else(|| {
                    self.chats
                        .get(chat_id)
                        .and_then(|c| c.heartbeat_interval_minutes)
                })
                .unwrap_or(self.heartbeat_interval_minutes)
        } else {
            self.chats
                .get(chat_id)
                .and_then(|c| c.heartbeat_interval_minutes)
                .unwrap_or(self.heartbeat_interval_minutes)
        };
        if interval == 0 {
            None
        } else {
            Some(interval)
        }
    }
}

/// Test helper: a basic Config with minimal defaults.
#[cfg(test)]
pub(crate) fn basic_config() -> Config {
    Config {
        telegram_token: "test-token".into(),
        openrouter_api_key: "test-key".into(),
        openrouter_default_model: "test/model".into(),
        conversation: ConversationConfig::default(),
        db: DatabaseConfig::default(),

        mcp_servers: vec![],
        heartbeat_interval_minutes: 90,
        heartbeat_scan_interval_seconds: 60,
        bash_enabled: true,
        embedding_model: None,
        embedding_search_limit: 1000,
        chats: HashMap::new(),
        dms: HashMap::new(),
        dm_enabled: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_config_load_save_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let config = basic_config();
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.telegram_token, config.telegram_token);
        assert_eq!(loaded.openrouter_api_key, config.openrouter_api_key);
        assert_eq!(
            loaded.openrouter_default_model,
            config.openrouter_default_model
        );
    }

    #[test]
    fn test_config_load_nonexistent() {
        let result = Config::load(Path::new("/nonexistent/path/config.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_config_invalid_yaml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "invalid: [yaml:").unwrap();
        let result = Config::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_chat_config_defaults() {
        let config = basic_config();
        let chat = config.chat_config("-123");
        assert_eq!(chat.interaction_mode, InteractionMode::MentionOnly);
        assert!(chat.model.is_none());
        assert!(chat.interaction_whitelist.is_empty());
        assert!(!chat.commands_enabled);
        assert!(chat.system_prompt.is_empty());
    }

    #[test]
    fn test_chat_config_override() {
        let mut config = basic_config();
        let chat = ChatConfig {
            interaction_mode: InteractionMode::EveryMessage,
            model: Some("custom/model".into()),
            ..Default::default()
        };
        config.chats.insert("-123".into(), chat);
        let loaded = config.chat_config("-123");
        assert_eq!(loaded.interaction_mode, InteractionMode::EveryMessage);
        assert_eq!(loaded.model.unwrap(), "custom/model");
    }

    #[test]
    fn test_model_for_chat() {
        let mut config = basic_config();
        // default
        assert_eq!(config.model_for_chat("-123"), "test/model");
        // override
        config.chats.insert(
            "-123".into(),
            ChatConfig {
                model: Some("custom/model".into()),
                ..Default::default()
            },
        );
        assert_eq!(config.model_for_chat("-123"), "custom/model");
    }

    #[test]
    fn test_interaction_mode_serde() {
        let yaml = "interaction_mode: every_message\n";
        let chat: ChatConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(chat.interaction_mode, InteractionMode::EveryMessage);

        let yaml2 = "interaction_mode: mention_only\n";
        let chat2: ChatConfig = serde_yaml::from_str(yaml2).unwrap();
        assert_eq!(chat2.interaction_mode, InteractionMode::MentionOnly);
    }

    #[test]
    fn test_chat_config_serialization() {
        let chat = ChatConfig {
            interaction_whitelist: vec!["123".into(), "456".into()],
            commands_enabled: true,
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&chat).unwrap();
        let loaded: ChatConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(loaded.interaction_whitelist, vec!["123", "456"]);
        assert!(loaded.commands_enabled);
    }

    #[test]
    fn test_config_save_to_invalid_path() {
        let config = basic_config();
        let result = config.save(Path::new("/nonexistent/dir/config.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_default_model() {
        assert_eq!(default_model(), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_chat_config_system_prompt() {
        let chat = ChatConfig {
            system_prompt: "custom prompt".into(),
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&chat).unwrap();
        assert!(yaml.contains("custom prompt"));
    }

    #[test]
    fn test_bash_enabled_global_default() {
        let config = basic_config();
        assert!(config.is_bash_enabled("-123"));
        assert!(config.is_bash_enabled("456"));
    }

    #[test]
    fn test_bash_enabled_per_chat_override_false() {
        let mut config = basic_config();
        config.chats.insert(
            "-123".into(),
            ChatConfig {
                bash_enabled: Some(false),
                ..Default::default()
            },
        );
        assert!(!config.is_bash_enabled("-123"));
        assert!(config.is_bash_enabled("-999")); // uses global true
    }

    #[test]
    fn test_bash_enabled_per_chat_override_true_global_false() {
        let mut config = basic_config();
        config.bash_enabled = false;
        config.chats.insert(
            "-123".into(),
            ChatConfig {
                bash_enabled: Some(true),
                ..Default::default()
            },
        );
        assert!(config.is_bash_enabled("-123"));
        assert!(!config.is_bash_enabled("-999")); // uses global false
    }

    #[test]
    fn test_heartbeat_per_chat_override_and_global_fallback() {
        let mut config = basic_config();
        config.heartbeat_interval_minutes = 90;
        config.chats.insert(
            "-123".into(),
            ChatConfig {
                heartbeat_interval_minutes: Some(30),
                ..Default::default()
            },
        );
        assert_eq!(config.heartbeat_interval("-123"), Some(30));
        assert_eq!(config.heartbeat_interval("-999"), Some(90));
    }

    // --- DmConfig & dm_enabled tests ---

    #[test]
    fn test_dm_config_defaults() {
        let dm = DmConfig::default();
        assert!(dm.model.is_none());
        assert!(!dm.commands_enabled);
        assert!(dm.system_prompt.is_empty());
        assert!(dm.heartbeat_interval_minutes.is_none());
        assert!(dm.bash_enabled.is_none());
    }

    #[test]
    fn test_dm_config_serialization() {
        let dm = DmConfig {
            model: Some("anthropic/claude-haiku-4".into()),
            commands_enabled: true,
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&dm).unwrap();
        let loaded: DmConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(loaded.model.unwrap(), "anthropic/claude-haiku-4");
        assert!(loaded.commands_enabled);
    }

    #[test]
    fn test_dm_enabled_effective_none_empty_dms() {
        let config = basic_config(); // dm_enabled=None, dms=empty
        assert!(config.dm_enabled_effective());
    }

    #[test]
    fn test_dm_enabled_effective_none_nonempty_dms() {
        let mut config = basic_config();
        config.dms.insert("123".into(), DmConfig::default());
        assert!(!config.dm_enabled_effective());
    }

    #[test]
    fn test_dm_enabled_effective_some_true() {
        let mut config = basic_config();
        config.dm_enabled = Some(true);
        assert!(config.dm_enabled_effective());
    }

    #[test]
    fn test_dm_enabled_effective_some_false() {
        let mut config = basic_config();
        config.dm_enabled = Some(false);
        assert!(!config.dm_enabled_effective());
    }

    #[test]
    fn test_dm_enabled_effective_some_true_ignores_dms_nonempty() {
        let mut config = basic_config();
        config.dm_enabled = Some(true);
        config.dms.insert("123".into(), DmConfig::default());
        assert!(config.dm_enabled_effective());
    }

    #[test]
    fn test_model_for_dm() {
        let mut config = basic_config();
        // DM without entry uses global default
        assert_eq!(config.model_for_chat("123"), "test/model");
        // DM with entry uses its model
        let mut dm = DmConfig::default();
        dm.model = Some("dm-model".into());
        config.dms.insert("123".into(), dm);
        assert_eq!(config.model_for_chat("123"), "dm-model");
    }

    #[test]
    fn test_bash_enabled_for_dm() {
        let mut config = basic_config();
        // DM without entry uses global
        assert!(config.is_bash_enabled("456"));
        // DM with bash_enabled: false
        let mut dm = DmConfig::default();
        dm.bash_enabled = Some(false);
        config.dms.insert("456".into(), dm);
        assert!(!config.is_bash_enabled("456"));
    }

    #[test]
    fn test_heartbeat_for_dm() {
        let mut config = basic_config();
        config.heartbeat_interval_minutes = 60;
        // DM without entry uses global
        assert_eq!(config.heartbeat_interval("789"), Some(60));
        // DM with override
        let mut dm = DmConfig::default();
        dm.heartbeat_interval_minutes = Some(15);
        config.dms.insert("789".into(), dm);
        assert_eq!(config.heartbeat_interval("789"), Some(15));
    }

    #[test]
    fn test_heartbeat_for_dm_disabled() {
        let mut config = basic_config();
        let mut dm = DmConfig::default();
        dm.heartbeat_interval_minutes = Some(0);
        config.dms.insert("111".into(), dm);
        assert_eq!(config.heartbeat_interval("111"), None);
    }

    #[test]
    fn test_dm_config_found_and_not_found() {
        let mut config = basic_config();
        assert!(config.dm_config("123").is_none());
        config.dms.insert("123".into(), DmConfig::default());
        assert!(config.dm_config("123").is_some());
    }

    #[test]
    fn test_config_load_save_with_dms() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let mut config = basic_config();
        config.dms.insert("42".into(), DmConfig {
            commands_enabled: true,
            ..Default::default()
        });
        config.dm_enabled = Some(false);
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.dm_enabled, Some(false));
        assert!(loaded.dms.contains_key("42"));
        assert!(loaded.dms.get("42").unwrap().commands_enabled);
    }

    #[test]
    fn test_embedding_model_serialization() {
        let mut config = basic_config();
        assert!(config.embedding_model.is_none());
        config.embedding_model = Some("openai/text-embedding-3-small".into());
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(
            loaded.embedding_model.as_deref(),
            Some("openai/text-embedding-3-small")
        );
    }

    #[test]
    fn test_embedding_search_limit_default() {
        let config = basic_config();
        assert_eq!(config.embedding_search_limit, 1000);
    }

    // --- ConversationConfig tests ---

    #[test]
    fn test_conversation_config_default() {
        let conv = ConversationConfig::default();
        assert_eq!(conv.recent_messages_window_size, 20);
        assert!(!conv.include_thoughts);
    }

    #[test]
    fn test_conversation_config_serialization() {
        let conv = ConversationConfig {
            recent_messages_window_size: 50,
            include_thoughts: true,
        };
        let yaml = serde_yaml::to_string(&conv).unwrap();
        assert!(yaml.contains("50"));
        assert!(yaml.contains("include_thoughts"));
    }

    #[test]
    fn test_config_load_save_with_conversation_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let mut config = basic_config();
        config.conversation.recent_messages_window_size = 30;
        config.conversation.include_thoughts = true;
        config.db.store_reasoning = true;
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.conversation.recent_messages_window_size, 30);
        assert!(loaded.conversation.include_thoughts);
        assert!(loaded.db.store_reasoning);
    }

    #[test]
    fn test_database_config_default() {
        let db = DatabaseConfig::default();
        assert!(!db.store_reasoning);
    }

    #[test]
    fn test_database_config_serialization() {
        let db = DatabaseConfig {
            store_reasoning: true,
        };
        let yaml = serde_yaml::to_string(&db).unwrap();
        assert!(yaml.contains("store_reasoning"));
    }

    #[test]
    fn test_config_defaults_db_store_reasoning_false() {
        let config = basic_config();
        assert!(!config.db.store_reasoning);
    }

    #[test]
    fn test_embedding_search_limit_serialization() {
        let mut config = basic_config();
        config.embedding_search_limit = 500;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.embedding_search_limit, 500);
    }
}
