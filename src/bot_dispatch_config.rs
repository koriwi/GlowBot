use super::BotState;
use crate::bot::PendingConfigChange;
use crate::config::Config;
use serde_json::Value;
use similar::{ChangeTag, TextDiff};
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::sync::Mutex;

/// Generate a unique short ID for callback data (limited to 64 bytes).
/// Uses a timestamp-based approach that's unique enough for our use case.
fn short_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", ts)
}

/// Tool: read_config — returns the current config as YAML.
pub(crate) async fn tool_read_config(state: &Arc<Mutex<BotState>>) -> String {
    let s = state.lock().await;
    match serde_yaml::to_string(&s.config) {
        Ok(yaml) => yaml,
        Err(e) => format!("Error serializing config: {}", e),
    }
}

/// Tool: edit_config — receives new YAML, validates, shows diff, asks for approval.
pub(crate) async fn tool_edit_config(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    args: &Value,
    tg_bot: Option<&teloxide::Bot>,
) -> String {
    let new_yaml = match args["config_yaml"].as_str() {
        Some(y) => y.to_string(),
        None => return "Error: config_yaml parameter required (the complete new config as YAML).".into(),
    };

    // Parse the proposed YAML
    let mut new_config: Config = match serde_yaml::from_str(&new_yaml) {
        Ok(c) => c,
        Err(e) => return format!("Error: invalid YAML — {}", e),
    };

    // Copy sensitive fields from current config (just in case the LLM omitted them)
    {
        let s = state.lock().await;
        new_config.telegram_token = s.config.telegram_token.clone();
        new_config.openrouter.api_key = s.config.openrouter.api_key.clone();
    }

    // Re-serialize to get canonical YAML for diff and storage
    let canonical_new_yaml = match serde_yaml::to_string(&new_config) {
        Ok(y) => y,
        Err(e) => return format!("Error: config is valid YAML but failed to re-serialize — {}", e),
    };

    // Get current config YAML for diff
    let old_yaml = {
        let s = state.lock().await;
        serde_yaml::to_string(&s.config).unwrap_or_default()
    };

    // Generate unified diff
    let diff = TextDiff::from_lines(&old_yaml, &canonical_new_yaml);
    let mut diff_text = String::new();
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        diff_text.push_str(sign);
        diff_text.push_str(change.value());
    }

    // If no changes, tell the LLM
    let has_changes = diff.iter_all_changes().any(|c| c.tag() != ChangeTag::Equal);
    if !has_changes {
        return "Config unchanged — no differences found.".into();
    }

    // Send Telegram message with diff and Accept/Deny buttons
    let bot = match tg_bot {
        Some(b) => b,
        None => return "Error: edit_config requires Telegram bot context to show approval buttons.".into(),
    };

    let chat = match chat_id.parse::<i64>() {
        Ok(c) => ChatId(c),
        Err(e) => return format!("Error: invalid chat_id '{}': {}", chat_id, e),
    };

    let pending_id = short_id();
    let accept_data = format!("cfg:{}:accept", pending_id);
    let deny_data = format!("cfg:{}:deny", pending_id);

    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("✅ Accept", accept_data),
        InlineKeyboardButton::callback("❌ Deny", deny_data),
    ]]);

    // Escape the diff for MarkdownV2. We wrap it in a code block so it renders as preformatted text.
    let header = format!("⚙️ *Config Change Proposal*\n\n```diff\n{}\n```", diff_text.trim());
    let escaped = crate::escape_v2_safe(&header);

    let sent_msg = match bot
        .send_message(chat, &escaped)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await
    {
        Ok(m) => m,
        Err(e) => return format!("Error sending approval message: {}", e),
    };

    // Store pending change in state
    {
        let mut s = state.lock().await;
        s.pending_config_changes.insert(
            pending_id.clone(),
            PendingConfigChange {
                chat_id: chat_id.to_string(),
                message_id: sent_msg.id.0,
                new_yaml: canonical_new_yaml,
            },
        );
    }

    format!(
        "Config change proposed to user for review. Message ID: {}. Status: waiting_for_approval.\n\
         If the user approves, the config will be applied automatically and the bot will restart.\n\
         If the user denies, you should ask the user what adjustments they'd like.\n\
         Do NOT ask the user about the config now — wait for their button response.",
        sent_msg.id.0
    )
}

/// Handle a config approval callback (called from the callback query handler in main.rs).
/// `data` is the callback data string, e.g. "cfg:<id>:accept" or "cfg:<id>:deny".
/// Returns Some((edit_text, followup_text)) to edit the original message, or None to leave it.
pub async fn handle_config_callback(
    state: &Arc<Mutex<BotState>>,
    data: &str,
) -> Option<(String, Option<String>)> {
    // (edit_text, optional_followup_text_for_llm)
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() != 3 || parts[0] != "cfg" {
        return None;
    }
    let pending_id = parts[1];
    let action = parts[2]; // "accept" or "deny"

    let pending = {
        let mut s = state.lock().await;
        s.pending_config_changes.remove(pending_id)
    };

    let pending = match pending {
        Some(p) => p,
        None => return Some(("⚠️ This config change has expired or was already processed.".into(), None)),
    };

    match action {
        "accept" => {
            // Save the new config
            let config_path = {
                let s = state.lock().await;
                s.config_path()
            };

            let parse_result: Result<Config, _> = serde_yaml::from_str(&pending.new_yaml);
            match parse_result {
                Ok(new_config) => {
                    if let Err(e) = new_config.save(&config_path) {
                        return Some((
                            format!("❌ Failed to save config: {}", e),
                            None,
                        ));
                    }

                    // Update in-memory config
                    {
                        let mut s = state.lock().await;
                        s.config = new_config;
                    }

                    // Git commit
                    // Note: git_repo is not easily accessible here.
                    // Auto-commit is best-effort anyway.

                    log::info!(
                        "Config change accepted by user in chat {}. Restarting...",
                        pending.chat_id
                    );

                    // Schedule restart — spawn a task so we can first send the confirmation
                    tokio::spawn(async {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        log::info!("Exiting for restart after config change.");
                        std::process::exit(0);
                    });

                    Some((
                        "✅ *Config Change Applied*\n\nThe bot is restarting to pick up the new configuration.".into(),
                        None,
                    ))
                }
                Err(e) => Some((
                    format!("❌ Config validation failed: {}", e),
                    None,
                )),
            }
        }
        "deny" => {
            log::info!(
                "Config change denied by user in chat {}",
                pending.chat_id
            );
            Some((
                "❌ *Config Change Denied*\n\nWhat would you like to change instead? The bot will ask for your feedback on the next message.".into(),
                Some("The user denied the proposed config change. On the next user message, ask what adjustments they'd like to make.".into()),
            ))
        }
        _ => Some(("⚠️ Unknown action.".into(), None)),
    }
}
