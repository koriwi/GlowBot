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

