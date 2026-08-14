use glowbot::bot::GlowBot;
use glowbot::bot_send::TextReplySender;
use glowbot::config::SmsConfig;
use glowbot::sms::{HuaweiSmsGateway, IncomingSms, SmsDeduplicator, SmsGateway, SmsReplySender};
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::types::ChatId;
use tokio::sync::Mutex;

pub type ChatLocks = Arc<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>;

pub async fn run_sms_loop(
    bot: Arc<Mutex<GlowBot>>,
    tg_bot: teloxide::Bot,
    chat_locks: ChatLocks,
    sms_config: SmsConfig,
) {
    loop {
        match HuaweiSmsGateway::connect(&sms_config).await {
            Ok(gateway) => {
                log::info!(
                    "SMS channel connected to Huawei modem at {}",
                    sms_config.ip_address
                );
                let gateway: Arc<dyn SmsGateway> = Arc::new(gateway);
                if let Err(error) = poll_connected_gateway(
                    Arc::clone(&bot),
                    tg_bot.clone(),
                    Arc::clone(&chat_locks),
                    gateway,
                )
                .await
                {
                    log::warn!("SMS modem polling failed: {error}; reconnecting in 15s");
                }
            }
            Err(error) => {
                log::warn!("Failed to connect SMS channel: {error}; retrying in 15s");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    }
}

async fn poll_connected_gateway(
    bot: Arc<Mutex<GlowBot>>,
    tg_bot: teloxide::Bot,
    chat_locks: ChatLocks,
    gateway: Arc<dyn SmsGateway>,
) -> anyhow::Result<()> {
    let mut deduplicator = SmsDeduplicator::default();
    loop {
        let messages = gateway.unread_messages().await?;
        for message in messages {
            if deduplicator.is_duplicate(&message) {
                log::debug!(
                    "Ignoring already processed SMS {} from {}",
                    message.id,
                    message.phone_number
                );
                continue;
            }
            if handle_incoming_sms(&bot, &tg_bot, &chat_locks, Arc::clone(&gateway), &message).await
            {
                deduplicator.remember(&message);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

async fn handle_incoming_sms(
    bot: &Arc<Mutex<GlowBot>>,
    tg_bot: &teloxide::Bot,
    chat_locks: &ChatLocks,
    gateway: Arc<dyn SmsGateway>,
    message: &IncomingSms,
) -> bool {
    let mapping = {
        let inner = bot.lock().await;
        let state = inner.state.lock().await;
        state
            .config
            .dm_for_phone_number(&message.phone_number)
            .map(|(chat_id, dm)| (chat_id.to_string(), dm.forward_sms_to_telegram))
    };

    let Some((chat_id, forward_to_telegram)) = mapping else {
        log::info!("Ignoring SMS from unknown number {}", message.phone_number);
        if let Err(error) = gateway.mark_read(&message.id).await {
            log::warn!("Failed to mark ignored SMS {} as read: {error}", message.id);
        }
        return true;
    };
    let Ok(chat_id_i64) = chat_id.parse::<i64>() else {
        log::error!("Mapped SMS chat ID '{}' is not a Telegram chat ID", chat_id);
        return false;
    };

    let telegram_forward = forward_to_telegram.then(|| (tg_bot.clone(), ChatId(chat_id_i64)));
    let reply_sender = SmsReplySender::new(
        Arc::clone(&gateway),
        message.phone_number.clone(),
        telegram_forward,
    );

    let chat_lock = {
        let mut locks = chat_locks.lock().unwrap_or_else(|error| error.into_inner());
        locks
            .entry(chat_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _guard = chat_lock.lock().await;

    reply_sender.forward_incoming(message).await;
    let (state, git_repo, stop_signals) = {
        let inner = bot.lock().await;
        (
            inner.state.clone(),
            inner.git_repo.clone(),
            inner.stop_signals.clone(),
        )
    };

    let result = glowbot::bot::process_sms_message_impl(
        &state,
        &git_repo,
        &stop_signals,
        &chat_id,
        &message.phone_number,
        &message.text,
        &message.modem_date,
        &reply_sender,
    )
    .await;

    let processed = finish_sms_turn(&message.id, result, &reply_sender).await;

    if processed {
        if let Err(error) = gateway.mark_read(&message.id).await {
            // Processing is terminal; remember the message even if this modem
            // update failed so the next poll cannot rerun the LLM turn.
            log::warn!(
                "Failed to mark processed SMS {} as read: {error:#}",
                message.id
            );
        }
    }
    processed
}

async fn finish_sms_turn(
    message_id: &str,
    result: anyhow::Result<Option<String>>,
    reply_sender: &dyn TextReplySender,
) -> bool {
    match result {
        Ok(Some(response)) => {
            if let Err(error) = reply_sender.send_text(&response).await {
                // The LLM turn has already been persisted. Retrying the unread
                // inbox entry would rerun the LLM and duplicate any segments
                // that were sent before the failure.
                log::error!("Failed to reply to SMS {message_id}: {error:#}");
            }
            true
        }
        Ok(None) => {
            log::warn!("Mapped SMS {message_id} produced no reply");
            true
        }
        Err(error) => {
            log::error!("Failed to process SMS {message_id}: {error:#}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingReplySender;

    #[async_trait::async_trait]
    impl TextReplySender for FailingReplySender {
        async fn send_text(&self, _text: &str) -> anyhow::Result<()> {
            anyhow::bail!("modem busy")
        }
    }

    #[tokio::test]
    async fn completed_turn_is_terminal_even_when_reply_delivery_fails() {
        assert!(
            finish_sms_turn("42", Ok(Some("reply".into())), &FailingReplySender).await,
            "a send failure must not cause the inbox message to rerun"
        );
        assert!(finish_sms_turn("42", Ok(None), &FailingReplySender).await);
        assert!(
            !finish_sms_turn(
                "42",
                Err(anyhow::anyhow!("LLM failed")),
                &FailingReplySender
            )
            .await
        );
    }
}
