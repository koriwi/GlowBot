//! Generic, stateless event ingress. Callers resolve external routing metadata.
use crate::bot::GlowBot;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

pub type ChatLocks = Arc<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRequest {
    pub chat_ids: Vec<String>,
    pub event: serde_json::Value,
}

#[derive(Clone)]
pub struct EventContext {
    pub bot: Arc<Mutex<GlowBot>>,
    pub telegram: teloxide::Bot,
    pub chat_locks: ChatLocks,
}

pub fn router(context: EventContext) -> Router {
    Router::new()
        .route("/events", post(receive))
        .with_state(context)
}

async fn receive(
    State(context): State<EventContext>,
    Json(request): Json<EventRequest>,
) -> StatusCode {
    if request.chat_ids.is_empty() || request.chat_ids.len() > 100 || request.event.is_null() {
        return StatusCode::BAD_REQUEST;
    }
    // Validate every destination before starting any work.
    let (state, stop_signals) = {
        let bot = context.bot.lock().await;
        (bot.state.clone(), bot.stop_signals.clone())
    };
    {
        let s = state.lock().await;
        for id in &request.chat_ids {
            let Ok(number) = id.parse::<i64>() else {
                return StatusCode::BAD_REQUEST;
            };
            if number == 0 || number.to_string() != *id {
                return StatusCode::BAD_REQUEST;
            }
            if !(s.config.chats.contains_key(id) || s.config.dms.contains_key(id)) {
                return StatusCode::BAD_REQUEST;
            }
        }
    }
    // No deduplication: each supplied destination and each request triggers a run.
    for chat_id in request.chat_ids {
        let state = state.clone();
        let signals = stop_signals.clone();
        let telegram = context.telegram.clone();
        let event = request.event.clone();
        let lock = {
            let mut locks = context.chat_locks.lock().unwrap_or_else(|e| e.into_inner());
            locks
                .entry(chat_id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        tokio::spawn(async move {
            let _guard = lock.lock().await;
            if let Ok(signals) = signals.lock() {
                if let Some(signal) = signals.get(&chat_id) {
                    signal.store(false, std::sync::atomic::Ordering::SeqCst);
                }
            }
            if let Err(error) =
                crate::bot::run_event(state, signals, &chat_id, &event, &telegram).await
            {
                log::error!("Event in chat {} failed: {}", chat_id, error);
            }
        });
    }
    StatusCode::ACCEPTED
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
