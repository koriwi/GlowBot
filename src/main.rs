use glowbot::bot::GlowBot;
use glowbot::config::Config;
use glowbot::llm::OpenRouterBackend;
use std::collections::HashSet;
use std::env;
use std::sync::Arc;
use std::time::Duration;
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
    let heartbeat_tg = tg_bot.clone();
    tokio::spawn(async move {
        run_heartbeat_loop(heartbeat_bot, heartbeat_tg).await;
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

/// Background heartbeat orchestrator. Discovers chats with tasks and spawns
/// a dedicated timer loop for each, using the chat's configured interval.
/// DMs use the global default; group chats can override.
async fn run_heartbeat_loop(bot: Arc<Mutex<GlowBot>>, tg_bot: Bot) {
    tokio::time::sleep(Duration::from_secs(30)).await;

    // Track which chats already have an active heartbeat task loop.
    // Using std::sync::Mutex because it's only accessed briefly for HashSet ops.
    let active: Arc<tokio::sync::Mutex<HashSet<String>>> =
        Arc::new(tokio::sync::Mutex::new(HashSet::new()));

    loop {
        // Scan for chats with pending tasks and their intervals.
        let chats: Vec<(String, u64)> = {
            let inner = bot.lock().await;
            let state = inner.state.lock().await;
            let chats_dir = state.chats_dir();
            let mut result = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&chats_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.parse::<i64>().is_ok() && state.has_pending_tasks(name) {
                                if let Some(interval_mins) = state.config.heartbeat_interval(name) {
                                    let interval_secs = interval_mins * 60;
                                    result.push((name.to_string(), interval_secs));
                                }
                            }
                        }
                    }
                }
            }
            result
        };

        for (chat_id, interval_secs) in chats {
            let mut guard = active.lock().await;
            if guard.insert(chat_id.clone()) {
                drop(guard);
                let bot_clone = Arc::clone(&bot);
                let tg_clone = tg_bot.clone();
                let active_clone = Arc::clone(&active);
                let cid = chat_id.clone();
                tokio::spawn(async move {
                    run_chat_heartbeat(bot_clone, tg_clone, cid, interval_secs).await;
                    active_clone.lock().await.remove(&chat_id);
                });
            }
        }

        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// Per-chat heartbeat loop. Processes available tasks, then sleeps
/// for the chat's configured interval before trying again.
async fn run_chat_heartbeat(
    bot: Arc<Mutex<GlowBot>>,
    tg_bot: Bot,
    chat_id: String,
    interval_secs: u64,
) {
    loop {
        // Check if heartbeat is still enabled for this chat
        let enabled = {
            let inner = bot.lock().await;
            let state = inner.state.lock().await;
            state.config.heartbeat_interval(&chat_id).is_some()
        };

        if !enabled {
            log::info!("Heartbeat disabled for chat {}, stopping loop", chat_id);
            break;
        }

        let (state, git_repo) = {
            let inner = bot.lock().await;
            (inner.state.clone(), inner.git_repo.clone())
        };

        glowbot::bot::run_heartbeat_task(state, git_repo, &chat_id, tg_bot.clone()).await;

        // After running tasks, check if there are any tasks remaining.
        // If the task list is empty, exit the loop so the chat becomes
        // eligible for re-discovery by the scheduler when new tasks arrive.
        let has_tasks = {
            let inner = bot.lock().await;
            let state = inner.state.lock().await;
            state.has_pending_tasks(&chat_id)
        };
        if !has_tasks {
            log::info!("Heartbeat chat {}: no tasks remaining, exiting loop", chat_id);
            break;
        }

        // Sleep until next interval for this specific chat
        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
    }
}
