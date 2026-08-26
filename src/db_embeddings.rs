use crate::openrouter::{ChatContent, ChatMessage, ToolCall};
use rusqlite::params;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::Database;

/// A scored search result for the bounded min-heap.
/// Ord is reversed (lower similarity = Greater) so that the heap
/// acts as a min-heap: the item with lowest similarity is at the
/// top and gets popped when the heap exceeds capacity.
#[derive(Debug)]
struct ScoredItem {
    similarity: f32,
    message_id: i64,
    text: String,
}

impl PartialEq for ScoredItem {
    fn eq(&self, other: &Self) -> bool {
        self.similarity == other.similarity
    }
}

impl Eq for ScoredItem {}

impl PartialOrd for ScoredItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse: lower similarity is "greater" so it sits at the top of
        // the max-heap (popped first when over capacity).
        other
            .similarity
            .partial_cmp(&self.similarity)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.message_id.cmp(&other.message_id))
    }
}

impl Database {
    /// Text used for semantic conversation search, including tool calls and results.
    pub(crate) fn searchable_text(message: &ChatMessage) -> String {
        let text = message.text_content();
        if message.role == "tool" {
            let call_id = message.tool_call_id.as_deref().unwrap_or("unknown");
            return format!("[Tool result]\nCall ID: {call_id}\n{text}");
        }

        let Some(tool_calls) = &message.tool_calls else {
            return text;
        };
        let calls = tool_calls
            .iter()
            .map(|call| {
                format!(
                    "[Tool call]\nName: {}\nArguments: {}",
                    call.function.name, call.function.arguments
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        if text.trim().is_empty() {
            calls
        } else {
            format!("{text}\n\n{calls}")
        }
    }

    fn searchable_text_from_json(
        role: String,
        content_json: &str,
        tool_calls_json: Option<String>,
        tool_call_id: Option<String>,
    ) -> Option<String> {
        let content = serde_json::from_str::<ChatContent>(content_json).ok()?;
        let tool_calls: Option<Vec<ToolCall>> = tool_calls_json
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .ok()?;
        let message = ChatMessage {
            role,
            content,
            name: None,
            tool_calls,
            tool_call_id,
            reasoning: None,
            provider_data: None,
        };
        let text = Self::searchable_text(&message);
        (!text.trim().is_empty()).then_some(text)
    }

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
        let conn = self.lock_conn();
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
        let conn = self.lock_conn();
        let count = conn.execute(
            "DELETE FROM message_embeddings WHERE model != ?1",
            params![model],
        )?;
        Ok(count)
    }

    /// Find message IDs that have no embedding (for backfill).
    /// Returns (message_id, text_content) pairs.
    pub fn find_unembedded_messages(&self) -> anyhow::Result<Vec<(i64, String)>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.role, m.content, m.tool_calls, m.tool_call_id
             FROM messages m
             LEFT JOIN message_embeddings e ON e.message_id = m.id
             WHERE e.id IS NULL
             ORDER BY m.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut results = Vec::new();
        for row in rows {
            let (id, role, content_json, tool_calls_json, tool_call_id) = row?;
            if let Some(text) =
                Self::searchable_text_from_json(role, &content_json, tool_calls_json, tool_call_id)
            {
                results.push((id, text));
            }
        }
        Ok(results)
    }

    /// Search embeddings by cosine similarity, streaming rows from SQLite
    /// and keeping the top-K in a bounded min-heap so memory is O(K) instead
    /// of O(scan_limit).
    ///
    /// Returns up to `top_k` (message_id, similarity_score, text_content)
    /// sorted highest score first.
    pub fn search_embeddings(
        &self,
        chat_id: &str,
        query_embedding: &[f32],
        model: &str,
        top_k: usize,
        scan_limit: usize,
    ) -> anyhow::Result<Vec<(i64, f32, String)>> {
        let conn = self.lock_conn();

        let mut stmt = conn.prepare(
            "SELECT e.message_id, e.embedding, m.role, m.content,
                    m.tool_calls, m.tool_call_id
             FROM message_embeddings e
             JOIN messages m ON m.id = e.message_id
             WHERE m.chat_id = ?1 AND e.model = ?2
             ORDER BY e.message_id DESC
             LIMIT ?3",
        )?;

        struct Raw {
            message_id: i64,
            embedding_blob: Vec<u8>,
            role: String,
            content_json: String,
            tool_calls_json: Option<String>,
            tool_call_id: Option<String>,
        }

        let rows = stmt.query_map(params![chat_id, model, scan_limit as i64], |row| {
            Ok(Raw {
                message_id: row.get(0)?,
                embedding_blob: row.get(1)?,
                role: row.get(2)?,
                content_json: row.get(3)?,
                tool_calls_json: row.get(4)?,
                tool_call_id: row.get(5)?,
            })
        })?;

        let query_norm: f32 = query_embedding.iter().map(|v| v * v).sum::<f32>().sqrt();

        if top_k == 0 {
            return Ok(Vec::new());
        }

        let mut heap = BinaryHeap::with_capacity(top_k + 1);

        for row in rows {
            let raw = row?;
            let Some(text) = Self::searchable_text_from_json(
                raw.role,
                &raw.content_json,
                raw.tool_calls_json,
                raw.tool_call_id,
            ) else {
                continue;
            };

            let stored_vec = Self::unpack_embedding(&raw.embedding_blob);
            if stored_vec.len() != query_embedding.len() {
                continue;
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

            heap.push(ScoredItem {
                similarity,
                message_id: raw.message_id,
                text,
            });
            if heap.len() > top_k {
                heap.pop();
            }
        }

        // Heap → sorted vec: BinaryHeap::into_sorted_vec gives ascending order
        // by our Ord.  Since our Ord is reversed (higher similarity = Less),
        // ascending yields highest similarity first — the desired order.
        let results: Vec<(i64, f32, String)> = heap
            .into_sorted_vec()
            .into_iter()
            .map(|item| (item.message_id, item.similarity, item.text))
            .collect();

        Ok(results)
    }
}
