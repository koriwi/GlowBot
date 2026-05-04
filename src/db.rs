use crate::openrouter::{ChatContent, ChatMessage};
use anyhow::Context;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Persistent SQLite-backed conversation history.
/// One row per message; read with a sliding window via `conversation_window`.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open (or create) the database at the given path and initialise tables.
    pub fn new(db_path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open database: {}", db_path.display()))?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory database for tests.
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn init(conn: &Connection) -> anyhow::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
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
        .context("Failed to create messages table")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_chat_created
             ON messages(chat_id, created_at)",
            [],
        )
        .context("Failed to create messages index")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS message_embeddings (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                embedding   BLOB NOT NULL,
                model       TEXT NOT NULL,
                created_at  INTEGER NOT NULL
            )",
            [],
        )
        .context("Failed to create message_embeddings table")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_embeddings_message
             ON message_embeddings(message_id)",
            [],
        )
        .context("Failed to create message_embeddings index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_embeddings_model_message
             ON message_embeddings(model, message_id)",
            [],
        )
        .context("Failed to create message_embeddings model index")?;

        Ok(())
    }

    /// Load the most recent `limit` messages for a chat, ordered from oldest to newest.
    pub fn load_messages(&self, chat_id: &str, limit: usize) -> anyhow::Result<Vec<ChatMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT role, content, name, tool_calls, tool_call_id
             FROM messages
             WHERE chat_id = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;

        struct Raw {
            role: String,
            content_json: String,
            name: Option<String>,
            tool_calls_json: Option<String>,
            tool_call_id: Option<String>,
        }

        let rows = stmt.query_map(params![chat_id, limit as i64], |row| {
            Ok(Raw {
                role: row.get(0)?,
                content_json: row.get(1)?,
                name: row.get(2)?,
                tool_calls_json: row.get(3)?,
                tool_call_id: row.get(4)?,
            })
        })?;

        let mut msgs = Vec::with_capacity(limit.min(20));
        for row in rows {
            let raw = row?;
            let content: ChatContent =
                serde_json::from_str(&raw.content_json).with_context(|| {
                    format!("Failed to deserialize content for chat {}", chat_id)
                })?;
            let tool_calls = raw
                .tool_calls_json
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .with_context(|| {
                    format!("Failed to deserialize tool_calls for chat {}", chat_id)
                })?;
            msgs.push(ChatMessage {
                role: raw.role,
                content,
                name: raw.name,
                tool_calls,
                tool_call_id: raw.tool_call_id,
            });
        }
        // rows come back newest-first; reverse to chronological order
        msgs.reverse();
        Ok(msgs)
    }

    /// Insert a batch of messages in a single transaction.
    /// Returns the row IDs of the inserted messages.
    pub fn save_messages(
        &self,
        chat_id: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<Vec<i64>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().timestamp();

        let mut ids = Vec::with_capacity(messages.len());
        for msg in messages {
            let content_json =
                serde_json::to_string(&msg.content).context("Failed to serialize content")?;
            let tool_calls_json = msg
                .tool_calls
                .as_ref()
                .map(|tc| serde_json::to_string(tc))
                .transpose()
                .context("Failed to serialize tool_calls")?;

            tx.execute(
                "INSERT INTO messages
                 (chat_id, role, content, name, tool_calls, tool_call_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    chat_id,
                    &msg.role,
                    content_json,
                    msg.name.as_deref(),
                    tool_calls_json.as_deref(),
                    msg.tool_call_id.as_deref(),
                    now
                ],
            )?;
            ids.push(tx.last_insert_rowid());
        }

        tx.commit()?;
        Ok(ids)
    }

    /// Delete all messages for a given chat (e.g. on `/clear`).
    pub fn clear_messages(&self, chat_id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM messages WHERE chat_id = ?1",
            params![chat_id],
        )?;
        Ok(())
    }

    // ─── embedding helpers ────────────────────────────────────────────

    /// Pack a slice of f32 values into a little-endian byte blob.
    pub fn pack_embedding(embedding: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(embedding.len() * 4);
        for &v in embedding {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    /// Unpack a byte blob into a Vec<f32>.
    pub fn unpack_embedding(blob: &[u8]) -> Vec<f32> {
        blob.chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect()
    }

    /// Store an embedding vector for a message.
    pub fn save_embedding(
        &self,
        message_id: i64,
        embedding: &[f32],
        model: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let blob = Self::pack_embedding(embedding);
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO message_embeddings (message_id, embedding, model, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![message_id, blob, model, now],
        )?;
        Ok(())
    }

    /// Delete embeddings where the model doesn't match (e.g. after config change).
    pub fn cleanup_mismatched_embeddings(&self, model: &str) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "DELETE FROM message_embeddings WHERE model != ?1",
            params![model],
        )?;
        Ok(count)
    }

    /// Find message IDs that have no embedding (for backfill).
    /// Returns (message_id, text_content) pairs.
    pub fn find_unembedded_messages(&self) -> anyhow::Result<Vec<(i64, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.content
             FROM messages m
             LEFT JOIN message_embeddings e ON e.message_id = m.id
             WHERE e.id IS NULL
             ORDER BY m.id",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let content_json: String = row.get(1)?;
            Ok((id, content_json))
        })?;
        let mut results = Vec::new();
        for row in rows {
            let (id, content_json) = row?;
            let text = match serde_json::from_str::<ChatContent>(&content_json) {
                Ok(ChatContent::Text(t)) => t,
                Ok(ChatContent::Parts(parts)) => parts.iter()
                    .map(|p| match p {
                        crate::openrouter::ContentPart::Text { text } => text.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                Err(_) => continue,
            };
            if !text.is_empty() {
                results.push((id, text));
            }
        }
        Ok(results)
    }

    /// Search embeddings by cosine similarity, limited to the N newest by message_id.
    /// Returns (message_id, similarity_score, text_content) sorted highest score first.
    pub fn search_embeddings(
        &self,
        chat_id: &str,
        query_embedding: &[f32],
        model: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, f32, String)>> {
        let conn = self.conn.lock().unwrap();

        // Load only the N newest embeddings for this chat (by message_id DESC)
        let mut stmt = conn.prepare(
            "SELECT e.message_id, e.embedding, m.content
             FROM message_embeddings e
             JOIN messages m ON m.id = e.message_id
             WHERE m.chat_id = ?1 AND e.model = ?2
             ORDER BY e.message_id DESC
             LIMIT ?3",
        )?;

        struct Raw {
            message_id: i64,
            embedding_blob: Vec<u8>,
            content_json: String,
        }

        let rows = stmt.query_map(params![chat_id, model, limit as i64], |row| {
            Ok(Raw {
                message_id: row.get(0)?,
                embedding_blob: row.get(1)?,
                content_json: row.get(2)?,
            })
        })?;

        // Compute query norm once
        let query_norm: f32 = query_embedding.iter().map(|v| v * v).sum::<f32>().sqrt();

        let mut scored: Vec<(i64, f32, String)> = Vec::new();
        for row in rows {
            let raw = row?;
            let text = match serde_json::from_str::<ChatContent>(&raw.content_json) {
                Ok(ChatContent::Text(t)) => t,
                Ok(ChatContent::Parts(parts)) => parts.iter()
                    .map(|p| match p {
                        crate::openrouter::ContentPart::Text { text } => text.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                Err(_) => continue,
            };
            if text.is_empty() {
                continue;
            }

            let stored_vec = Self::unpack_embedding(&raw.embedding_blob);
            if stored_vec.len() != query_embedding.len() {
                continue; // model changed, skip stale rows (shouldn't happen after cleanup)
            }

            let mut dot = 0.0f32;
            let mut stored_norm_sq = 0.0f32;
            for (i, &v) in stored_vec.iter().enumerate() {
                dot += v * query_embedding[i];
                stored_norm_sq += v * v;
            }
            let stored_norm = stored_norm_sq.sqrt();

            let similarity = if query_norm > 0.0 && stored_norm > 0.0 {
                dot / (query_norm * stored_norm)
            } else {
                0.0
            };

            scored.push((raw.message_id, similarity, text));
        }

        // Sort by similarity descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let loaded = db.load_messages(chat_id, 10).unwrap();
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

        let loaded = db.load_messages(chat_id, 3).unwrap();
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
        let loaded = db.load_messages(chat_id, 10).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].tool_calls.is_some());
        let tc = loaded[0].tool_calls.as_ref().unwrap();
        assert_eq!(tc[0].id, "call_1");
        assert_eq!(tc[0].function.name, "bash");
    }

    #[test]
    fn test_empty_chat() {
        let db = make_db();
        let loaded = db.load_messages("nonexistent", 10).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_clear_messages() {
        let db = make_db();
        let chat_id = "-abc";

        db.save_messages(chat_id, &[ChatMessage::user("test")])
            .unwrap();
        assert_eq!(db.load_messages(chat_id, 10).unwrap().len(), 1);

        db.clear_messages(chat_id).unwrap();
        assert!(db.load_messages(chat_id, 10).unwrap().is_empty());
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
        let emb1 = vec![1.0f32, 0.0, 0.0, 0.0];  // aligns with query
        let emb2 = vec![0.0f32, 1.0, 0.0, 0.0];  // orthogonal

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
        assert!((results[0].1 - 1.0).abs() < 0.01, "Expected ~1.0, got {}", results[0].1);
        assert!(results[0].2.contains("Rust"));
        // Second result should be ~0.0 (orthogonal)
        assert!(results[1].1.abs() < 0.01, "Expected ~0.0, got {}", results[1].1);
    }

    #[test]
    fn test_search_embeddings_respects_limit() {
        let db = make_db();
        let chat_id = "-888";

        for i in 0..5 {
            let ids = db
                .save_messages(chat_id, &[ChatMessage::user(&format!("msg {i}"))])
                .unwrap();
            db.save_embedding(
                ids[0],
                &vec![i as f32, 0.0, 0.0, 0.0],
                "test-model",
            )
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
        db.save_embedding(ids[0], &[1.0, 0.0], "model-a")
            .unwrap();

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
        db.save_embedding(ids[0], &[1.0], "old-model")
            .unwrap();
        db.save_embedding(ids[1], &[1.0], "new-model")
            .unwrap();

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
        db.save_embedding(ids[1], &[1.0], "any-model")
            .unwrap();

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
        db.save_embedding(ids[0], &[1.0, 0.5], "model-x")
            .unwrap();

        // Search with a 4-dim query
        let results = db
            .search_embeddings("-222", &[1.0, 0.0, 0.0, 0.0], "model-x", 10)
            .unwrap();
        // Mismatched dimension should be skipped
        assert!(results.is_empty());
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
        db.save_embedding(ids[0], &[1.0, 0.0], "model-e")
            .unwrap();

        let results = db
            .search_embeddings("-000", &[1.0, 0.0], "model-e", 10)
            .unwrap();
        // Empty text messages should be skipped
        assert!(results.is_empty());
    }
}
