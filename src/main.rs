use glowbot::bot::GlowBot;
use glowbot::config::Config;
use glowbot::llm::OpenRouterBackend;
use std::env;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    run_bot().await
}

async fn run_bot() -> anyhow::Result<()> {
    let data_dir = env::var("GLOWBOT_DATA_DIR").unwrap_or_else(|_| "glowbot_data".to_string());
    let data_dir = std::path::PathBuf::from(data_dir);

    log::info!("GlowBot starting...");
    log::info!("Loading configuration from: {}", data_dir.display());

    // Load config first to get the API key
    let config = Config::load(&data_dir.join("config.yaml"))?;
    let telegram_token = config.telegram_token.clone();
    let openrouter_key = config.openrouter_api_key.clone();

    let llm = Arc::new(OpenRouterBackend::new(openrouter_key));
    let bot = GlowBot::new_with_llm(&data_dir, llm).await?;
    let bot = Arc::new(Mutex::new(bot));

    log::info!("Initializing Telegram bot...");
    let tg_bot = Bot::new(telegram_token);
    let bot_username = tg_bot.get_me().await?.username.clone().unwrap_or_default();
    log::info!("Bot username: @{}", bot_username);

    // Spawn heartbeat task runner in the background
    let heartbeat_bot = Arc::clone(&bot);
    tokio::spawn(async move {
        run_heartbeat_loop(heartbeat_bot).await;
    });

    log::info!("GlowBot is ready. Starting long-polling...");

    // Wrap polling in a loop to survive network timeouts/disconnects.
    // teloxide's repl returns () and swallows internal errors, but if the
    // underlying connection dies, the task completes. Restart it.
    loop {
        let handler_bot = Arc::clone(&bot);
        let handler_username = bot_username.clone();
        let handler_tg = tg_bot.clone();

        let handle = tokio::spawn(async move {
            teloxide::repl(handler_tg, move |tg_bot: Bot, msg: Message| {
                let bot = Arc::clone(&handler_bot);
                let bot_username = handler_username.clone();
                async move {
                    handle_message(tg_bot, bot, msg, &bot_username).await;
                    Ok(())
                }
            })
            .await;
        });

        match handle.await {
            Ok(()) => {
                log::error!("Polling loop exited unexpectedly, restarting in 5s");
            }
            Err(e) => {
                if e.is_cancelled() {
                    log::info!("Polling task cancelled, shutting down");
                    break;
                }
                log::error!("Polling task panicked, restarting in 5s: {}", e);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    Ok(())
}

async fn handle_message(tg_bot: Bot, bot: Arc<Mutex<GlowBot>>, msg: Message, bot_username: &str) {
    let text = match msg.text() {
        Some(t) => t,
        None => return,
    };

    let chat_id = msg.chat.id.to_string();
    let user_id = msg
        .from
        .as_ref()
        .map(|u| u.id.to_string())
        .unwrap_or_default();
    let username = msg
        .from
        .as_ref()
        .and_then(|u| u.username.as_deref())
        .unwrap_or("unknown");

    log::info!(
        "Message from {} ({}) in chat {}: {}",
        username,
        user_id,
        chat_id,
        text
    );

    // Show typing indicator while processing
    let chat = ChatId(chat_id.parse().unwrap_or_default());
    let _ = tg_bot
        .send_chat_action(chat, teloxide::types::ChatAction::Typing)
        .await;

    let bot_inner = bot.lock().await;
    match bot_inner
        .process_message(&chat_id, &user_id, username, text, bot_username)
        .await
    {
        Ok(Some(response)) => {
            // MarkdownV2: escape reserved chars that LLMs output in natural text,
            // but preserve formatting markers: * _ ` ~
            let escaped = glowbot::escape_v2_safe(&response);
            let result = tg_bot
                .send_message(chat, &escaped)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .await;
            if let Err(e) = result {
                log::warn!("MarkdownV2 parse failed, sending as plain text: {}", e);
                if let Err(e2) = tg_bot.send_message(chat, &response).await {
                    log::error!("Failed to send message: {}", e2);
                }
            }
        }
        Ok(None) => {}
        Err(e) => {
            log::error!("Error processing message: {}", e);
        }
    }
}

/// Background heartbeat loop. Periodically processes pending tasks for all chats.
async fn run_heartbeat_loop(bot: Arc<Mutex<GlowBot>>) {
    // Wait 30s after startup before first tick
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;

    loop {
        // Collect chat IDs that have pending tasks and their heartbeat intervals
        let chats_to_process: Vec<(String, u64)> = {
            let inner = bot.lock().await;
            let state = inner.state.lock().await;
            let chats_dir = state.chats_dir();

            // Discover all chat directories
            let mut chat_ids = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&chats_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            // Check if this chat has pending tasks
                            if let Ok(list) = glowbot::tasks::TaskList::load(&chats_dir, name) {
                                if list.has_tasks() {
                                    if let Some(interval) = state.config.heartbeat_interval(name) {
                                        chat_ids.push((name.to_string(), interval));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            chat_ids
        };

        // Process one task per chat with pending tasks
        for (chat_id, _interval) in &chats_to_process {
            let chat_id = chat_id.clone();
            // Extract state and git_repo without blocking message processing
            let (state, git_repo) = {
                let inner = bot.lock().await;
                (inner.state.clone(), inner.git_repo.clone())
            };
            glowbot::bot::run_heartbeat_task(state, git_repo, &chat_id).await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        // Determine next sleep: use the minimum interval across all chats with tasks,
        // or fall back to the global default
        let sleep_secs = if chats_to_process.is_empty() {
            // No tasks — check every 5 minutes
            300
        } else {
            // Use the smallest interval, or 300s if empty
            chats_to_process
                .iter()
                .map(|(_, i)| *i * 60)
                .min()
                .unwrap_or(300)
        };

        log::debug!(
            "Heartbeat processed {} chats, sleeping {}s",
            chats_to_process.len(),
            sleep_secs
        );
        tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;
    }
}
