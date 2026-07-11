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
fn test_codex_config_load_and_provider_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        r#"
telegram_token: test-token
provider: codex
codex:
  model: gpt-5.4
  auth_file: /tmp/codex-auth.json
  reasoning_effort: high
"#,
    )
    .unwrap();
    let config = Config::load(&path).unwrap();
    assert_eq!(config.provider, LlmProvider::Codex);
    assert_eq!(config.default_model(), "gpt-5.4");
    assert!(!config.uses_openrouter());
    assert_eq!(
        config.codex.as_ref().unwrap().base_url,
        "https://chatgpt.com/backend-api"
    );
    assert!(config.openrouter.api_key.is_empty());

    let redacted = config.redacted();
    assert_eq!(redacted.codex.unwrap().auth_file, "[REDACTED]");
}

#[test]
fn test_provider_validation_errors() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "telegram_token: test\nprovider: codex\n").unwrap();
    assert!(Config::load(&path).unwrap_err().to_string().contains("codex configuration"));

    std::fs::write(
        &path,
        "telegram_token: test\nprovider: openrouter\nopenrouter: {model: test, api_key: key}\nchats: {'-1': {provider: codex}}\n",
    )
    .unwrap();
    assert!(Config::load(&path)
        .unwrap_err()
        .to_string()
        .contains("any chat uses provider codex"));

    std::fs::write(
        &path,
        "telegram_token: test\nprovider: openrouter\nopenrouter: {model: test, api_key: ''}\n",
    )
    .unwrap();
    assert!(Config::load(&path).unwrap_err().to_string().contains("api_key"));

    std::fs::write(
        &path,
        "telegram_token: test\nprovider: codex\ncodex: {model: '', auth_file: auth.json}\n",
    )
    .unwrap();
    assert!(Config::load(&path).unwrap_err().to_string().contains("codex.model"));

    std::fs::write(
        &path,
        "telegram_token: test\nprovider: codex\ncodex: {model: gpt-5.4}\nopenrouter: {api_key: '', model: '', embedding_model: embed}\n",
    )
    .unwrap();
    assert!(Config::load(&path)
        .unwrap_err()
        .to_string()
        .contains("openrouter.api_key"));
}

#[test]
fn test_openrouter_is_default_provider() {
    let config = basic_config();
    assert_eq!(config.provider, LlmProvider::Openrouter);
    assert!(config.uses_openrouter());
    assert_eq!(config.default_model(), "test/model");
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
fn test_provider_override_uses_provider_default_model() {
    let mut config = basic_config();
    config.openrouter.advice_model = Some("openai/advisor".into());
    config.codex = Some(CodexConfig {
        model: "gpt-5.4".into(),
        auth_file: "auth.json".into(),
        reasoning_effort: None,
        base_url: "https://chatgpt.com/backend-api".into(),
    });
    config.chats.insert(
        "-123".into(),
        ChatConfig {
            provider: Some(LlmProvider::Codex),
            ..Default::default()
        },
    );
    config.dms.insert(
        "456".into(),
        DmConfig {
            provider: Some(LlmProvider::Codex),
            model: Some("gpt-5.3-codex".into()),
            ..Default::default()
        },
    );

    assert_eq!(config.provider_for_chat("-123"), LlmProvider::Codex);
    assert_eq!(config.model_for_chat("-123"), "gpt-5.4");
    assert!(config.advice_model_for_chat("-123").is_none());
    assert_eq!(config.provider_for_chat("456"), LlmProvider::Codex);
    assert_eq!(config.model_for_chat("456"), "gpt-5.3-codex");
    assert_eq!(config.provider_for_chat("-999"), LlmProvider::Openrouter);
    config.validate().unwrap();
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
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        r#"
telegram_token: "test-token"
openrouter:
  api_key: "test-key"
"#,
    )
    .unwrap();
    let result = Config::load(&path);
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

