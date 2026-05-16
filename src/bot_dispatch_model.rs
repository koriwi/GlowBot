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
pub(crate) async fn tool_get_model_info(state: &Arc<Mutex<BotState>>, chat_id: &str) -> String {
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
                return format!(
                    "Error: unknown specifier ':{}'. Valid specifiers: {}",
                    spec, list
                );
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
                return format!(
                    "Error: unknown specifier ':{}'. Valid specifiers: {}",
                    spec, list
                );
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
        None => {
            return Some((
                "⚠️ This model change proposal has expired or was already processed.".into(),
                None,
            ))
        }
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
                    let _ = bot.send_message(ChatId(chat_id), &followup).await;
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
            log::info!("Model change denied in chat {}", pending.chat_id);

            let followup = "Model change denied. The current model remains unchanged.".to_string();

            // Send followup as a new message so the LLM sees it
            if let Some(bot) = tg_bot {
                if let Ok(chat_id) = pending.chat_id.parse::<i64>() {
                    let _ = bot.send_message(ChatId(chat_id), &followup).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::BotState;
    use crate::llm::mock::MockLlmBackend;
    use crate::openrouter::ModelInfo;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    async fn make_state() -> (Arc<Mutex<BotState>>, TempDir) {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let config = crate::config::basic_config();
        config.save(&data_dir.join("config.yaml")).unwrap();
        let mock_llm: Arc<dyn crate::llm::LlmBackend> = Arc::new(MockLlmBackend::new());
        let state = Arc::new(Mutex::new(BotState {
            config,
            skills: std::collections::HashMap::new(),
            llm: mock_llm,
            data_dir: data_dir.clone(),
            db: crate::db::Database::open_in_memory().unwrap(),
            mcp_tools: vec![],
            _mcp_services: vec![],
            mcp_peers: std::collections::HashMap::new(),
            model_metadata: std::collections::HashMap::new(),
            model_order: vec![],
            last_usage: std::collections::HashMap::new(),
            pending_config_changes: std::collections::HashMap::new(),
            pending_model_changes: std::collections::HashMap::new(),
            model_overrides: std::collections::HashMap::new(),
            last_browse_cb: std::collections::HashMap::new(),
        }));
        (state, dir)
    }

    #[test]
    fn test_short_id_non_empty() {
        let id = short_id();
        assert!(!id.is_empty());
    }

    #[test]
    fn test_short_id_unique() {
        let id1 = short_id();
        let id2 = short_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_short_id_is_hex() {
        let id = short_id();
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ─── tool_get_model_info ─────────────────────────────────────

    #[tokio::test]
    async fn test_get_model_info_default() {
        let (state, _dir) = make_state().await;
        let result = tool_get_model_info(&state, "-123").await;
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        // Default config uses "test/model"
        assert_eq!(v["effective_model"], "test/model");
        assert_eq!(v["config_default_model"], "test/model");
        assert!(!v["has_temporary_override"].as_bool().unwrap());
        assert!(v["specifier"].is_null());
    }

    #[tokio::test]
    async fn test_get_model_info_with_override() {
        let (state, _dir) = make_state().await;
        {
            let mut s = state.lock().await;
            s.model_overrides
                .insert("-123".into(), "openai/gpt-4o".into());
        }
        let result = tool_get_model_info(&state, "-123").await;
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["effective_model"], "openai/gpt-4o");
        assert_eq!(v["config_default_model"], "test/model");
        assert!(v["has_temporary_override"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_get_model_info_with_specifier() {
        let (state, _dir) = make_state().await;
        {
            let mut s = state.lock().await;
            s.model_overrides
                .insert("-123".into(), "openai/gpt-4o:nitro".into());
        }
        let result = tool_get_model_info(&state, "-123").await;
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["effective_model"], "openai/gpt-4o:nitro");
        assert_eq!(v["base_model"], "openai/gpt-4o");
        assert_eq!(v["specifier"], "nitro");
    }

    #[tokio::test]
    async fn test_get_model_info_with_unknown_specifier() {
        let (state, _dir) = make_state().await;
        {
            let mut s = state.lock().await;
            s.model_overrides
                .insert("-123".into(), "openai/gpt-4o:unknown".into());
        }
        let result = tool_get_model_info(&state, "-123").await;
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        // "unknown" is not a valid specifier, should not be split
        assert_eq!(v["base_model"], "openai/gpt-4o:unknown");
        assert!(v["specifier"].is_null());
    }

    #[tokio::test]
    async fn test_get_model_info_with_cached_metadata() {
        let (state, _dir) = make_state().await;
        {
            let mut s = state.lock().await;
            s.model_metadata.insert(
                "test/model".into(),
                ModelInfo {
                    id: "test/model".into(),
                    name: "Test Model Display".into(),
                    context_length: 128000,
                    created: 0,
                    pricing: crate::openrouter::ModelPricing {
                        prompt: "0.5".into(),
                        completion: "1.5".into(),
                        request: "0".into(),
                    },
                    architecture: crate::openrouter::ModelArchitecture {
                        input_modalities: vec!["text".into()],
                    },
                },
            );
        }
        let result = tool_get_model_info(&state, "-123").await;
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let meta = &v["model_metadata"];
        assert_eq!(meta["display_name"], "Test Model Display");
        assert_eq!(meta["context_length"], 128000);
        assert_eq!(meta["is_free"], false);
    }

    // ─── tool_propose_model_change ───────────────────────────────

    #[tokio::test]
    async fn test_propose_model_change_no_args() {
        let (state, _dir) = make_state().await;
        let args = json!({});
        let result = tool_propose_model_change(&state, "-123", &args, None).await;
        assert!(result.contains("at least one"));
    }

    #[tokio::test]
    async fn test_propose_model_change_invalid_specifier() {
        let (state, _dir) = make_state().await;
        let args = json!({"specifier": "invalid"});
        let result = tool_propose_model_change(&state, "-123", &args, None).await;
        assert!(result.contains("unknown specifier"));
    }

    #[tokio::test]
    async fn test_propose_model_change_same_as_current() {
        let (state, _dir) = make_state().await;
        // Default model is "test/model"
        let args = json!({"model_id": "test/model"});
        let result = tool_propose_model_change(&state, "-123", &args, None).await;
        assert!(result.contains("already the active model"));
    }

    #[tokio::test]
    async fn test_propose_model_change_no_tg_bot() {
        let (state, _dir) = make_state().await;
        let args = json!({"model_id": "openai/gpt-4o"});
        let result = tool_propose_model_change(&state, "-123", &args, None).await;
        assert!(result.contains("requires Telegram bot context"));
    }

    #[tokio::test]
    async fn test_propose_model_change_only_specifier_invalid() {
        let (state, _dir) = make_state().await;
        let args = json!({"specifier": "bad"});
        let result = tool_propose_model_change(&state, "-123", &args, None).await;
        assert!(result.contains("unknown specifier"));
    }

    #[tokio::test]
    async fn test_propose_model_change_with_specifier_invalid_chat_id() {
        let (state, _dir) = make_state().await;
        let args = json!({"model_id": "openai/gpt-4o"});
        let result = tool_propose_model_change(&state, "not-a-number", &args, None).await;
        assert!(result.starts_with("Error"));
    }

    // ─── handle_model_callback_approval ──────────────────────────

    #[tokio::test]
    async fn test_handle_callback_invalid_format() {
        let (state, _dir) = make_state().await;
        let result = handle_model_callback_approval(&state, "garbage", None).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_handle_callback_wrong_prefix() {
        let (state, _dir) = make_state().await;
        let result = handle_model_callback_approval(&state, "cfg:abc:accept", None).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_handle_callback_expired() {
        let (state, _dir) = make_state().await;
        let result = handle_model_callback_approval(&state, "mdl:nonexistent:accept", None).await;
        let (text, followup) = result.unwrap();
        assert!(text.contains("expired"));
        assert!(followup.is_none());
    }

    #[tokio::test]
    async fn test_handle_callback_accept() {
        let (state, _dir) = make_state().await;
        // Register a pending model change
        let pending_id = {
            let mut s = state.lock().await;
            let id = short_id();
            s.pending_model_changes.insert(
                id.clone(),
                crate::bot::PendingModelChange {
                    chat_id: "-123".into(),
                    message_id: 42,
                    proposed_model: "openai/gpt-4o".into(),
                },
            );
            id
        };
        let cb_data = format!("mdl:{}:accept", pending_id);
        let result = handle_model_callback_approval(&state, &cb_data, None).await;
        let (text, followup) = result.unwrap();
        assert!(text.contains("Model Changed"));
        // Followup exists even if tg_bot is None (the text is generated, send is best-effort)
        assert!(followup.is_some());
        assert!(followup.unwrap().contains("accepted"));
        // Override should be applied
        let s = state.lock().await;
        assert!(s.model_overrides.get("-123").is_some());
    }

    #[tokio::test]
    async fn test_handle_callback_deny() {
        let (state, _dir) = make_state().await;
        let pending_id = {
            let mut s = state.lock().await;
            let id = short_id();
            s.pending_model_changes.insert(
                id.clone(),
                crate::bot::PendingModelChange {
                    chat_id: "-123".into(),
                    message_id: 42,
                    proposed_model: "openai/gpt-4o".into(),
                },
            );
            id
        };
        let cb_data = format!("mdl:{}:deny", pending_id);
        let result = handle_model_callback_approval(&state, &cb_data, None).await;
        let (text, followup) = result.unwrap();
        assert!(text.contains("Denied"));
        assert!(followup.is_some());
        // Override should NOT be applied
        let s = state.lock().await;
        assert!(!s.model_overrides.contains_key("-123"));
    }

    #[tokio::test]
    async fn test_handle_callback_unknown_action() {
        let (state, _dir) = make_state().await;
        let pending_id = {
            let mut s = state.lock().await;
            let id = short_id();
            s.pending_model_changes.insert(
                id.clone(),
                crate::bot::PendingModelChange {
                    chat_id: "-123".into(),
                    message_id: 42,
                    proposed_model: "openai/gpt-4o".into(),
                },
            );
            id
        };
        let cb_data = format!("mdl:{}:bogus", pending_id);
        let result = handle_model_callback_approval(&state, &cb_data, None).await;
        let (text, followup) = result.unwrap();
        assert!(text.contains("Unknown action"));
        assert!(followup.is_none());
    }
}
