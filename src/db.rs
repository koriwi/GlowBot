use crate::openrouter::{ChatContent, ChatMessage};
use anyhow::Context;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Persistent SQLite-backed conversation history.
/// One row per message; read with a sliding window configurable via
/// `conversation.recent_messages_window_size`.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open (or create) the database at the given path and run migrations.
    ///
    /// Uses `sqldiff --schema` to diff a reference database (built from
    /// the schema directory) against the live database, falling back to
    /// direct SQL initialisation if the binary is not available.
    pub fn new(db_path: &Path, schema_dir: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

        match Self::migrate_with_sqldiff(db_path, schema_dir) {
            Ok(()) => {}
            Err(e) => {
                log::warn!(
                    "sqldiff migration failed, falling back to direct init: {}",
                    e
                );
                Self::init_direct(&conn)?;
            }
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory database for tests.
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;
        Self::init_direct(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Build a temporary reference database from the schema `.sql` files,
    /// run `sqldiff --schema` to compute the delta, and apply it to the
    /// live database.
    fn migrate_with_sqldiff(db_path: &Path, schema_dir: &Path) -> anyhow::Result<()> {
        // Build a temporary reference database with the desired schema.
        let ref_file = tempfile::NamedTempFile::new()
            .context("Failed to create temp file for reference database")?;
        let ref_conn =
            Connection::open(ref_file.path()).context("Failed to open reference database")?;

        for entry in std::fs::read_dir(schema_dir)
            .with_context(|| format!("Failed to read schema dir: {}", schema_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "sql") {
                let sql = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read schema file: {}", path.display()))?;
                ref_conn.execute_batch(&sql).with_context(|| {
                    format!("Failed to execute schema file: {}", path.display())
                })?;
            }
        }
        drop(ref_conn);

        // Diff the live database against the reference.
        let output = std::process::Command::new("sqldiff")
            .args([
                "--schema",
                db_path.to_str().context("db_path is not valid UTF-8")?,
                ref_file
                    .path()
                    .to_str()
                    .context("ref_path is not valid UTF-8")?,
            ])
            .output()
            .context("Failed to run sqldiff. Is it installed?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("sqldiff failed:\n{}", stderr);
        }

        let diff_sql = String::from_utf8_lossy(&output.stdout);
        let diff_sql = diff_sql.trim();
        if diff_sql.is_empty() {
            log::info!("Database schema is up to date.");
            return Ok(());
        }

        log::info!("Applying schema migration:\n{}", diff_sql);

        // Apply the diff to the live database (open a fresh connection).
        let conn = Connection::open(db_path).with_context(|| {
            format!(
                "Failed to open database for migration: {}",
                db_path.display()
            )
        })?;
        conn.execute_batch(diff_sql)
            .context("Failed to apply migration SQL")?;

        Ok(())
    }

    /// Direct schema initialisation — used for tests and as a fallback
    /// when `sqldiff` is not available.
    fn init_direct(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
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
            CREATE INDEX IF NOT EXISTS idx_messages_chat_created
             ON messages(chat_id, created_at);
            CREATE TABLE IF NOT EXISTS message_embeddings (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                embedding   BLOB NOT NULL,
                model       TEXT NOT NULL,
                created_at  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_embeddings_message
             ON message_embeddings(message_id);
            CREATE INDEX IF NOT EXISTS idx_embeddings_model_message
             ON message_embeddings(model, message_id);",
        )
        .context("Failed to initialize database schema")?;

        // Migration: add reasoning column if it doesn't exist (for databases
        // created before this column was added).
        let _ = conn.execute("ALTER TABLE messages ADD COLUMN reasoning TEXT", []);

        Ok(())
    }

    /// Load the most recent `limit` messages for a chat, ordered from oldest to newest.
    pub fn load_messages(&self, chat_id: &str, limit: usize) -> anyhow::Result<Vec<ChatMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT role, content, reasoning, name, tool_calls, tool_call_id
             FROM messages
             WHERE chat_id = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;

        struct Raw {
            role: String,
            content_json: String,
            reasoning: Option<String>,
            name: Option<String>,
            tool_calls_json: Option<String>,
            tool_call_id: Option<String>,
        }

        let rows = stmt.query_map(params![chat_id, limit as i64], |row| {
            Ok(Raw {
                role: row.get(0)?,
                content_json: row.get(1)?,
                reasoning: row.get(2)?,
                name: row.get(3)?,
                tool_calls_json: row.get(4)?,
                tool_call_id: row.get(5)?,
            })
        })?;

        let mut msgs = Vec::with_capacity(limit.min(20));
        for row in rows {
            let raw = row?;
            let content: ChatContent = serde_json::from_str(&raw.content_json)
                .with_context(|| format!("Failed to deserialize content for chat {}", chat_id))?;
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
                reasoning: raw.reasoning,
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
                .map(serde_json::to_string)
                .transpose()
                .context("Failed to serialize tool_calls")?;

            tx.execute(
                "INSERT INTO messages
                 (chat_id, role, content, reasoning, name, tool_calls, tool_call_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    chat_id,
                    &msg.role,
                    content_json,
                    msg.reasoning.as_deref(),
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
        conn.execute("DELETE FROM messages WHERE chat_id = ?1", params![chat_id])?;
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
                Ok(ChatContent::Parts(parts)) => parts
                    .iter()
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
                Ok(ChatContent::Parts(parts)) => parts
                    .iter()
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
#[path = "db_tests.rs"]
mod tests;
