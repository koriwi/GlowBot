use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Interaction mode for a chat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum InteractionMode {
    #[serde(rename = "every_message")]
    EveryMessage,
    #[serde(rename = "mention_only")]
    #[default]
    MentionOnly,
}

/// Per-chat configuration override.
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
    /// User IDs allowed to run commands. Empty = nobody.
    #[serde(default)]
    pub command_whitelist: Vec<String>,
    /// Optional per-chat system prompt appended to the base.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
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
    /// Number of recent messages to include as conversation context.
    #[serde(default = "default_conversation_window")]
    pub conversation_window: usize,
    /// DM whitelist: user IDs allowed full tool access in DMs.
    /// Empty = DMs respond but tools disabled. Non-empty = only listed users can interact.
    #[serde(default)]
    pub dm_whitelist: Vec<String>,
    /// Per-chat configuration overrides, keyed by chat ID string.
    #[serde(default)]
    pub chats: HashMap<String, ChatConfig>,
}

fn default_model() -> String {
    "anthropic/claude-sonnet-4".to_string()
}

fn default_conversation_window() -> usize {
    20
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

    /// Get the effective chat config for a given chat ID.
    pub fn chat_config(&self, chat_id: &str) -> ChatConfig {
        self.chats.get(chat_id).cloned().unwrap_or_default()
    }

    /// Get the effective model for a given chat ID.
    pub fn model_for_chat(&self, chat_id: &str) -> &str {
        self.chats
            .get(chat_id)
            .and_then(|c| c.model.as_deref())
            .unwrap_or(&self.openrouter_default_model)
    }

    /// Check whether tools are enabled for a DM user.
    /// Returns true if dm_whitelist is non-empty AND the user is in it.
    pub fn dm_tools_enabled(&self, user_id: &str) -> bool {
        if self.dm_whitelist.is_empty() {
            return false;
        }
        self.dm_whitelist.contains(&user_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn basic_config() -> Config {
        Config {
            telegram_token: "test-token".into(),
            openrouter_api_key: "test-key".into(),
            openrouter_default_model: "test/model".into(),
            conversation_window: 20,
            dm_whitelist: vec![],
            chats: HashMap::new(),
        }
    }

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
        assert!(chat.command_whitelist.is_empty());
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
    fn test_whitelists_serialization() {
        let chat = ChatConfig {
            interaction_whitelist: vec!["123".into(), "456".into()],
            command_whitelist: vec!["789".into()],
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&chat).unwrap();
        let loaded: ChatConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(loaded.interaction_whitelist, vec!["123", "456"]);
        assert_eq!(loaded.command_whitelist, vec!["789"]);
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
    fn test_dm_tools_disabled_when_whitelist_empty() {
        let config = basic_config();
        assert!(!config.dm_tools_enabled("anyone"));
    }

    #[test]
    fn test_dm_tools_enabled_for_whitelisted_user() {
        let mut config = basic_config();
        config.dm_whitelist = vec!["123".into()];
        assert!(config.dm_tools_enabled("123"));
        assert!(!config.dm_tools_enabled("456"));
    }
}
