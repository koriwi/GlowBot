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
    pub fn save_messages(
        &self,
        chat_id: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().timestamp();

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
        }

        tx.commit()?;
        Ok(())
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
}
