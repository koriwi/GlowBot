use super::*;

// ─── migration tests (require sqldiff binary) ───────────────────

/// Helper: check if sqldiff is available on PATH.
fn has_sqldiff() -> bool {
    std::process::Command::new("sqldiff")
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Helper: write schema .sql files into a temp directory.
fn setup_schema_dir(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("messages.sql"),
        "CREATE TABLE messages (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            chat_id     TEXT    NOT NULL,
            role        TEXT    NOT NULL,
            content     TEXT    NOT NULL,
            name        TEXT,
            tool_calls  TEXT,
            tool_call_id TEXT,
            created_at  INTEGER NOT NULL,
            reasoning   TEXT
        );
        CREATE INDEX idx_messages_chat_created ON messages(chat_id, created_at);
        ",
    )
    .unwrap();
    std::fs::write(
        dir.join("message_embeddings.sql"),
        "CREATE TABLE message_embeddings (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            embedding   BLOB NOT NULL,
            model       TEXT NOT NULL,
            created_at  INTEGER NOT NULL
        );
        CREATE INDEX idx_embeddings_message ON message_embeddings(message_id);
        CREATE INDEX idx_embeddings_model_message ON message_embeddings(model, message_id);
        ",
    )
    .unwrap();
}

/// Simulate an old database that was created without the `reasoning` column.
/// sqldiff should generate ALTER TABLE ADD COLUMN to add it.
#[test]
fn test_migration_adds_reasoning_column() {
    if !has_sqldiff() {
        eprintln!("Skipping: sqldiff not installed");
        return;
    }

    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("glowbot.db");
    let schema_dir = dir.path().join("schema");
    setup_schema_dir(&schema_dir);

    // Create a v0 database manually (no reasoning column)
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id     TEXT    NOT NULL,
                role        TEXT    NOT NULL,
                content     TEXT    NOT NULL,
                name        TEXT,
                tool_calls  TEXT,
                tool_call_id TEXT,
                created_at  INTEGER NOT NULL
            )",
            [],
        )
        .unwrap();
        conn.close().unwrap();
    }

    // Now open with Database::new — it should migrate
    let db = Database::new(&db_path, &schema_dir).unwrap();

    // Save a message with reasoning and read it back
    let msg = ChatMessage::assistant_with_reasoning("answer", "thinking...".into());
    db.save_messages("-test", &[msg]).unwrap();

    let loaded = db.load_messages("-test", 10, None).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].reasoning.as_deref(), Some("thinking..."));
}

/// Verify that a second open doesn't break anything (idempotent migration).
#[test]
fn test_migration_idempotent() {
    if !has_sqldiff() {
        eprintln!("Skipping: sqldiff not installed");
        return;
    }

    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("glowbot.db");
    let schema_dir = dir.path().join("schema");
    setup_schema_dir(&schema_dir);

    // Open once — creates + migrates
    let db1 = Database::new(&db_path, &schema_dir).unwrap();
    drop(db1);

    // Open again — migration should be a no-op
    let db2 = Database::new(&db_path, &schema_dir).unwrap();

    let msg = ChatMessage::assistant_with_reasoning("x", "y".into());
    db2.save_messages("-test2", &[msg]).unwrap();
    let loaded = db2.load_messages("-test2", 10, None).unwrap();
    assert_eq!(loaded[0].reasoning.as_deref(), Some("y"));
}

fn make_db() -> Database {
    Database::open_in_memory().unwrap()
}

#[test]
fn test_roundtrip_messages() {
    let db = make_db();
    let chat_id = "-123";

    let msgs = vec![
        ChatMessage::user_with_name("Hello", "Alice"),
        ChatMessage::assistant("Hi Alice!"),
        ChatMessage::user_with_name("What's up?", "Alice"),
    ];
    db.save_messages(chat_id, &msgs).unwrap();

    let loaded = db.load_messages(chat_id, 10, None).unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].text_content(), "Hello");
    assert_eq!(loaded[1].text_content(), "Hi Alice!");
    assert_eq!(loaded[2].text_content(), "What's up?");
}

#[test]
fn test_window_limit() {
    let db = make_db();
    let chat_id = "-456";

    for i in 0..10 {
        db.save_messages(chat_id, &[ChatMessage::user(&format!("msg{i}"))])
            .unwrap();
    }

    let loaded = db.load_messages(chat_id, 3, None).unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].text_content(), "msg7");
    assert_eq!(loaded[1].text_content(), "msg8");
    assert_eq!(loaded[2].text_content(), "msg9");
}

#[test]
fn test_tool_calls_roundtrip() {
    use crate::openrouter::{FunctionCall, ToolCall};

    let db = make_db();
    let chat_id = "-789";

    let msg = ChatMessage::assistant_tool_calls(vec![ToolCall {
        id: "call_1".into(),
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: r#"{"command":"echo hi"}"#.into(),
        },
    }]);

    db.save_messages(chat_id, &[msg.clone()]).unwrap();
    let loaded = db.load_messages(chat_id, 10, None).unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].tool_calls.is_some());
    let tc = loaded[0].tool_calls.as_ref().unwrap();
    assert_eq!(tc[0].id, "call_1");
    assert_eq!(tc[0].function.name, "bash");
}

#[test]
fn test_empty_chat() {
    let db = make_db();
    let loaded = db.load_messages("nonexistent", 10, None).unwrap();
    assert!(loaded.is_empty());
}

#[test]
fn test_clear_messages() {
    let db = make_db();
    let chat_id = "-abc";

    db.save_messages(chat_id, &[ChatMessage::user("test")])
        .unwrap();
    assert_eq!(db.load_messages(chat_id, 10, None).unwrap().len(), 1);

    db.clear_messages(chat_id).unwrap();
    assert!(db.load_messages(chat_id, 10, None).unwrap().is_empty());
}

// ─── embedding tests ──────────────────────────────────────────

#[test]
fn test_pack_unpack_roundtrip() {
    let original = vec![1.0f32, -0.5, 0.25, 3.14];
    let blob = Database::pack_embedding(&original);
    assert_eq!(blob.len(), original.len() * 4);
    let unpacked = Database::unpack_embedding(&blob);
    assert_eq!(unpacked.len(), original.len());
    for (a, b) in original.iter().zip(unpacked.iter()) {
        assert!((a - b).abs() < 1e-6, "{} != {}", a, b);
    }
}

#[test]
fn test_pack_unpack_empty() {
    let blob = Database::pack_embedding(&[]);
    assert!(blob.is_empty());
    let unpacked = Database::unpack_embedding(&blob);
    assert!(unpacked.is_empty());
}

#[test]
fn test_save_and_search_embeddings() {
    let db = make_db();
    let chat_id = "-999";

    // Save a message first
    let msg = ChatMessage::user("Alice likes Rust programming");
    let ids = db.save_messages(chat_id, &[msg]).unwrap();
    assert_eq!(ids.len(), 1);

    // Create two simple 4-dim embeddings (mock)
    let emb1 = vec![1.0f32, 0.0, 0.0, 0.0]; // aligns with query
    let emb2 = vec![0.0f32, 1.0, 0.0, 0.0]; // orthogonal

    // Save another message
    let msg2 = ChatMessage::user("Bob enjoys Python");
    let ids2 = db.save_messages(chat_id, &[msg2]).unwrap();

    db.save_embedding(ids[0], &emb1, "test-embed-model")
        .unwrap();
    db.save_embedding(ids2[0], &emb2, "test-embed-model")
        .unwrap();

    // Search with query that matches emb1
    let query = vec![1.0f32, 0.0, 0.0, 0.0];
    let results = db
        .search_embeddings(chat_id, &query, "test-embed-model", 10)
        .unwrap();

    assert_eq!(results.len(), 2);
    // First result should be most similar
    assert!(
        (results[0].1 - 1.0).abs() < 0.01,
        "Expected ~1.0, got {}",
        results[0].1
    );
    assert!(results[0].2.contains("Rust"));
    // Second result should be ~0.0 (orthogonal)
    assert!(
        results[1].1.abs() < 0.01,
        "Expected ~0.0, got {}",
        results[1].1
    );
}

#[test]
fn test_search_embeddings_respects_limit() {
    let db = make_db();
    let chat_id = "-888";

    for i in 0..5 {
        let ids = db
            .save_messages(chat_id, &[ChatMessage::user(&format!("msg {i}"))])
            .unwrap();
        db.save_embedding(ids[0], &vec![i as f32, 0.0, 0.0, 0.0], "test-model")
            .unwrap();
    }

    let query = vec![0.0f32, 1.0, 0.0, 0.0];
    let results = db
        .search_embeddings(chat_id, &query, "test-model", 3)
        .unwrap();
    assert!(
        results.len() <= 3,
        "Should return at most 3, got {}",
        results.len()
    );
}

#[test]
fn test_search_embeddings_model_filter() {
    let db = make_db();
    let chat_id = "-777";

    let ids = db
        .save_messages(chat_id, &[ChatMessage::user("test")])
        .unwrap();
    db.save_embedding(ids[0], &[1.0, 0.0], "model-a").unwrap();

    // Search with different model should find nothing
    let results = db
        .search_embeddings(chat_id, &[1.0, 0.0], "model-b", 10)
        .unwrap();
    assert!(results.is_empty());

    // Search with correct model should find it
    let results = db
        .search_embeddings(chat_id, &[1.0, 0.0], "model-a", 10)
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_search_embeddings_empty_chat() {
    let db = make_db();
    let results = db
        .search_embeddings("nonexistent", &[1.0, 0.0], "any-model", 10)
        .unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_cleanup_mismatched_embeddings() {
    let db = make_db();
    let chat_id = "-666";

    let ids = db
        .save_messages(chat_id, &[ChatMessage::user("a"), ChatMessage::user("b")])
        .unwrap();
    db.save_embedding(ids[0], &[1.0], "old-model").unwrap();
    db.save_embedding(ids[1], &[1.0], "new-model").unwrap();

    let cleaned = db.cleanup_mismatched_embeddings("new-model").unwrap();
    assert_eq!(cleaned, 1);

    // Only new-model embedding should survive
    let results = db
        .search_embeddings(chat_id, &[1.0], "new-model", 10)
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_find_unembedded_messages() {
    let db = make_db();
    let chat_id = "-555";

    let ids = db
        .save_messages(
            chat_id,
            &[
                ChatMessage::user("alpha"),
                ChatMessage::user("beta"),
                ChatMessage::user("gamma"),
            ],
        )
        .unwrap();

    // Embed only the middle message
    db.save_embedding(ids[1], &[1.0], "any-model").unwrap();

    let unembedded = db.find_unembedded_messages().unwrap();
    assert_eq!(unembedded.len(), 2);

    let texts: Vec<&str> = unembedded.iter().map(|(_, t)| t.as_str()).collect();
    assert!(texts.contains(&"alpha"));
    assert!(texts.contains(&"gamma"));
    assert!(!texts.contains(&"beta"));
}

#[test]
fn test_find_unembedded_all_embedded() {
    let db = make_db();
    let ids = db
        .save_messages("-444", &[ChatMessage::user("only")])
        .unwrap();
    db.save_embedding(ids[0], &[1.0], "model").unwrap();

    let unembedded = db.find_unembedded_messages().unwrap();
    assert!(unembedded.is_empty());
}

#[test]
fn test_save_messages_returns_ids() {
    let db = make_db();
    let ids = db
        .save_messages("-333", &[ChatMessage::user("a"), ChatMessage::user("b")])
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids[0] > 0);
    assert!(ids[1] > ids[0]);
}

#[test]
fn test_search_embeddings_dimension_mismatch_skipped() {
    // If somehow a stored embedding has a different dimension than the query,
    // it should be skipped gracefully.
    let db = make_db();
    let ids = db
        .save_messages("-222", &[ChatMessage::user("test")])
        .unwrap();
    // Store a 2-dim vector
    db.save_embedding(ids[0], &[1.0, 0.5], "model-x").unwrap();

    // Search with a 4-dim query
    let results = db
        .search_embeddings("-222", &[1.0, 0.0, 0.0, 0.0], "model-x", 10)
        .unwrap();
    // Mismatched dimension should be skipped
    assert!(results.is_empty());
}

// ─── reasoning roundtrip test ─────────────────────────────────

#[test]
fn test_reasoning_roundtrip() {
    let db = make_db();
    let chat_id = "-reason";

    let msg = ChatMessage::assistant_with_reasoning("The answer is 42.", "Let me think...".into());
    db.save_messages(chat_id, &[msg]).unwrap();

    let loaded = db.load_messages(chat_id, 10, None).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].text_content(), "The answer is 42.");
    assert_eq!(loaded[0].reasoning.as_deref(), Some("Let me think..."));
}

#[test]
fn test_reasoning_null_roundtrip() {
    let db = make_db();
    let chat_id = "-noreason";

    let msg = ChatMessage::assistant("simple reply");
    db.save_messages(chat_id, &[msg]).unwrap();

    let loaded = db.load_messages(chat_id, 10, None).unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].reasoning.is_none());
}

#[test]
fn test_tool_calls_with_reasoning_roundtrip() {
    use crate::openrouter::{FunctionCall, ToolCall};

    let db = make_db();
    let chat_id = "-tool-reason";

    let msg = ChatMessage::assistant_tool_calls_with_reasoning(
        vec![ToolCall {
            id: "call_99".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "bash".into(),
                arguments: r#"{"command":"echo hi"}"#.into(),
            },
        }],
        "Considering using bash...".into(),
    );

    db.save_messages(chat_id, &[msg]).unwrap();
    let loaded = db.load_messages(chat_id, 10, None).unwrap();
    assert_eq!(loaded.len(), 1);
    let tc = loaded[0].tool_calls.as_ref().unwrap();
    assert_eq!(tc[0].id, "call_99");
    assert_eq!(
        loaded[0].reasoning.as_deref(),
        Some("Considering using bash...")
    );
}

#[test]
fn test_find_unembedded_skips_empty_text() {
    let db = make_db();
    // Assistant message with empty text (no content)
    let msg = ChatMessage::assistant("");
    let _ids = db.save_messages("-111", &[msg]).unwrap();

    let unembedded = db.find_unembedded_messages().unwrap();
    assert!(unembedded.is_empty());
}

#[test]
fn test_search_embedding_skips_empty_text() {
    let db = make_db();
    let ids = db
        .save_messages("-000", &[ChatMessage::assistant("")])
        .unwrap();
    db.save_embedding(ids[0], &[1.0, 0.0], "model-e").unwrap();

    let results = db
        .search_embeddings("-000", &[1.0, 0.0], "model-e", 10)
        .unwrap();
    // Empty text messages should be skipped
    assert!(results.is_empty());
}

// ─── cutoff tests ──────────────────────────────────────────

#[test]
fn test_set_and_get_cutoff() {
    let db = make_db();

    // Initially no cutoff
    assert!(db.get_cutoff("-chat").unwrap().is_none());

    // Set a cutoff
    db.set_cutoff("-chat", 1000).unwrap();
    assert_eq!(db.get_cutoff("-chat").unwrap(), Some(1000));

    // Overwrite
    db.set_cutoff("-chat", 2000).unwrap();
    assert_eq!(db.get_cutoff("-chat").unwrap(), Some(2000));
}

#[test]
fn test_load_messages_with_since_filters_correctly() {
    let db = make_db();
    let chat_id = "-ct";

    // Save 3 messages. They all get the same `created_at` timestamp,
    // so we can't rely on real timestamps. Instead, insert messages with
    // different timestamps via raw SQL.
    let msgs = vec![
        ChatMessage::user("msg1"),
        ChatMessage::user("msg2"),
        ChatMessage::user("msg3"),
    ];
    db.save_messages(chat_id, &msgs).unwrap();

    // Manually set created_at to distinct values so we can test filtering
    {
        let conn = db.lock_conn();
        conn.execute("UPDATE messages SET created_at = 100 WHERE rowid = 1", [])
            .unwrap();
        conn.execute("UPDATE messages SET created_at = 200 WHERE rowid = 2", [])
            .unwrap();
        conn.execute("UPDATE messages SET created_at = 300 WHERE rowid = 3", [])
            .unwrap();
    }

    // Load with since=150 — should get msg2 and msg3
    let loaded = db.load_messages(chat_id, 10, Some(150)).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].text_content(), "msg2");
    assert_eq!(loaded[1].text_content(), "msg3");

    // Load with since=250 — only msg3
    let loaded = db.load_messages(chat_id, 10, Some(250)).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].text_content(), "msg3");

    // Load with since=400 — nothing
    let loaded = db.load_messages(chat_id, 10, Some(400)).unwrap();
    assert!(loaded.is_empty());
}

#[test]
fn test_load_messages_without_since_returns_all() {
    let db = make_db();
    let chat_id = "-ns";

    let msgs = vec![
        ChatMessage::user("a"),
        ChatMessage::user("b"),
        ChatMessage::user("c"),
    ];
    db.save_messages(chat_id, &msgs).unwrap();

    let loaded = db.load_messages(chat_id, 10, None).unwrap();
    assert_eq!(loaded.len(), 3);
}

#[test]
fn test_load_messages_with_since_respects_limit() {
    let db = make_db();
    let chat_id = "-sl";

    for i in 0..5 {
        db.save_messages(chat_id, &[ChatMessage::user(&format!("msg{i}"))])
            .unwrap();
    }

    // All messages pass the filter (since=0), but limit should still apply
    let loaded = db.load_messages(chat_id, 3, Some(0)).unwrap();
    assert_eq!(loaded.len(), 3);
    // Should be the last 3 messages
    assert_eq!(loaded[0].text_content(), "msg2");
    assert_eq!(loaded[1].text_content(), "msg3");
    assert_eq!(loaded[2].text_content(), "msg4");
}
