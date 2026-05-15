use rusqlite::params;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use crate::openrouter::{ChatContent, ContentPart};

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
               AND m.role != 'tool'
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
            "SELECT e.message_id, e.embedding, m.content
             FROM message_embeddings e
             JOIN messages m ON m.id = e.message_id
             WHERE m.chat_id = ?1 AND e.model = ?2
               AND m.role != 'tool'
             ORDER BY e.message_id DESC
             LIMIT ?3",
        )?;

        struct Raw {
            message_id: i64,
            embedding_blob: Vec<u8>,
            content_json: String,
        }

        let rows = stmt.query_map(params![chat_id, model, scan_limit as i64], |row| {
            Ok(Raw {
                message_id: row.get(0)?,
                embedding_blob: row.get(1)?,
                content_json: row.get(2)?,
            })
        })?;

        let query_norm: f32 = query_embedding.iter().map(|v| v * v).sum::<f32>().sqrt();

        if top_k == 0 {
            return Ok(Vec::new());
        }

        let mut heap = BinaryHeap::with_capacity(top_k + 1);

        for row in rows {
            let raw = row?;
            let Some(text) = Self::text_from_content_json(&raw.content_json) else {
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
