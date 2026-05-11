use super::BotState;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::sync::Mutex;

/// Generate a unique short ID for model change callback data.
fn short_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", ts)
}

/// Tool: get_model_info — returns current model, specifier, override status, and metadata.
pub(crate) async fn tool_get_model_info(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
) -> String {
    let s = state.lock().await;
    let effective = s.effective_model(chat_id);
    let config_default = s.config.model_for_chat(chat_id);
    let has_override = s.model_overrides.contains_key(chat_id);

    // Determine the base model and any applied specifier
    let (base_model, specifier) = if let Some(pos) = effective.rfind(':') {
        let maybe_spec = &effective[pos + 1..];
        let valid: Vec<&str> = crate::openrouter::SPECIFIER_BUTTONS
            .iter()
            .map(|(s, _)| *s)
            .collect();
        if valid.contains(&maybe_spec) {
            (effective[..pos].to_string(), Some(maybe_spec.to_string()))
        } else {
            (effective.clone(), None)
        }
    } else {
        (effective.clone(), None)
    };

    // Get model metadata from cache
    let norm = crate::openrouter::normalize_model_id(&effective);
    let metadata = s.model_metadata.get(norm).map(|m| {
        serde_json::json!({
            "display_name": if m.name.is_empty() { &m.id } else { &m.name },
            "context_length": m.context_length,
            "is_free": m.pricing.is_free(),
            "pricing_per_million": m.pricing.format_per_million(),
        })
    });

    let available_specifiers: Vec<&str> = crate::openrouter::SPECIFIER_BUTTONS
        .iter()
        .map(|(s, _)| *s)
        .collect();

    serde_json::json!({
        "effective_model": effective,
        "base_model": base_model,
        "specifier": specifier,
        "config_default_model": config_default,
        "has_temporary_override": has_override,
        "available_specifiers": available_specifiers,
        "model_metadata": metadata,
    })
    .to_string()
}

/// Tool: propose_model_change — sends Accept/Deny dialog to the user.
pub(crate) async fn tool_propose_model_change(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    args: &serde_json::Value,
    tg_bot: Option<&teloxide::Bot>,
) -> String {
    let model_id = args["model_id"].as_str();
    let specifier = args["specifier"].as_str();

    // At least one must be provided
    if model_id.is_none() && specifier.is_none() {
        return "Error: at least one of model_id or specifier is required.".into();
    }

    // Build the proposed model string
    let proposed_model = match (model_id, specifier) {
        (Some(mid), Some(spec)) => {
            // Validate specifier
            let valid: Vec<&str> = crate::openrouter::SPECIFIER_BUTTONS
                .iter()
                .map(|(s, _)| *s)
                .collect();
            if !valid.contains(&spec) {
                let list = valid
                    .iter()
                    .map(|s| format!(":{}", s))
                    .collect::<Vec<_>>()
                    .join(", ");
                return format!("Error: unknown specifier ':{}'. Valid specifiers: {}", spec, list);
            }
            crate::openrouter::apply_specifier(mid, spec)
        }
        (Some(mid), None) => {
            // Check if the model_id already includes a specifier
            mid.to_string()
        }
        (None, Some(spec)) => {
            let valid: Vec<&str> = crate::openrouter::SPECIFIER_BUTTONS
                .iter()
                .map(|(s, _)| *s)
                .collect();
            if !valid.contains(&spec) {
                let list = valid
                    .iter()
                    .map(|s| format!(":{}", s))
                    .collect::<Vec<_>>()
                    .join(", ");
                return format!("Error: unknown specifier ':{}'. Valid specifiers: {}", spec, list);
            }
            let s = state.lock().await;
            let current = s.effective_model(chat_id);
            crate::openrouter::apply_specifier(&current, spec)
        }
        (None, None) => unreachable!(),
    };

    // Get the current model for comparison
    let current_model = {
        let s = state.lock().await;
        s.effective_model(chat_id)
    };

    // If proposed model is the same as current, no change needed
    if proposed_model == current_model {
        return format!(
            "The proposed model '{}' is already the active model for this chat. No change needed.",
            proposed_model
        );
    }

    let bot = match tg_bot {
        Some(b) => b,
        None => return "Error: propose_model_change requires Telegram bot context.".into(),
    };

    let chat = match chat_id.parse::<i64>() {
        Ok(c) => ChatId(c),
        Err(e) => return format!("Error: invalid chat_id '{}': {}", chat_id, e),
    };

    let pending_id = short_id();
    let accept_data = format!("mdl:{}:accept", pending_id);
    let deny_data = format!("mdl:{}:deny", pending_id);

    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("✅ Accept", accept_data),
        InlineKeyboardButton::callback("❌ Deny", deny_data),
    ]]);

    let spec_label = crate::openrouter::SPECIFIER_BUTTONS
        .iter()
        .find(|(s, _)| Some(*s) == specifier)
        .map(|(_, l)| format!(" ({})", l))
        .unwrap_or_default();

    let header_text = format!(
        "🔄 *Model Change Proposal*\n\nCurrent: `{}`\nProposed: `{}`{}\n\nSwitch to this model?",
        crate::escape_v2_safe(&current_model),
        crate::escape_v2_safe(&proposed_model),
        spec_label,
    );

    let sent_msg = match bot
        .send_message(chat, &header_text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await
    {
        Ok(m) => m,
        Err(e) => return format!("Error sending model proposal message: {}", e),
    };

    // Store pending model change in state
    {
        let mut s = state.lock().await;
        s.pending_model_changes.insert(
            pending_id.clone(),
            crate::bot::PendingModelChange {
                chat_id: chat_id.to_string(),
                message_id: sent_msg.id.0,
                proposed_model: proposed_model.clone(),
            },
        );
    }

    format!(
        "Model change proposal sent to user for review. Proposed: '{}'. Message ID: {}. Status: waiting_for_approval.\n\
         Do NOT ask the user about the model change now — wait for their Accept/Deny button response.\n\
         If the user accepts, the model will be temporarily switched and you'll be told the new model.\n\
         If the user denies, you should ask what model they'd prefer instead.",
        proposed_model, sent_msg.id.0
    )
}

/// Handle a model change approval callback (called from the callback query handler in main.rs).
/// `data` is the callback data string, e.g. "mdl:<id>:accept" or "mdl:<id>:deny".
/// Returns Some((edit_text, followup_text)) — edit_text replaces the original message,
/// followup_text is sent as a new message so the LLM sees it on the next processing cycle.
pub async fn handle_model_callback_approval(
    state: &Arc<Mutex<BotState>>,
    data: &str,
    tg_bot: Option<&teloxide::Bot>,
) -> Option<(String, Option<String>)> {
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() != 3 || parts[0] != "mdl" {
        return None;
    }
    let pending_id = parts[1];
    let action = parts[2]; // "accept" or "deny"

    let pending = {
        let mut s = state.lock().await;
        s.pending_model_changes.remove(pending_id)
    };

    let pending = match pending {
        Some(p) => p,
        None => return Some((
            "⚠️ This model change proposal has expired or was already processed.".into(),
            None,
        )),
    };

    match action {
        "accept" => {
            // Apply the model override
            {
                let mut s = state.lock().await;
                s.model_overrides
                    .insert(pending.chat_id.clone(), pending.proposed_model.clone());
            }

            log::info!(
                "Model change accepted in chat {}: {}",
                pending.chat_id,
                pending.proposed_model
            );

            let followup = format!(
                "Model change accepted. Now using `{}`.",
                pending.proposed_model
            );

            // Send followup as a new message so the LLM sees it
            if let Some(bot) = tg_bot {
                if let Ok(chat_id) = pending.chat_id.parse::<i64>() {
                    let _ = bot
                        .send_message(ChatId(chat_id), &followup)
                        .await;
                }
            }

            Some((
                format!(
                    "✅ *Model Changed*\n\nNow using: `{}`\n\nUse `/model_default` to reset to the config default.",
                    crate::escape_v2_safe(&pending.proposed_model)
                ),
                Some(followup),
            ))
        }
        "deny" => {
            log::info!(
                "Model change denied in chat {}",
                pending.chat_id
            );

            let followup = "Model change denied. The current model remains unchanged.".to_string();

            // Send followup as a new message so the LLM sees it
            if let Some(bot) = tg_bot {
                if let Ok(chat_id) = pending.chat_id.parse::<i64>() {
                    let _ = bot
                        .send_message(ChatId(chat_id), &followup)
                        .await;
                }
            }

            Some((
                "❌ *Model Change Denied*\n\nThe current model remains unchanged.".into(),
                Some(followup),
            ))
        }
        _ => Some(("⚠️ Unknown action.".into(), None)),
    }
}
