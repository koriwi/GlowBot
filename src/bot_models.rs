use super::BotState;
use crate::openrouter::ModelInfo;
use crate::openrouter::OpenRouterClient;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use tokio::sync::Mutex;

const MODELS_PER_PAGE: usize = 6;
/// Telegram's max callback data length in bytes.
const MAX_CALLBACK_BYTES: usize = 64;

/// Build a safe detail callback, shortening the prefix to `d:` if the full
/// `model:detail:{id}` would exceed Telegram's 64-byte callback data limit.
fn detail_cb(model_id: &str) -> String {
    let full = format!("model:detail:{}", model_id);
    if full.len() <= MAX_CALLBACK_BYTES {
        full
    } else {
        format!("d:{}", model_id)
    }
}

/// Build a safe select callback, shortening the prefix to `s:` if the full
/// `model:select:{id}` would exceed Telegram's 64-byte callback data limit.
fn select_cb(model_id: &str) -> String {
    let full = format!("model:select:{}", model_id);
    if full.len() <= MAX_CALLBACK_BYTES {
        full
    } else {
        format!("s:{}", model_id)
    }
}

/// Send the main model browse menu as a new message.
pub(crate) async fn send_model_menu(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    bot: teloxide::Bot,
) -> anyhow::Result<()> {
    let chat = ChatId(chat_id.parse::<i64>()?);

    // Fetch models if not cached
    fetch_models_if_needed(state).await?;

    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("🆓 Free Models", "model:browse:free:0")],
        vec![InlineKeyboardButton::callback("🏭 By Provider", "model:provider_list")],
        vec![InlineKeyboardButton::callback("🆕 Newest", "model:browse:newest:0")],
        vec![InlineKeyboardButton::callback("🔥 Popular", "model:browse:popular:0")],
    ]);

    let text = format_model_status(state, chat_id).await;

    bot.send_message(chat, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Handle a model-related callback (called from main.rs callback handler).
pub async fn handle_model_callback(
    state: &Arc<Mutex<BotState>>,
    data: &str,
    bot: &teloxide::Bot,
    chat_id: ChatId,
    msg_id: MessageId,
) {
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() < 2 {
        return;
    }

    match parts[1] {
        "menu" => {
            if let Err(e) = edit_to_menu(state, chat_id, bot, msg_id).await {
                log::error!("Failed to edit to menu: {}", e);
            }
        }
        "browse" => {
            let category = parts.get(2).unwrap_or(&"free");
            // For provider browsing, the format is model:browse:provider:<name>:<page>
            // For other categories, it's model:browse:<category>:<page>
            let (provider, page): (Option<String>, usize) = if *category == "provider" {
                let p = parts.get(3).map(|s| s.to_string());
                let pg = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
                (p, pg)
            } else {
                let pg = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                (None, pg)
            };
            if let Err(e) = edit_to_browse(
                state, chat_id, bot, msg_id, category, page, provider.as_deref(),
            )
            .await
            {
                log::error!("Failed to edit browse page {}: {}", page, e);
            }
        }
        "detail" | "d" => {
            // Use splitn(3) for "model:detail:" (keeps model IDs with colons intact,
            // e.g. "google/gemma-4-31b-it:free"). For short prefix "d:", use
            // splitn(2).
            let model_id = if parts[1] == "detail" {
                data.splitn(3, ':').nth(2).unwrap_or("")
            } else {
                data.splitn(2, ':').nth(1).unwrap_or("")
            };
            if let Err(e) = edit_to_detail(state, chat_id, bot, msg_id, model_id).await {
                log::error!("Failed to edit to detail for model '{}': {}", model_id, e);
            }
        }
        "select" | "s" => {
            // Same split logic as detail: "model:select:" uses splitn(3),
            // short prefix "s:" uses splitn(2).
            let model_id = if parts[1] == "select" {
                data.splitn(3, ':').nth(2).unwrap_or("")
            } else {
                data.splitn(2, ':').nth(1).unwrap_or("")
            };
            if let Err(e) = select_model(state, chat_id, bot, msg_id, model_id).await {
                log::error!("Failed to select model '{}': {}", model_id, e);
            }
        }
        "provider_list" => {
            let _ = edit_to_provider_list(state, chat_id, bot, msg_id).await;
        }
        _ => {}
    }
}

/// Fetch models from OpenRouter if not already cached.
async fn fetch_models_if_needed(state: &Arc<Mutex<BotState>>) -> anyhow::Result<()> {
    let needs_fetch = {
        let s = state.lock().await;
        s.model_metadata.is_empty()
    };
    if needs_fetch {
        let api_key = {
            let s = state.lock().await;
            s.config.openrouter.api_key.clone()
        };
        let client = OpenRouterClient::new(api_key);
        let models = client.fetch_models().await?;
        let mut s = state.lock().await;
        for m in models {
            s.model_order.push(m.id.clone());
            s.model_metadata.insert(m.id.clone(), m);
        }
    }
    Ok(())
}

/// Edit message to show the main menu.
async fn edit_to_menu(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
) -> anyhow::Result<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("🆓 Free Models", "model:browse:free:0")],
        vec![InlineKeyboardButton::callback("🏭 By Provider", "model:provider_list")],
        vec![InlineKeyboardButton::callback("🆕 Newest", "model:browse:newest:0")],
        vec![InlineKeyboardButton::callback("🔥 Popular", "model:browse:popular:0")],
    ]);

    let text = format_model_status(state, &chat_id.to_string()).await;

    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Edit message to show a list of providers.
async fn edit_to_provider_list(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
) -> anyhow::Result<()> {
    let s = state.lock().await;
    let mut providers: Vec<String> = s
        .model_metadata
        .values()
        .map(|m| m.provider().to_string())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    providers.sort();
    drop(s);

    let total = providers.len();
    if total == 0 {
        bot.edit_message_text(chat_id, msg_id, "No models cached\\.")
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        return Ok(());
    }

    let max_display = 30.min(providers.len());
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for provider in providers.iter().take(max_display) {
        let label = if provider.len() > 20 {
            format!("{}…", &provider[..19])
        } else {
            provider.clone()
        };
        rows.push(vec![InlineKeyboardButton::callback(
            label,
            format!("model:browse:provider:{}:0", provider),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "↩ Back",
        "model:menu",
    )]);

    let keyboard = InlineKeyboardMarkup::new(rows);
    let text = format!(
        "*Select Provider* \\({} available\\)",
        total
    );

    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Edit message to show a browsable list of models.
async fn edit_to_browse(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
    category: &str,
    page: usize,
    provider: Option<&str>,
) -> anyhow::Result<()> {
    let s = state.lock().await;
    let all_models: Vec<&ModelInfo> = s.model_metadata.values().collect();

    // Filter models based on category
    let mut filtered: Vec<&&ModelInfo> = all_models.iter().filter(|m| {
        match category {
            "free" => m.pricing.is_free(),
            "provider" => {
                if let Some(p) = provider {
                    if p == "all" {
                        return true;
                    }
                    m.provider() == p
                } else {
                    true
                }
            }
            _ => true,
        }
    }).collect();

    // Sort
    match category {
        "newest" => filtered.sort_by(|a, b| b.created.cmp(&a.created)),
        "popular" => {
            // Sort by API return order (model_order)
            filtered.sort_by(|a, b| {
                let pos_a = s.model_order.iter().position(|id| *id == a.id);
                let pos_b = s.model_order.iter().position(|id| *id == b.id);
                pos_a.cmp(&pos_b)
            });
        }
        _ => filtered.sort_by(|a, b| a.name.cmp(&b.name)),
    }

    let total = filtered.len();
    let total_pages = if total == 0 { 1 } else { (total + MODELS_PER_PAGE - 1) / MODELS_PER_PAGE };
    let clamped_page = page.min(total_pages.saturating_sub(1));

    let start = clamped_page * MODELS_PER_PAGE;
    let end = (start + MODELS_PER_PAGE).min(total);
    let page_models: Vec<(String, String)> = filtered[start..end]
        .iter()
        .map(|m| (m.id.clone(), if m.name.is_empty() { m.id.clone() } else { m.name.clone() }))
        .collect();
    drop(s);

    if total == 0 {
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("↩ Back", "model:menu"),
        ]]);
        bot.edit_message_text(chat_id, msg_id, "No models found for this category\\.")
            .parse_mode(ParseMode::MarkdownV2)
            .reply_markup(keyboard)
            .await?;
        return Ok(());
    }

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for (m_id, m_name) in &page_models {
        let label = if m_name.len() > 40 {
            format!("{}…", &m_name[..39])
        } else {
            m_name.clone()
        };
        rows.push(vec![InlineKeyboardButton::callback(
            label,
            detail_cb(m_id),
        )]);
    }

    // Navigation row
    let mut nav_row: Vec<InlineKeyboardButton> = Vec::new();
    if clamped_page > 0 {
        let prev_cb = match provider {
            Some(p) => format!("model:browse:{}:{}:{}", category, p, clamped_page - 1),
            None => format!("model:browse:{}:{}", category, clamped_page - 1),
        };
        nav_row.push(InlineKeyboardButton::callback("◀ Prev", prev_cb));
    }
    if total_pages > 1 {
        nav_row.push(InlineKeyboardButton::callback(
            format!("{}/{}", clamped_page + 1, total_pages),
            "noop",
        ));
    }
    if clamped_page + 1 < total_pages {
        let next_cb = match provider {
            Some(p) => format!("model:browse:{}:{}:{}", category, p, clamped_page + 1),
            None => format!("model:browse:{}:{}", category, clamped_page + 1),
        };
        nav_row.push(InlineKeyboardButton::callback("Next ▶", next_cb));
    }
    if !nav_row.is_empty() {
        rows.push(nav_row);
    }

    // Back button
    let back_cb = if provider.is_some() { "model:provider_list" } else { "model:menu" };
    rows.push(vec![InlineKeyboardButton::callback("↩ Back", back_cb)]);

    let keyboard = InlineKeyboardMarkup::new(rows);

    let category_label = match category {
        "free" => "Free Models",
        "provider" => {
            if let Some(p) = provider {
                if p == "all" { "All Models" } else { p }
            } else {
                "Provider"
            }
        }
        "newest" => "Newest",
        "popular" => "Popular",
        _ => "Models",
    };

    let text = format!(
        "*{}* \\({} models\\)",
        category_label, total
    );

    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Edit message to show model details with a Select button.
async fn edit_to_detail(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
    model_id: &str,
) -> anyhow::Result<()> {
    let s = state.lock().await;
    let model_data = s.model_metadata.get(model_id).map(|m| {
        (
            if m.name.is_empty() { m.id.clone() } else { m.name.clone() },
            m.context_length,
            m.pricing.is_free(),
            m.pricing.prompt.clone(),
            m.pricing.completion.clone(),
        )
    });
    let is_selected = s.model_overrides.get(&chat_id.to_string()).map(|m| m == model_id).unwrap_or(false);
    let current_config_model = s.config.model_for_chat(&chat_id.to_string()).to_string();
    drop(s);

    let (display_name, context_len, pricing_text) = match model_data {
        Some((name, ctx, is_free, prompt, completion)) => {
            let ctx_str = if ctx == 0 {
                "unknown".to_string()
            } else if ctx >= 1_000_000 {
                format!("{:.1}M", ctx as f64 / 1_000_000.0)
            } else {
                format!("{}k", ctx / 1000)
            };
            let pricing = if is_free {
                "🆓 Free".to_string()
            } else {
                format!(
                    "💲 {}/{} per token",
                    prompt, completion
                )
            };
            (name, ctx_str, pricing)
        }
        None => (model_id.to_string(), "unknown".to_string(), "unknown".to_string()),
    };

    let is_config_default = model_id == current_config_model;

    // Escape all variable text for MarkdownV2. The model name/id may contain
    // special chars like (, ), -, etc. that break Telegram's parser.
    let escaped_display = crate::escape_v2_safe(&display_name);
    let escaped_context = crate::escape_v2_safe(&context_len);
    let escaped_pricing = crate::escape_v2_safe(&pricing_text);
    let text = format!(
        "*{}*\nID: `{}`\nContext: {}\nPricing: {}{}{}",
        escaped_display,
        model_id,
        escaped_context,
        escaped_pricing,
        if is_config_default { "\n📌 Config default" } else { "" },
        if is_selected { "\n✅ Currently selected" } else { "" },
    );

    let mut rows = vec![vec![InlineKeyboardButton::callback(
        if is_selected { "✅ Selected" } else { "📌 Select Model" },
        select_cb(model_id),
    )]];
    rows.push(vec![InlineKeyboardButton::callback(
        "↩ Back",
        "model:menu",
    )]);

    let keyboard = InlineKeyboardMarkup::new(rows);

    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Select a model (set temporary override) and show confirmation.
async fn select_model(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
    model_id: &str,
) -> anyhow::Result<()> {
    {
        let mut s = state.lock().await;
        s.model_overrides
            .insert(chat_id.to_string(), model_id.to_string());
    }

    let display_name = {
        let s = state.lock().await;
        s.model_metadata
            .get(model_id)
            .map(|m| if m.name.is_empty() { m.id.clone() } else { m.name.clone() })
            .unwrap_or_else(|| model_id.to_string())
    };

    let text = format!(
        "✅ *Model Selected*\n\n`{}`\n\nUse `/model\\_default` to reset to config default\\.",
        display_name
    );

    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("🔙 Back to Menu", "model:menu")],
    ]);

    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Format the current model status line for the menu header.
async fn format_model_status(state: &Arc<Mutex<BotState>>, chat_id: &str) -> String {
    let s = state.lock().await;
    let current = s.effective_model(chat_id);
    let config_default = s.config.model_for_chat(chat_id);
    let has_override = s.model_overrides.contains_key(chat_id);

    if has_override {
        format!(
            "🎯 *Current model:* `{}` \\(override\\)\n📌 Config default: `{}`\n\nBrowse models:",
            current, config_default
        )
    } else {
        format!(
            "🎯 *Current model:* `{}`\n\nBrowse models:",
            current
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detail_cb_short_id_uses_full_prefix() {
        let id = "openai/gpt-4";
        let cb = detail_cb(id);
        assert_eq!(cb, "model:detail:openai/gpt-4");
    }

    #[test]
    fn test_detail_cb_long_id_uses_short_prefix() {
        // model:detail: is 13 chars, so an ID of 52+ chars triggers the short prefix
        let id = "a".repeat(52);
        let cb = detail_cb(&id);
        assert!(cb.starts_with("d:"));
        assert!(!cb.starts_with("model:detail:"));
        assert!(cb.len() <= 64);
    }

    #[test]
    fn test_select_cb_short_id_uses_full_prefix() {
        let id = "openai/gpt-4";
        let cb = select_cb(id);
        assert_eq!(cb, "model:select:openai/gpt-4");
    }

    #[test]
    fn test_select_cb_long_id_uses_short_prefix() {
        // model:select: is 13 chars, so an ID of 52+ chars triggers the short prefix
        let id = "a".repeat(52);
        let cb = select_cb(&id);
        assert!(cb.starts_with("s:"));
        assert!(!cb.starts_with("model:select:"));
        assert!(cb.len() <= 64);
    }
}
