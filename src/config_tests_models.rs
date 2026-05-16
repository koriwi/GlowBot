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
}

#[test]
fn test_conversation_advice_config_serialization() {
    let conv = ConversationConfig {
        advice_recent_messages_window_size: 10,
        ..Default::default()
    };
    let yaml = serde_yaml::to_string(&conv).unwrap();
    assert!(yaml.contains("advice_recent_messages_window_size"));
    let loaded: ConversationConfig = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(loaded.advice_recent_messages_window_size, 10);
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
