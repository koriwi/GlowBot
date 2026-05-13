// ---------- dispatch_tool edge-case tests ----------

#[tokio::test]
async fn test_dispatch_send_message_empty_text() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_message",
        &serde_json::json!({"text":""}),
        None,
    )
    .await;
    assert_eq!(out, "Error: text required");
}

#[tokio::test]
async fn test_dispatch_send_message_no_tg_bot() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_message",
        &serde_json::json!({"text":"hi"}),
        None,
    )
    .await;
    assert_eq!(out, "Error: send_message not available in this context.");
}

#[tokio::test]
async fn test_dispatch_send_media_empty_file_path() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_media",
        &serde_json::json!({"file_path":""}),
        None,
    )
    .await;
    assert_eq!(out, "Error: file_path required");
}

#[tokio::test]
async fn test_dispatch_send_media_file_not_found() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_media",
        &serde_json::json!({"file_path":"nonexistent.png"}),
        None,
    )
    .await;
    assert!(out.starts_with("Error: file not found:"));
}

#[tokio::test]
async fn test_dispatch_send_media_no_tg_bot() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    // Create a dummy file so the file-exists check passes before the tg_bot check
    std::fs::write(data_dir.join("test.png"), b"fake png").unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_media",
        &serde_json::json!({"file_path":"test.png"}),
        None,
    )
    .await;
    assert_eq!(out, "Error: send_media not available in this context.");
}

#[tokio::test]
async fn test_dispatch_send_media_original_quality() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("test.png"), b"fake png").unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "send_media",
        &serde_json::json!({"file_path":"test.png", "original_quality": true}),
        None,
    )
    .await;
    // Without a tg_bot, still returns not available, but param is parsed correctly
    assert_eq!(out, "Error: send_media not available in this context.");
}

// ─── list_media dispatch tests ─────────────────────────────────────

fn setup_state_with_media_dir(media_dir: &std::path::Path) -> Arc<Mutex<BotState>> {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: Config {
            media_dir: media_dir.to_string_lossy().to_string(),
            ..crate::config::basic_config()
        },
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    state
}

#[tokio::test]
async fn test_dispatch_list_media_root_empty() {
    let media_dir = TempDir::new().unwrap();
    let state = setup_state_with_media_dir(media_dir.path());
    let out = dispatch_tool(&state, "-123", "list_media", &serde_json::json!({}), None).await;
    assert!(out.starts_with("Media directory listing for"));
    assert!(out.contains("(empty)"));
}

#[tokio::test]
async fn test_dispatch_list_media_with_files_and_dirs() {
    let media_dir = TempDir::new().unwrap();
    let media_path = media_dir.path();

    // Create some files
    std::fs::write(media_path.join("photo.jpg"), b"fake jpeg").unwrap();
    std::fs::write(media_path.join("notes.txt"), b"hello").unwrap();
    // Create a subdirectory with files
    std::fs::create_dir_all(media_path.join("images")).unwrap();
    std::fs::write(media_path.join("images/cat.png"), b"cat png").unwrap();
    std::fs::write(media_path.join("images/dog.jpg"), b"dog jpg").unwrap();
    // Another subdirectory (empty)
    std::fs::create_dir_all(media_path.join("videos")).unwrap();

    let state = setup_state_with_media_dir(media_path);
    let out = dispatch_tool(&state, "-123", "list_media", &serde_json::json!({}), None).await;
    assert!(out.starts_with("Media directory listing for"));
    assert!(out.contains("photo.jpg"));
    assert!(out.contains("notes.txt"));
    assert!(out.contains("images/"));
    assert!(out.contains("videos/"));
    assert!(out.contains("cat.png"));
    assert!(out.contains("dog.jpg"));
    // Verify size formatting
    assert!(out.contains("9 B")); // "fake jpeg" is 9 bytes
}

#[tokio::test]
async fn test_dispatch_list_media_subpath() {
    let media_dir = TempDir::new().unwrap();
    let media_path = media_dir.path();
    std::fs::create_dir_all(media_path.join("images/cats")).unwrap();
    std::fs::write(media_path.join("images/cats/fluffy.jpg"), b"meow").unwrap();
    std::fs::write(media_path.join("images/logo.png"), b"logo").unwrap();
    std::fs::write(media_path.join("root.txt"), b"top").unwrap();

    let state = setup_state_with_media_dir(media_path);
    let out = dispatch_tool(
        &state,
        "-123",
        "list_media",
        &serde_json::json!({"subpath": "images"}),
        None,
    )
    .await;
    assert!(out.contains("cats/"));
    assert!(out.contains("fluffy.jpg"));
    assert!(out.contains("logo.png"));
    assert!(
        !out.contains("root.txt"),
        "subpath should not show root files"
    );
}

#[tokio::test]
async fn test_dispatch_list_media_subpath_not_found() {
    let media_dir = TempDir::new().unwrap();
    let state = setup_state_with_media_dir(media_dir.path());
    let out = dispatch_tool(
        &state,
        "-123",
        "list_media",
        &serde_json::json!({"subpath": "nonexistent"}),
        None,
    )
    .await;
    assert!(out.starts_with("Error: directory not found:"));
}

#[tokio::test]
async fn test_dispatch_list_media_subpath_is_file() {
    let media_dir = TempDir::new().unwrap();
    let media_path = media_dir.path();
    std::fs::write(media_path.join("readme.md"), b"# media").unwrap();

    let state = setup_state_with_media_dir(media_path);
    let out = dispatch_tool(
        &state,
        "-123",
        "list_media",
        &serde_json::json!({"subpath": "readme.md"}),
        None,
    )
    .await;
    assert!(out.contains("is a file, not a directory"));
}

#[tokio::test]
async fn test_dispatch_list_media_path_traversal_blocked() {
    let media_dir = TempDir::new().unwrap();
    let state = setup_state_with_media_dir(media_dir.path());
    // Try to escape with ..
    let out = dispatch_tool(
        &state,
        "-123",
        "list_media",
        &serde_json::json!({"subpath": ".."}),
        None,
    )
    .await;
    assert!(out.starts_with("Error: invalid subpath"));
}

#[tokio::test]
async fn test_dispatch_list_media_root_missing() {
    let state = setup_state_with_media_dir(std::path::Path::new("/nonexistent/media/dir"));
    let out = dispatch_tool(&state, "-123", "list_media", &serde_json::json!({}), None).await;
    assert!(out.starts_with("Error: directory not found:"));
}

#[tokio::test]
async fn test_dispatch_bash_empty_command() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "bash",
        &serde_json::json!({"command":""}),
        None,
    )
    .await;
    assert!(out.contains("exit code"));
}

#[tokio::test]
async fn test_dispatch_bash_disabled() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut cfg = crate::config::basic_config();
    cfg.bash_enabled = false;
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "bash",
        &serde_json::json!({"command":"echo hi"}),
        None,
    )
    .await;
    assert!(
        out.contains("disabled"),
        "expected disabled message, got: {}",
        out
    );
}

#[tokio::test]
async fn test_dispatch_read_memory_missing() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "read_memory",
        &serde_json::json!({"user_id":"999"}),
        None,
    )
    .await;
    assert!(out.contains("No memory file found"));
}

#[tokio::test]
async fn test_dispatch_update_memory_no_fields() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config();
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir,
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            last_browse_cb: HashMap::new(),
            mcp_server_locks: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "update_memory",
        &serde_json::json!({"user_id":"999"}),
        None,
    )
    .await;
    assert_eq!(out, "No fields to update.");
}

