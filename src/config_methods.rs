use super::*;
use anyhow::Context;
use std::path::Path;

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
            .unwrap_or(&self.openrouter.model)
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
        openrouter: OpenRouterConfig {
            api_key: "test-key".into(),
            model: "test/model".into(),
        },
        conversation: ConversationConfig::default(),
        db: DatabaseConfig::default(),

        mcp_servers: vec![],
        heartbeat_interval_minutes: 90,
        bash_enabled: true,
        embedding: EmbeddingConfig::default(),
        chats: HashMap::new(),
        dms: HashMap::new(),
        dm_enabled: None,
        media_dir: default_media_dir(),
    }
}
