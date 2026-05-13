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

