// ─── embedding dispatch tests ───────────────────────────────

#[tokio::test]
async fn test_dispatch_search_conversations_no_model() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = crate::config::basic_config(); // no embedding_model
    cfg.save(&data_dir.join("config.yaml")).unwrap();
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: data_dir.clone(),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            provider_overrides: HashMap::new(),
            picker_providers: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": "hello"}),
        None,
    )
    .await;
    assert!(out.contains("not configured"));
}

#[tokio::test]
async fn test_dispatch_search_conversations_empty_query() {
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.embedding_model = Some("test-embed-model".into());
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: std::path::PathBuf::from("/tmp"),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            provider_overrides: HashMap::new(),
            picker_providers: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": ""}),
        None,
    )
    .await;
    assert_eq!(out, "Error: query required");
}

#[tokio::test]
async fn test_dispatch_search_conversations_no_results() {
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.embedding_model = Some("test-embed-model".into());
    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: Arc::new(MockLlmBackend::new()),
        data_dir: std::path::PathBuf::from("/tmp"),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            provider_overrides: HashMap::new(),
            picker_providers: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": "nothing here"}),
        None,
    )
    .await;
    assert_eq!(out, "No similar messages found.");
}

#[tokio::test]
async fn test_dispatch_search_conversations_with_results() {
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.embedding_model = Some("test-embed-model".into());
    cfg.embedding.search_limit = 5;

    let db = crate::db::Database::open_in_memory().unwrap();
    // Store a message and its embedding
    let ids = db
        .save_messages(
            "-123",
            &[ChatMessage::user("Alice talked about Rust programming")],
        )
        .unwrap();
    db.save_embedding(ids[0], &[1.0f32, 0.0, 0.0, 0.0], "test-embed-model")
        .unwrap();

    // Mock LLM returns a matching embedding query
    let mock_llm = Arc::new(MockLlmBackend::new());
    mock_llm.add_embedding(vec![1.0f32, 0.0, 0.0, 0.0]);

    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: mock_llm,
        data_dir: std::path::PathBuf::from("/tmp"),
        db,
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            provider_overrides: HashMap::new(),
            picker_providers: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": "rust"}),
        None,
    )
    .await;
    assert!(out.contains("similarity"));
    assert!(out.contains("Rust"));
}

#[tokio::test]
async fn test_dispatch_search_conversations_embedding_error() {
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.embedding_model = Some("test-embed-model".into());

    let mock_llm = Arc::new(MockLlmBackend::new());
    mock_llm.set_error(true);

    let state = Arc::new(Mutex::new(BotState {
        config: cfg,
        skills: HashMap::new(),
        llm: mock_llm,
        data_dir: std::path::PathBuf::from("/tmp"),
        db: crate::db::Database::open_in_memory().unwrap(),
        mcp_tools: vec![],
        model_metadata: HashMap::new(),
            model_order: Vec::new(),
        last_usage: HashMap::new(),
            pending_config_changes: HashMap::new(),
            pending_model_changes: HashMap::new(),
            model_overrides: HashMap::new(),
            provider_overrides: HashMap::new(),
            picker_providers: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "search_conversations",
        &serde_json::json!({"query": "test"}),
        None,
    )
    .await;
    assert!(out.contains("Error embedding"));
}

// ─── generate_image dispatch tests ──────────────────────────────────

#[tokio::test]
async fn test_dispatch_generate_image_no_model() {
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
            provider_overrides: HashMap::new(),
            picker_providers: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "generate_image",
        &serde_json::json!({"prompt": "a cat"}),
        None,
    )
    .await;
    assert!(out.contains("image generation model not configured"));
}

#[tokio::test]
async fn test_dispatch_generate_image_empty_prompt() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut cfg = crate::config::basic_config();
    cfg.openrouter.image_gen_model = Some("test/image-model".into());
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
            provider_overrides: HashMap::new(),
            picker_providers: HashMap::new(),
            last_browse_cb: HashMap::new(),
            _mcp_services: vec![], mcp_peers: HashMap::new(),
    }));
    let out = dispatch_tool(
        &state,
        "-123",
        "generate_image",
        &serde_json::json!({}),
        None,
    )
    .await;
    assert!(out.contains("prompt required"));
}

// ─── generate_image helper function tests ──────────────────────────

#[test]
fn test_base64_decode_standard() {
    let decoded =
        super::bot_dispatch::bot_dispatch_image::tests::base64_decode_for_test("aGVsbG8=").unwrap();
    assert_eq!(decoded, b"hello");
}

#[test]
fn test_base64_decode_with_data_url_prefix() {
    let decoded = super::bot_dispatch::bot_dispatch_image::tests::base64_decode_for_test(
        "data:image/png;base64,aGVsbG8=",
    )
    .unwrap();
    assert_eq!(decoded, b"hello");
}

#[test]
fn test_base64_decode_invalid() {
    let result = super::bot_dispatch::bot_dispatch_image::tests::base64_decode_for_test(
        "!!!not-base64!!!",
    );
    assert!(result.is_err());
}

#[test]
fn test_detect_image_format_png() {
    let data = [0x89, 0x50, 0x4E, 0x47, 0x00, 0x00];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        Some("png")
    );
}

#[test]
fn test_detect_image_format_jpg() {
    let data = [0xFF, 0xD8, 0xFF, 0x00, 0x00];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        Some("jpg")
    );
}

#[test]
fn test_detect_image_format_webp() {
    let mut data = vec![b'R', b'I', b'F', b'F', 0x00, 0x00, 0x00, 0x00];
    data.extend_from_slice(b"WEBP");
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        Some("webp")
    );
}

#[test]
fn test_detect_image_format_gif() {
    let data = [b'G', b'I', b'F', b'8', b'9', b'a'];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        Some("gif")
    );
}

#[test]
fn test_detect_image_format_unknown() {
    let data = [0x00, 0x00, 0x00, 0x00];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::detect_image_format_for_test(&data),
        None
    );
}

#[test]
fn test_image_gen_model_id() {
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::image_gen_model_id_for_test(
            "provider/model"
        ),
        "model"
    );
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::image_gen_model_id_for_test(
            "black-forest-labs/flux-1.1-pro",
        ),
        "flux-1_1-pro"
    );
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::image_gen_model_id_for_test("simple"),
        "simple"
    );
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::image_gen_model_id_for_test(
            "provider/model:route",
        ),
        "model_route"
    );
}

#[test]
fn test_guess_mime_png() {
    let data = [0x89, 0x50, 0x4E, 0x47];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::guess_mime_for_test(&data),
        "image/png"
    );
}

#[test]
fn test_guess_mime_jpeg() {
    let data = [0xFF, 0xD8, 0xFF];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::guess_mime_for_test(&data),
        "image/jpeg"
    );
}

#[test]
fn test_guess_mime_default() {
    let data = [0x00, 0x00, 0x00];
    assert_eq!(
        super::bot_dispatch::bot_dispatch_image::tests::guess_mime_for_test(&data),
        "image/png"
    );
}

