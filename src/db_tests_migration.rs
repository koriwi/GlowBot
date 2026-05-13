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

