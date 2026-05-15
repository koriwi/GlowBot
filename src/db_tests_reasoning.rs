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
        .search_embeddings("-000", &[1.0, 0.0], "model-e", 10, 10)
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
