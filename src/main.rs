use glowbot::bot::GlowBot;
use glowbot::config::Config;
use glowbot::llm::OpenRouterBackend;
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::Arc;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::{BotCommand, ParseMode, UpdateKind};
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
    let openrouter_key = config.openrouter.api_key.clone();

    let llm = Arc::new(OpenRouterBackend::new(openrouter_key.clone()));
    let bot = GlowBot::new_with_llm(&data_dir, llm).await?;
    let bot = Arc::new(Mutex::new(bot));

    // Fetch model metadata from OpenRouter in the background.
    // Avoid holding the bot lock during the HTTP call so message processing isn't blocked.
    {
        let api_key = openrouter_key.clone();
        let state = bot.lock().await.state.clone();
        tokio::spawn(async move {
            let client = glowbot::openrouter::OpenRouterClient::new(api_key);
            match client.fetch_models().await {
                Ok(models) => {
                    let mut s = state.lock().await;
                    for m in models {
                        s.model_metadata.insert(m.id.clone(), m);
                    }
                    log::info!(
                        "Cached {} model metadata entries from OpenRouter",
                        s.model_metadata.len()
                    );
                }
                Err(e) => {
                    log::warn!("Failed to fetch model metadata: {}", e);
                }
            }
        });
    }

    // Start embedding backfill (cleanup + async background job)
    {
        let bot_clone = Arc::clone(&bot);
        bot_clone.lock().await.start_embedding_backfill().await;
    }

    log::info!("Initializing Telegram bot...");
    // Create HTTP client with TCP keepalive so the 30s long-poll connection
    // survives NAT/firewall idle-connection timeouts (which were causing the
    // "operation timed out" errors on get_updates every ~17s).
    let http_client = reqwest_011::Client::builder()
        .tcp_keepalive(std::time::Duration::from_secs(15))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(35))
        .build()
        .expect("Failed to build reqwest client for Telegram");
    let tg_bot = Bot::with_client(telegram_token, http_client);
    let bot_username = tg_bot.get_me().await?.username.clone().unwrap_or_default();
    log::info!("Bot username: @{}", bot_username);

    // Register slash commands with Telegram so they show in the menu and autocomplete
    let commands = vec![
        BotCommand::new("status", "Show current config for this chat"),
        BotCommand::new("model", "Set or view the current model"),
        BotCommand::new("models", "Browse and temporarily switch models"),
        BotCommand::new("model_default", "Reset model to config default"),
        BotCommand::new("tasks", "Show pending tasks for this chat"),
        BotCommand::new("todos", "Show your todo list for this chat"),
        BotCommand::new("reminders", "Show pending reminders for this chat"),
        BotCommand::new("new", "Reset context — messages before now are excluded from conversation"),
        BotCommand::new("prompt", "Show the system prompt sent to the LLM"),
        BotCommand::new("run", "Run task agent immediately for this chat"),
        BotCommand::new("tools", "Show available tools in this chat"),
        BotCommand::new("config", "Show the current config (redacted)"),
        BotCommand::new("config_schema", "Show the JSON Schema for config fields"),
        BotCommand::new("stop", "Stop the bot"),
    ];
    if let Err(e) = tg_bot.set_my_commands(commands).await {
        log::warn!("Failed to set bot commands: {}", e);
    } else {
        log::info!("Registered bot commands with Telegram");
    }

    // Spawn heartbeat task runner in the background
    let heartbeat_bot = Arc::clone(&bot);
    let heartbeat_tg = tg_bot.clone();
    tokio::spawn(async move {
        run_heartbeat_loop(heartbeat_bot, heartbeat_tg).await;
    });

    // Per-chat locks: ensures only one message per chat is processed at a time,
    // while /stop commands can bypass the lock to signal cancellation.
    let chat_locks: Arc<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        Arc::new(std::sync::Mutex::new(HashMap::new()));

    log::info!("GlowBot is ready. Starting long-polling...");

    // Manual polling loop to handle both Message and CallbackQuery updates.
    // Wrapped in a restart loop to survive network timeouts/disconnects.
    'polling: loop {
        let poll_bot = tg_bot.clone();
        let poll_bot_inner = Arc::clone(&bot);
        let poll_username = bot_username.clone();
        let poll_locks = Arc::clone(&chat_locks);

        let handle = tokio::spawn(async move {
            let mut offset: i32 = 0;
            let mut consecutive_errors: u32 = 0;
            loop {
                let updates_result = poll_bot
                    .get_updates()
                    .offset(offset)
                    .timeout(30)
                    .allowed_updates(vec![
                        teloxide::types::AllowedUpdate::Message,
                        teloxide::types::AllowedUpdate::CallbackQuery,
                    ])
                    .await;

                match updates_result {
                    Ok(updates) => {
                        if consecutive_errors > 0 {
                            log::info!(
                                "GetUpdates recovered after {} consecutive error(s)",
                                consecutive_errors
                            );
                            consecutive_errors = 0;
                        }
                        for update in updates {
                            offset = (update.id.0 as i32) + 1;
                            match update.kind {
                                UpdateKind::Message(msg) => {
                                    let tg = poll_bot.clone();
                                    let b = Arc::clone(&poll_bot_inner);
                                    let l = Arc::clone(&poll_locks);
                                    let uname = poll_username.clone();
                                    tokio::spawn(async move {
                                        handle_message(tg, b, l, msg, &uname).await;
                                    });
                                }
                                UpdateKind::CallbackQuery(cb) => {
                                    let tg = poll_bot.clone();
                                    let b = Arc::clone(&poll_bot_inner);
                                    handle_callback(tg, b, cb).await;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        consecutive_errors += 1;
                        // Only escalate to WARN after 5 consecutive failures.
                        // TCP keepalive (15s) should prevent most timeouts;
                        // these should now only fire during genuine outages.
                        if consecutive_errors >= 5 {
                            log::warn!(
                                "GetUpdates error: {} ({} consecutive failures, retrying in 5s)",
                                e,
                                consecutive_errors
                            );
                        } else {
                            log::debug!(
                                "GetUpdates error: {}, retrying in 5s",
                                e
                            );
                        }
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });

        match handle.await {
            Ok(()) => {
                log::error!("Polling loop exited unexpectedly, restarting in 5s");
            }
            Err(e) => {
                if e.is_cancelled() {
                    log::info!("Polling task cancelled, shutting down");
                    break 'polling;
                }
                log::error!("Polling task panicked, restarting in 5s: {}", e);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    Ok(())
}

async fn handle_message(
    tg_bot: Bot,
    bot: Arc<Mutex<GlowBot>>,
    chat_locks: Arc<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    msg: Message,
    bot_username: &str,
) {
    let text = msg.text().map(|s| s.to_string());
    let caption = msg.caption().map(|s| s.to_string());
    let media = glowbot::media::IngestedMedia::try_from_message(&msg);

    if text.is_none() && media.is_none() {
        return; // truly nothing to process (e.g. unsupported message type)
    }

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
        text.as_deref().unwrap_or("(media)")
    );

    let Ok(chat_id_i64) = chat_id.parse::<i64>() else {
        log::error!("BUG: Telegram chat_id '{}' does not parse as i64", chat_id);
        return;
    };
    let chat = ChatId(chat_id_i64);

    // Extract bot components
    let (state, git_repo, stop_signals) = {
        let bot_inner = bot.lock().await;
        (
            bot_inner.state.clone(),
            bot_inner.git_repo.clone(),
            bot_inner.stop_signals.clone(),
        )
    };

    // /stop bypasses the per-chat lock: just set the stop signal and return
    if text.as_deref().is_some_and(|t| t.trim() == "/stop") {
        if let Ok(signals) = stop_signals.lock() {
            if let Some(signal) = signals.get(&chat_id) {
                signal.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        glowbot::bot_send::send_message(
            &tg_bot,
            chat,
            "⏹ Stop signal sent. Current operations will be cancelled.",
        )
        .await;
        return;
    }

    // Acquire per-chat lock for normal message processing
    let chat_lock = {
        let mut locks = chat_locks.lock().unwrap_or_else(|e| e.into_inner());
        locks
            .entry(chat_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _guard = chat_lock.lock().await;

    // Clear any lingering stop signal from a previous /stop command.
    // The signal is meant to abort ongoing LLM processing, not to block
    // subsequent messages once the per-chat lock is re-acquired.
    if let Ok(signals) = stop_signals.lock() {
        if let Some(sig) = signals.get(&chat_id) {
            sig.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    match glowbot::bot::process_message_impl(
        &state,
        &git_repo,
        &stop_signals,
        &chat_id,
        &user_id,
        username,
        text.as_deref(),
        caption.as_deref(),
        media.as_ref(),
        bot_username,
        Some(&tg_bot),
    )
    .await
    {
        Ok(Some(response)) => {
            log::info!(
                "main: got response for chat {} (len={}), sending...",
                chat_id,
                response.len()
            );
            glowbot::bot_send::send_message(&tg_bot, chat, &response).await;
        }
        Ok(None) => {
            log::info!("main: no response for chat {}", chat_id);
        }
        Err(e) => {
            log::error!("Error processing message: {}", e);
        }
    }
}

/// Background heartbeat orchestrator. Discovers chats with tasks/reminders and spawns
/// a dedicated timer loop for each, using the chat's configured interval.
/// DMs use the global default; group chats can override.
/// Chats with due reminders are processed immediately even without heartbeat enabled.
async fn run_heartbeat_loop(bot: Arc<Mutex<GlowBot>>, tg_bot: Bot) {
    tokio::time::sleep(Duration::from_secs(30)).await;

    // Track which chats already have an active heartbeat task loop.
    let active: Arc<tokio::sync::Mutex<HashSet<String>>> =
        Arc::new(tokio::sync::Mutex::new(HashSet::new()));

    loop {
        // --- Scan for due reminders (fires even without heartbeat enabled) ---
        let reminder_chats: Vec<String> = {
            let inner = bot.lock().await;
            let state = inner.state.lock().await;
            let chats_dir = state.chats_dir();
            let mut result = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&chats_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.parse::<i64>().is_ok() && state.has_due_reminders(name) {
                                result.push(name.to_string());
                            }
                        }
                    }
                }
            }
            result
        };

        for chat_id in reminder_chats {
            let mut guard = active.lock().await;
            if !guard.contains(&chat_id) {
                guard.insert(chat_id.clone());
                drop(guard);
                let bot_clone = Arc::clone(&bot);
                let tg_clone = tg_bot.clone();
                let active_clone = Arc::clone(&active);
                let cid = chat_id.clone();
                tokio::spawn(async move {
                    let (state, git_repo, stop_signals) = {
                        let inner = bot_clone.lock().await;
                        (inner.state.clone(), inner.git_repo.clone(), inner.stop_signals.clone())
                    };
                    glowbot::bot::run_heartbeat_task(state, git_repo, stop_signals, &cid, tg_clone.clone()).await;
                    active_clone.lock().await.remove(&chat_id);
                });
            }
        }

        // --- Scan for chats with pending tasks ---
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

        let (state, git_repo, stop_signals) = {
            let inner = bot.lock().await;
            (inner.state.clone(), inner.git_repo.clone(), inner.stop_signals.clone())
        };

        glowbot::bot::run_heartbeat_task(state, git_repo, stop_signals, &chat_id, tg_bot.clone()).await;

        // After running tasks, check if there are any tasks remaining.
        // If the task list is empty, exit the loop so the chat becomes
        // eligible for re-discovery by the scheduler when new tasks arrive.
        let has_tasks = {
            let inner = bot.lock().await;
            let state = inner.state.lock().await;
            state.has_pending_tasks(&chat_id)
        };
        if !has_tasks {
            log::info!(
                "Heartbeat chat {}: no tasks remaining, exiting loop",
                chat_id
            );
            break;
        }

        // Sleep until next interval for this specific chat
        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
    }
}

/// Handle incoming Telegram callback queries (inline keyboard button presses).
async fn handle_callback(tg_bot: Bot, bot: Arc<Mutex<GlowBot>>, cb: teloxide::types::CallbackQuery) {
    let data = cb.data.clone().unwrap_or_default();
    let callback_id = cb.id.clone();

    log::info!("Callback query received: data={}", data);

    // Handle model browsing callbacks (model:, d:, s: prefixes)
    if data.starts_with("model:") || data.starts_with("d:") || data.starts_with("s:") {
        let _ = tg_bot.answer_callback_query(&callback_id).await;
        if let Some(msg) = &cb.message {
            let state = {
                let bot_inner = bot.lock().await;
                bot_inner.state.clone()
            };
            glowbot::bot::bot_models::handle_model_callback(
                &state, &data, &tg_bot, msg.chat().id, msg.id(),
            ).await;
        }
        return;
    }

    // Handle model change proposal callbacks (mdl: prefix)
    if data.starts_with("mdl:") {
        let _ = tg_bot.answer_callback_query(&callback_id).await;
        let state = {
            let bot_inner = bot.lock().await;
            bot_inner.state.clone()
        };
        let result = glowbot::bot::bot_dispatch::bot_dispatch_model::handle_model_callback_approval(
            &state, &data, Some(&tg_bot),
        )
        .await;
        if let Some((edit_text, _followup)) = result {
            if let Some(msg) = cb.message {
                let chat_id = msg.chat().id;
                let msg_id = msg.id();
                let escaped = glowbot::escape_v2_safe(&edit_text);
                let res = tg_bot
                    .edit_message_text(chat_id, msg_id, &escaped)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await;
                if let Err(e) = res {
                    log::warn!("Failed to edit model proposal message: {}", e);
                    let _ = tg_bot.edit_message_text(chat_id, msg_id, &edit_text).await;
                }
            }
        }
        return;
    }

    // Handle todo callbacks (todo:menu:N, todo:toggle:UUID, todo:close)
    if data.starts_with("todo:") {
        let _ = tg_bot.answer_callback_query(&callback_id).await;
        if let Some(msg) = &cb.message {
            let state = {
                let bot_inner = bot.lock().await;
                bot_inner.state.clone()
            };
            glowbot::bot::bot_todos::handle_todo_callback(
                &state, &data, &callback_id, &tg_bot, msg.chat().id, msg.id(),
            )
            .await;
        }
        return;
    }

    // Only handle config approval callbacks for now
    if !data.starts_with("cfg:") {
        // Silently ignore noop callbacks (page indicators) and unknown callbacks
        if data != "noop" {
            log::info!("Unknown callback data prefix, ignoring: {}", data);
        }
        let _ = tg_bot
            .answer_callback_query(&callback_id)
            .text(if data == "noop" { "" } else { "Unknown action" })
            .await;
        return;
    }

    // Answer the callback query immediately (Telegram requires this)
    let _ = tg_bot.answer_callback_query(&callback_id).await;

    // Process the config callback
    let state = {
        let bot_inner = bot.lock().await;
        bot_inner.state.clone()
    };

    let result = glowbot::bot::bot_dispatch::bot_dispatch_config::handle_config_callback(
        &state, &data, Some(&tg_bot),
    )
    .await;

    if let Some((edit_text, _followup_text)) = result {
        // Edit the original message to remove buttons and show result
        if let Some(msg) = cb.message {
            let chat_id = msg.chat().id;
            let msg_id = msg.id();
            let escaped = glowbot::escape_v2_safe(&edit_text);
            let result = tg_bot
                .edit_message_text(chat_id, msg_id, &escaped)
                .parse_mode(ParseMode::MarkdownV2)
                .await;
            if let Err(e) = result {
                log::warn!("Failed to edit message on callback: {}", e);
                // Try without MarkdownV2
                let _ = tg_bot
                    .edit_message_text(chat_id, msg_id, &edit_text)
                    .await;
            }
        }
    }
}
