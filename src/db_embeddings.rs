use rusqlite::params;
use crate::openrouter::{ChatContent, ContentPart};

use super::Database;

impl Database {
    /// Extract readable text from a serialised ChatContent JSON string.
    fn text_from_content_json(content_json: &str) -> Option<String> {
        let text = match serde_json::from_str::<ChatContent>(content_json) {
            Ok(ChatContent::Text(t)) => t,
            Ok(ChatContent::Parts(parts)) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
            Err(_) => return None,
        };
        if text.is_empty() { None } else { Some(text) }
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
            if let Some(text) = Self::text_from_content_json(&content_json) {
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
        let conn = self.lock_conn();

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
            let Some(text) = Self::text_from_content_json(&raw.content_json) else {
                continue;
            };

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
