use super::{process_message_for_source, BotState};
use crate::git::GitRepo;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Process an incoming Telegram message without holding the main GlowBot lock.
#[allow(clippy::too_many_arguments)]
pub async fn process_message_impl(
    state: &Arc<Mutex<BotState>>,
    git_repo: &GitRepo,
    stop_signals: &Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    chat_id: &str,
    user_id: &str,
    username: &str,
    text: Option<&str>,
    caption: Option<&str>,
    media: Option<&crate::media::IngestedMedia>,
    sender_name: Option<&str>,
    sent_at: Option<chrono::DateTime<chrono::Utc>>,
    bot_username: &str,
    tg_bot: Option<&teloxide::Bot>,
) -> anyhow::Result<Option<String>> {
    let telegram_sender = tg_bot.and_then(|bot| {
        chat_id
            .parse::<i64>()
            .ok()
            .map(|id| crate::bot_send::TelegramReplySender::new(bot, teloxide::types::ChatId(id)))
    });
    process_message_for_source(
        state,
        git_repo,
        stop_signals,
        chat_id,
        user_id,
        username,
        text,
        caption,
        media,
        sender_name,
        sent_at,
        bot_username,
        tg_bot,
        &super::bot_pipeline::MessageSource::Telegram,
        telegram_sender
            .as_ref()
            .map(|sender| sender as &dyn crate::bot_send::TextReplySender),
    )
    .await
}
