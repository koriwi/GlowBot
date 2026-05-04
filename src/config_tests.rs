use super::*;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn test_config_load_save_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let config = basic_config();
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.telegram_token, config.telegram_token);
    assert_eq!(loaded.openrouter.api_key, config.openrouter.api_key);
    assert_eq!(loaded.openrouter.model, config.openrouter.model);
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
    assert!(config.embedding.model.is_none());
    config.embedding.model = Some("openai/text-embedding-3-small".into());
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(
        loaded.embedding.model.as_deref(),
        Some("openai/text-embedding-3-small")
    );
}

#[test]
fn test_embedding_search_limit_default() {
    let config = basic_config();
    assert_eq!(config.embedding.search_limit, 1000);
}

#[test]
fn test_embedding_config_defaults() {
    let ec = EmbeddingConfig::default();
    assert!(ec.model.is_none());
    assert_eq!(ec.max_chars, 0);
    assert!(!ec.allow_split);
    assert_eq!(ec.search_limit, 1000);
}

#[test]
fn test_embedding_config_serialization() {
    let ec = EmbeddingConfig {
        model: Some("test-model".into()),
        max_chars: 500,
        allow_split: true,
        search_limit: 200,
    };
    let yaml = serde_yaml::to_string(&ec).unwrap();
    assert!(yaml.contains("test-model"));
    assert!(yaml.contains("max_chars"));
    let loaded: EmbeddingConfig = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(loaded.model.as_deref(), Some("test-model"));
    assert_eq!(loaded.max_chars, 500);
    assert!(loaded.allow_split);
    assert_eq!(loaded.search_limit, 200);
}

#[test]
fn test_embedding_search_limit_serialization() {
    let mut config = basic_config();
    config.embedding.search_limit = 500;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.embedding.search_limit, 500);
}

// --- ConversationConfig tests ---

#[test]
fn test_conversation_config_default() {
    let conv = ConversationConfig::default();
    assert_eq!(conv.recent_messages_window_size, 20);
    assert!(!conv.include_reasoning);
}

#[test]
fn test_conversation_config_serialization() {
    let conv = ConversationConfig {
        recent_messages_window_size: 50,
        include_reasoning: true,
    };
    let yaml = serde_yaml::to_string(&conv).unwrap();
    assert!(yaml.contains("50"));
    assert!(yaml.contains("include_reasoning"));
}

#[test]
fn test_config_load_save_with_conversation_config() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let mut config = basic_config();
    config.conversation.recent_messages_window_size = 30;
    config.conversation.include_reasoning = true;
    config.db.store_reasoning = true;
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.conversation.recent_messages_window_size, 30);
    assert!(loaded.conversation.include_reasoning);
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
