use crate::openrouter::{ChatContent, ChatMessage, ToolCall};
use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[path = "db_migrations.rs"]
mod db_migrations;
#[path = "db_embeddings.rs"]
mod db_embeddings;

/// Intermediate row type for message loading.
struct RawMessage {
    role: String,
    content_json: String,
    reasoning: Option<String>,
    name: Option<String>,
    tool_calls_json: Option<String>,
    tool_call_id: Option<String>,
}

/// Persistent SQLite-backed conversation history.
/// One row per message; read with a sliding window configurable via
/// `conversation.recent_messages_window_size`.
#[derive(Clone)]
pub struct Database {
    pub(crate) conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Lock the connection, recovering from poison if a previous holder panicked.
    fn lock_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open (or create) the database at the given path and run migrations.
    ///
    /// Uses `sqldiff --schema` to diff a reference database (built from
    /// the schema directory) against the live database, falling back to
    /// direct SQL initialisation if the binary is not available.
    pub fn new(db_path: &Path, schema_dir: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

        match db_migrations::migrate_with_sqldiff(db_path, schema_dir) {
            Ok(()) => {}
            Err(e) => {
                log::warn!(
                    "sqldiff migration failed, falling back to direct init: {}",
                    e
                );
                db_migrations::init_direct(&conn)?;
            }
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory database for tests.
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;
        db_migrations::init_direct(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Load the most recent `limit` messages for a chat, ordered from oldest to newest.
    /// If `since` is provided, only messages with `created_at > since` are returned.
    pub fn load_messages(
        &self,
        chat_id: &str,
        limit: usize,
        since: Option<i64>,
    ) -> anyhow::Result<Vec<ChatMessage>> {
        let conn = self.lock_conn();

        let sql = if since.is_some() {
            "SELECT role, content, reasoning, name, tool_calls, tool_call_id
             FROM messages
             WHERE chat_id = ?1 AND created_at > ?2
             ORDER BY id DESC
             LIMIT ?3"
        } else {
            "SELECT role, content, reasoning, name, tool_calls, tool_call_id
             FROM messages
             WHERE chat_id = ?1
             ORDER BY id DESC
             LIMIT ?2"
        };
        let mut stmt = conn.prepare(sql)?;

        let raws: Vec<RawMessage> = if let Some(s) = since {
            stmt.query_map(params![chat_id, s, limit as i64], Self::map_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(params![chat_id, limit as i64], Self::map_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut msgs = Vec::with_capacity(raws.len());
        for raw in raws {
            let content: ChatContent = serde_json::from_str(&raw.content_json)
                .with_context(|| format!("Failed to deserialize content for chat {}", chat_id))?;
            let tool_calls: Option<Vec<ToolCall>> = raw
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

    /// Helper to map a row into a Raw struct for load_messages.
    fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawMessage> {
        Ok(RawMessage {
            role: row.get(0)?,
            content_json: row.get(1)?,
            reasoning: row.get(2)?,
            name: row.get(3)?,
            tool_calls_json: row.get(4)?,
            tool_call_id: row.get(5)?,
        })
    }

    /// Insert a batch of messages in a single transaction.
    /// Returns the row IDs of the inserted messages.
    pub fn save_messages(
        &self,
        chat_id: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<Vec<i64>> {
        let mut conn = self.lock_conn();
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
        let conn = self.lock_conn();
        conn.execute("DELETE FROM messages WHERE chat_id = ?1", params![chat_id])?;
        Ok(())
    }

    /// Set the "forget" cutoff timestamp for a chat. Messages with `created_at <= cutoff_at`
    /// are excluded from future `load_messages` calls when `since` is provided.
    pub fn set_cutoff(&self, chat_id: &str, cutoff_at: i64) -> anyhow::Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO chat_cutoffs (chat_id, cutoff_at) VALUES (?1, ?2)
             ON CONFLICT(chat_id) DO UPDATE SET cutoff_at = ?2",
            params![chat_id, cutoff_at],
        )?;
        Ok(())
    }

    /// Get the cutoff timestamp for a chat, if set.
    pub fn get_cutoff(&self, chat_id: &str) -> anyhow::Result<Option<i64>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare("SELECT cutoff_at FROM chat_cutoffs WHERE chat_id = ?1")?;
        let result = stmt
            .query_row(params![chat_id], |row| row.get(0))
            .optional();
        match result {
            Ok(val) => Ok(val),
            Err(e) => Err(anyhow::anyhow!("Failed to get cutoff: {}", e)),
        }
    }
}

#[cfg(test)]
#[path = "db_tests.rs"]
mod tests;
