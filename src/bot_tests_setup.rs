#[tokio::test]
async fn test_bot_creation() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    crate::config::basic_config()
        .save(&data_dir.join("config.yaml"))
        .unwrap();

    let mock_llm = Arc::new(MockLlmBackend::new());
    let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
    let state = bot.state.lock().await;
    assert_eq!(state.config.telegram_token, "test-token");
}

#[tokio::test]
async fn test_bot_creation_nonexistent_config() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("glowbot_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mock_llm = Arc::new(MockLlmBackend::new());
    let result = GlowBot::new_with_llm(&data_dir, mock_llm).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ensure_memory_exists() {
    let (bot, _dir, _mock) = setup_test_bot().await;
    bot.ensure_memory_exists("-123", "456", "@testuser")
        .await
        .unwrap();

    let state = bot.state.lock().await;
    let mem = crate::memory::load_memory(&state.chats_dir(), "-123", "456");
    assert!(mem.is_some());
    assert_eq!(mem.unwrap().frontmatter.username, "@testuser");
}

#[tokio::test]
async fn test_reload_skills() {
    let (bot, dir, _mock) = setup_test_bot().await;

    use crate::skills::{write_skill, SkillFrontmatter};
    let skills_dir = dir.path().join("glowbot_data").join("skills");
    let fm = SkillFrontmatter {
        name: "test-skill".into(),
        description: "A test".into(),
    };
    write_skill(&skills_dir, "test-skill", &fm, "body text").unwrap();

    bot.reload_skills().await.unwrap();
    let state = bot.state.lock().await;
    assert!(state.skills.contains_key("test-skill"));
}

