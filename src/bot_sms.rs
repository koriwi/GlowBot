use super::{process_message_for_source, BotState};
use crate::git::GitRepo;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Process an incoming SMS in the mapped Telegram DM conversation.
#[allow(clippy::too_many_arguments)]
pub async fn process_sms_message_impl(
    state: &Arc<Mutex<BotState>>,
    git_repo: &GitRepo,
    stop_signals: &Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    chat_id: &str,
    phone_number: &str,
    text: &str,
    modem_date: &str,
    reply_sender: &dyn crate::bot_send::TextReplySender,
) -> anyhow::Result<Option<String>> {
    let sender_name = {
        let state = state.lock().await;
        state
            .config
            .dm_config(chat_id)
            .and_then(|dm| dm.name.clone())
    };
    process_message_for_source(
        state,
        git_repo,
        stop_signals,
        chat_id,
        chat_id,
        phone_number,
        Some(text),
        None,
        None,
        sender_name.as_deref(),
        None,
        "",
        None,
        &super::bot_pipeline::MessageSource::Sms {
            phone_number: phone_number.to_string(),
            modem_date: modem_date.to_string(),
        },
        Some(reply_sender),
    )
    .await
}
