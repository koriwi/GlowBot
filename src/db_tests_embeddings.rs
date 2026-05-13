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

