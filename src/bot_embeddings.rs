use crate::db::Database;
use crate::openrouter::{ChatMessage, OpenRouterClient};
/// Embed each message in a turn and store the vectors.
/// Runs as a background task — failures are logged but don't affect the user.
pub(crate) async fn embed_turn(
    db: &Database,
    api_key: &str,
    embed_model: &str,
    max_chars: usize,
    allow_split: bool,
    message_ids: &[i64],
    turn_messages: &[ChatMessage],
) {
    let client = OpenRouterClient::new(api_key.to_string());
    for (i, msg) in turn_messages.iter().enumerate() {
        if i >= message_ids.len() {
            break;
        }
        let text = msg.text_content();
        if text.is_empty() {
            continue;
        }
        let chunks = chunk_for_embedding(&text, max_chars, allow_split);
        for chunk in &chunks {
            let text_preview: String = chunk.chars().take(80).collect();
            match client.embeddings(embed_model, chunk).await {
                Ok(vec) => {
                    if let Err(e) = db.save_embedding(message_ids[i], &vec, embed_model) {
                        log::warn!(
                            "Failed to save embedding for message {} (model={}, text=\"{}\"): {}",
                            message_ids[i],
                            embed_model,
                            text_preview,
                            e
                        );
                    }
                }
                Err(e) => {
                    log::warn!(
                        "Failed to embed message {} (model={}, text=\"{}\"): {}",
                        message_ids[i],
                        embed_model,
                        text_preview,
                        e
                    );
                }
            }
        }
    }
}

/// Split text into chunks for embedding based on max_chars and allow_split.
/// Returns a Vec of strings — always at least one element.
pub(crate) fn chunk_for_embedding(text: &str, max_chars: usize, allow_split: bool) -> Vec<String> {
    if max_chars == 0 {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return vec![text.to_string()];
    }
    if allow_split {
        chars
            .chunks(max_chars)
            .map(|c| c.iter().collect())
            .collect()
    } else {
        vec![chars[..max_chars].iter().collect()]
    }
}

/// Embed a freshly saved incoming message even when auto_detect stays silent.
pub(crate) async fn embed_saved(
    state: &std::sync::Arc<tokio::sync::Mutex<super::super::BotState>>,
    ids: Vec<i64>,
    messages: Vec<ChatMessage>,
) {
    let s = state.lock().await;
    if let Some(model) = s.config.openrouter.embedding_model.clone() {
        let db = s.db.clone();
        let key = s.config.openrouter.api_key.clone();
        let max_chars = s.config.embedding.max_chars;
        let allow_split = s.config.embedding.allow_split;
        drop(s);
        tokio::spawn(async move {
            embed_turn(&db, &key, &model, max_chars, allow_split, &ids, &messages).await;
        });
    }
}
