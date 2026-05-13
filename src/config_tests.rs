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
fn test_chat_config_serialization() {
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
fn test_missing_model_is_error() {
    let yaml = r#"
telegram_token: "test-token"
openrouter:
  api_key: "test-key"
"#;
    let result: Result<Config, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err());
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
    config.dms.insert(
        "42".into(),
        DmConfig {
            commands_enabled: true,
            ..Default::default()
        },
    );
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert!(loaded.dms.contains_key("42"));
    assert!(loaded.dms.get("42").unwrap().commands_enabled);
}

#[test]
fn test_embedding_model_serialization() {
    let mut config = basic_config();
    assert!(config.openrouter.embedding_model.is_none());
    config.openrouter.embedding_model = Some("openai/text-embedding-3-small".into());
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(
        loaded.openrouter.embedding_model.as_deref(),
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
    assert_eq!(ec.max_chars, 0);
    assert!(!ec.allow_split);
    assert_eq!(ec.search_limit, 1000);
}

#[test]
fn test_embedding_config_serialization() {
    let ec = EmbeddingConfig {
        max_chars: 500,
        allow_split: true,
        search_limit: 200,
    };
    let yaml = serde_yaml::to_string(&ec).unwrap();
    assert!(yaml.contains("max_chars"));
    let loaded: EmbeddingConfig = serde_yaml::from_str(&yaml).unwrap();
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
        ..Default::default()
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

#[test]
fn test_is_mcp_server_allowed_default() {
    let config = basic_config();
    // No blacklist set — all servers allowed for group chats
    assert!(config.is_mcp_server_allowed("-123", "homeassistant"));
    assert!(config.is_mcp_server_allowed("-456", "anything"));
}

#[test]
fn test_is_mcp_server_allowed_blacklisted() {
    let mut config = basic_config();
    config.chats.insert(
        "-123".into(),
        ChatConfig {
            mcp_blacklist: vec!["homeassistant".into()],
            ..Default::default()
        },
    );
    assert!(!config.is_mcp_server_allowed("-123", "homeassistant"));
    // Other servers still allowed
    assert!(config.is_mcp_server_allowed("-123", "download-server"));
    // Other chats not affected
    assert!(config.is_mcp_server_allowed("-456", "homeassistant"));
}

#[test]
fn test_is_mcp_server_allowed_dm_always_allowed() {
    let mut config = basic_config();
    config.chats.insert(
        "12345".into(),
        ChatConfig {
            mcp_blacklist: vec!["homeassistant".into()],
            ..Default::default()
        },
    );
    // DM chats (positive chat_id) ignore the blacklist
    assert!(config.is_mcp_server_allowed("12345", "homeassistant"));
}

#[test]
fn test_image_gen_model_serialization() {
    let mut config = basic_config();
    assert!(config.openrouter.image_gen_model.is_none());
    config.openrouter.image_gen_model = Some("black-forest-labs/flux-1.1-pro".into());
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(
        loaded.openrouter.image_gen_model.as_deref(),
        Some("black-forest-labs/flux-1.1-pro")
    );
}

#[test]
fn test_heartbeat_recent_messages_window_size_default() {
    let config = basic_config();
    // None → falls back to recent_messages_window_size (20)
    assert_eq!(config.heartbeat_recent_messages_window_size(), 20);
    assert_eq!(
        config.conversation.heartbeat_recent_messages_window_size,
        None
    );
}

#[test]
fn test_heartbeat_recent_messages_window_size_set() {
    let mut config = basic_config();
    config.conversation.heartbeat_recent_messages_window_size = Some(5);
    assert_eq!(config.heartbeat_recent_messages_window_size(), 5);
}

#[test]
fn test_heartbeat_recent_messages_window_size_set_to_zero() {
    let mut config = basic_config();
    config.conversation.heartbeat_recent_messages_window_size = Some(0);
    assert_eq!(config.heartbeat_recent_messages_window_size(), 0);
}

#[test]
fn test_heartbeat_recent_messages_window_size_serialization() {
    let conv = ConversationConfig {
        heartbeat_recent_messages_window_size: Some(10),
        ..Default::default()
    };
    let yaml = serde_yaml::to_string(&conv).unwrap();
    assert!(yaml.contains("heartbeat_recent_messages_window_size"));
    let loaded: ConversationConfig = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(loaded.heartbeat_recent_messages_window_size, Some(10));
}

#[test]
fn test_heartbeat_recent_messages_window_size_none_serialization() {
    let conv = ConversationConfig::default();
    let yaml = serde_yaml::to_string(&conv).unwrap();
    // None should be skipped
    assert!(!yaml.contains("heartbeat_recent_messages_window_size"));
}

// --- image_fallback_model_for_chat tests ---

#[test]
fn test_image_fallback_model_for_group_no_override() {
    let mut config = basic_config();
    config.openrouter.image_fallback_model = Some("global-img-fb".into());
    assert_eq!(
        config.image_fallback_model_for_chat("-123"),
        Some("global-img-fb")
    );
}

#[test]
fn test_image_fallback_model_for_group_override() {
    let mut config = basic_config();
    config.openrouter.image_fallback_model = Some("global-img-fb".into());
    config.chats.insert(
        "-123".into(),
        ChatConfig {
            image_fallback_model: Some("chat-img-fb".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        config.image_fallback_model_for_chat("-123"),
        Some("chat-img-fb")
    );
    // Other chat uses global
    assert_eq!(
        config.image_fallback_model_for_chat("-999"),
        Some("global-img-fb")
    );
}

#[test]
fn test_image_fallback_model_for_dm_no_override() {
    let mut config = basic_config();
    config.openrouter.image_fallback_model = Some("global-img-fb".into());
    // DM without config entry → uses global
    assert_eq!(
        config.image_fallback_model_for_chat("123"),
        Some("global-img-fb")
    );
}

#[test]
fn test_image_fallback_model_for_dm_override() {
    let mut config = basic_config();
    config.openrouter.image_fallback_model = Some("global-img-fb".into());
    config.dms.insert(
        "123".into(),
        DmConfig {
            image_fallback_model: Some("dm-img-fb".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        config.image_fallback_model_for_chat("123"),
        Some("dm-img-fb")
    );
}

#[test]
fn test_image_fallback_model_none_global() {
    let config = basic_config();
    assert_eq!(config.image_fallback_model_for_chat("-123"), None);
    assert_eq!(config.image_fallback_model_for_chat("456"), None);
}

// --- audio_fallback_model_for_chat tests ---

#[test]
fn test_audio_fallback_model_for_group_no_override() {
    let mut config = basic_config();
    config.openrouter.audio_fallback_model = Some("global-audio-fb".into());
    assert_eq!(
        config.audio_fallback_model_for_chat("-123"),
        Some("global-audio-fb")
    );
}

#[test]
fn test_audio_fallback_model_for_group_override() {
    let mut config = basic_config();
    config.openrouter.audio_fallback_model = Some("global-audio-fb".into());
    config.chats.insert(
        "-123".into(),
        ChatConfig {
            audio_fallback_model: Some("chat-audio-fb".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        config.audio_fallback_model_for_chat("-123"),
        Some("chat-audio-fb")
    );
    assert_eq!(
        config.audio_fallback_model_for_chat("-999"),
        Some("global-audio-fb")
    );
}

#[test]
fn test_audio_fallback_model_for_dm_override() {
    let mut config = basic_config();
    config.openrouter.audio_fallback_model = Some("global-audio-fb".into());
    config.dms.insert(
        "123".into(),
        DmConfig {
            audio_fallback_model: Some("dm-audio-fb".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        config.audio_fallback_model_for_chat("123"),
        Some("dm-audio-fb")
    );
}

#[test]
fn test_audio_fallback_model_none_global() {
    let config = basic_config();
    assert_eq!(config.audio_fallback_model_for_chat("-123"), None);
}

// --- image_gen_model_for_chat tests ---

#[test]
fn test_image_gen_model_for_group_no_override() {
    let mut config = basic_config();
    config.openrouter.image_gen_model = Some("global-img-gen".into());
    assert_eq!(
        config.image_gen_model_for_chat("-123"),
        Some("global-img-gen")
    );
}

#[test]
fn test_image_gen_model_for_group_override() {
    let mut config = basic_config();
    config.openrouter.image_gen_model = Some("global-img-gen".into());
    config.chats.insert(
        "-123".into(),
        ChatConfig {
            image_gen_model: Some("chat-img-gen".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        config.image_gen_model_for_chat("-123"),
        Some("chat-img-gen")
    );
    assert_eq!(
        config.image_gen_model_for_chat("-999"),
        Some("global-img-gen")
    );
}

#[test]
fn test_image_gen_model_for_dm_override() {
    let mut config = basic_config();
    config.openrouter.image_gen_model = Some("global-img-gen".into());
    config.dms.insert(
        "123".into(),
        DmConfig {
            image_gen_model: Some("dm-img-gen".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        config.image_gen_model_for_chat("123"),
        Some("dm-img-gen")
    );
}

#[test]
fn test_image_gen_model_none_global() {
    let config = basic_config();
    assert_eq!(config.image_gen_model_for_chat("-123"), None);
    assert_eq!(config.image_gen_model_for_chat("456"), None);
}

#[test]
fn test_mcp_blacklist_serialization() {
    let chat = ChatConfig {
        mcp_blacklist: vec!["homeassistant".into(), "download-srv".into()],
        ..Default::default()
    };
    let yaml = serde_yaml::to_string(&chat).unwrap();
    assert!(yaml.contains("homeassistant"));
    assert!(yaml.contains("download-srv"));
    let loaded: ChatConfig = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(loaded.mcp_blacklist, vec!["homeassistant", "download-srv"]);
}

// --- advice_model tests ---

#[test]
fn test_advice_model_default_none() {
    let config = basic_config();
    assert!(config.openrouter.advice_model.is_none());
}

#[test]
fn test_advice_model_openrouter_serialization() {
    let mut config = basic_config();
    config.openrouter.advice_model = Some("openai/gpt-4o".into());
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.openrouter.advice_model.as_deref(), Some("openai/gpt-4o"));
}

#[test]
fn test_advice_model_for_group_no_override() {
    let mut config = basic_config();
    config.openrouter.advice_model = Some("global-advice".into());
    assert_eq!(config.advice_model_for_chat("-123"), Some("global-advice"));
}

#[test]
fn test_advice_model_for_group_override() {
    let mut config = basic_config();
    config.openrouter.advice_model = Some("global-advice".into());
    config.chats.insert(
        "-123".into(),
        ChatConfig {
            advice_model: Some("chat-advice".into()),
            ..Default::default()
        },
    );
    assert_eq!(config.advice_model_for_chat("-123"), Some("chat-advice"));
    assert_eq!(config.advice_model_for_chat("-999"), Some("global-advice"));
}

#[test]
fn test_advice_model_for_dm_override() {
    let mut config = basic_config();
    config.openrouter.advice_model = Some("global-advice".into());
    config.dms.insert(
        "123".into(),
        DmConfig {
            advice_model: Some("dm-advice".into()),
            ..Default::default()
        },
    );
    assert_eq!(config.advice_model_for_chat("123"), Some("dm-advice"));
}

#[test]
fn test_advice_model_for_dm_no_override() {
    let mut config = basic_config();
    config.openrouter.advice_model = Some("global-advice".into());
    // DM without entry → uses global
    assert_eq!(config.advice_model_for_chat("456"), Some("global-advice"));
}

#[test]
fn test_advice_model_none_global() {
    let config = basic_config();
    assert_eq!(config.advice_model_for_chat("-123"), None);
    assert_eq!(config.advice_model_for_chat("456"), None);
}

#[test]
fn test_conversation_advice_config_defaults() {
    let conv = ConversationConfig::default();
    assert_eq!(conv.advice_recent_messages_window_size, 5);
    assert!(!conv.advice_include_reasoning);
}

#[test]
fn test_conversation_advice_config_serialization() {
    let conv = ConversationConfig {
        advice_recent_messages_window_size: 10,
        advice_include_reasoning: true,
        ..Default::default()
    };
    let yaml = serde_yaml::to_string(&conv).unwrap();
    assert!(yaml.contains("advice_recent_messages_window_size"));
    assert!(yaml.contains("advice_include_reasoning"));
    let loaded: ConversationConfig = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(loaded.advice_recent_messages_window_size, 10);
    assert!(loaded.advice_include_reasoning);
}

#[test]
fn test_advice_model_chat_config_serialization() {
    let chat = ChatConfig {
        advice_model: Some("chat-advice-model".into()),
        ..Default::default()
    };
    let yaml = serde_yaml::to_string(&chat).unwrap();
    assert!(yaml.contains("chat-advice-model"));
    let loaded: ChatConfig = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(loaded.advice_model.as_deref(), Some("chat-advice-model"));
}

#[test]
fn test_advice_model_dm_config_serialization() {
    let dm = DmConfig {
        advice_model: Some("dm-advice-model".into()),
        ..Default::default()
    };
    let yaml = serde_yaml::to_string(&dm).unwrap();
    assert!(yaml.contains("dm-advice-model"));
    let loaded: DmConfig = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(loaded.advice_model.as_deref(), Some("dm-advice-model"));
}
