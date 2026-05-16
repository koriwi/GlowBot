// --- ConversationConfig tests ---

#[test]
fn test_conversation_config_default() {
    let conv = ConversationConfig::default();
    assert_eq!(conv.recent_messages_window_size, 20);
}

#[test]
fn test_conversation_config_serialization() {
    let conv = ConversationConfig {
        recent_messages_window_size: 50,
        ..Default::default()
    };
    let yaml = serde_yaml::to_string(&conv).unwrap();
    assert!(yaml.contains("50"));
}

#[test]
fn test_config_load_save_with_conversation_config() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let mut config = basic_config();
    config.conversation.recent_messages_window_size = 30;
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.conversation.recent_messages_window_size, 30);
}

#[test]
fn test_database_config_default() {
    let db = DatabaseConfig::default();
    assert_eq!(db.tool_max_content_len, None);
    assert_eq!(db.reasoning_max_content_len, None);
}

#[test]
fn test_database_config_serialization() {
    let db = DatabaseConfig {
        tool_max_content_len: Some(2048),
        reasoning_max_content_len: Some(4096),
    };
    let yaml = serde_yaml::to_string(&db).unwrap();
    assert!(yaml.contains("tool_max_content_len"));
    assert!(yaml.contains("reasoning_max_content_len"));
    let loaded: DatabaseConfig = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(loaded.tool_max_content_len, Some(2048));
    assert_eq!(loaded.reasoning_max_content_len, Some(4096));
}

#[test]
fn test_database_config_minimal_serialization() {
    // Defaults should not appear in serialized output
    let db = DatabaseConfig::default();
    let yaml = serde_yaml::to_string(&db).unwrap();
    assert!(!yaml.contains("tool_max_content_len"));
    assert!(!yaml.contains("reasoning_max_content_len"));
}

#[test]
fn test_config_load_save_with_db_config() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let mut config = basic_config();
    config.db.tool_max_content_len = Some(1000);
    config.db.reasoning_max_content_len = Some(500);
    config.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.db.tool_max_content_len, Some(1000));
    assert_eq!(loaded.db.reasoning_max_content_len, Some(500));
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

