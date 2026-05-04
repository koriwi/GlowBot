use super::BotState;
use crate::git::GitRepo;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle a bot command (free function).
pub(crate) async fn handle_bot_command_impl(
    state: &Arc<Mutex<BotState>>,
    stop_signals: &Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    chat_id: &str,
    _user_id: &str,
    command: &crate::commands::Command,
    tg_bot: Option<&teloxide::Bot>,
    _git_repo: &GitRepo,
) -> anyhow::Result<Option<String>> {
    // /stop sets the stop signal and returns immediately
    if matches!(command, crate::commands::Command::Stop) {
        if let Ok(signals) = stop_signals.lock() {
            if let Some(signal) = signals.get(chat_id) {
                signal.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        return Ok(Some("Stop signal sent. Current operations will be cancelled.".into()));
    }

    let allowed = {
        let s = state.lock().await;
        let is_dm = !chat_id.starts_with('-');
        if is_dm {
            s.config.dm_config(chat_id).map(|d| d.commands_enabled).unwrap_or(false)
        } else {
            let chat_config = s.config.chat_config(chat_id);
            crate::commands::can_run_command(&chat_config)
        }
    };

    if !allowed {
        return Ok(Some("You are not authorized to run bot commands.".into()));
    }

    if matches!(command, crate::commands::Command::Tasks) {
        let s = state.lock().await;
        let list = crate::tasks::TaskList::load(&s.chats_dir(), chat_id).unwrap_or_default();
        let response = if list.tasks.is_empty() {
            "No pending tasks for this chat.".to_string()
        } else {
            let mut lines = vec![format!("*{} pending task(s):*", list.tasks.len())];
            for (i, t) in list.tasks.iter().enumerate() {
                lines.push(format!("{}. `{}` — {}", i + 1, t.id, t.description));
            }
            lines.join("\n")
        };
        return Ok(Some(response));
    }

    // /run triggers the heartbeat task agent immediately for this chat
    if matches!(command, crate::commands::Command::Run) {
        if let Some(bot) = tg_bot {
            let state_clone = Arc::clone(state);
            let git_clone = _git_repo.clone();
            let cid = chat_id.to_string();
            let tg_clone = bot.clone();
            tokio::spawn(async move {
                crate::bot::run_heartbeat_task(state_clone, git_clone, &cid, tg_clone).await;
            });
            return Ok(Some("🔄 Running task agent for this chat now...".into()));
        }
        return Ok(Some("Run command cannot be used in this context.".into()));
    }

    let response = {
        let mut s = state.lock().await;
        let usage = s.context_usage(chat_id);
        crate::commands::handle_command(command, &mut s.config, chat_id, &usage)
    };

    Ok(Some(response))
}
