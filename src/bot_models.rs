use super::BotState;
use crate::openrouter::ModelInfo;
use crate::openrouter::OpenRouterClient;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use tokio::sync::Mutex;

const MODELS_PER_PAGE: usize = 6;
const PROVIDERS_PER_PAGE: usize = 20;
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

fn provider_menu_rows(has_codex: bool) -> Vec<Vec<InlineKeyboardButton>> {
    let mut rows = vec![vec![InlineKeyboardButton::callback(
        "🌐 OpenRouter Models",
        "model:provider:openrouter",
    )]];
    if has_codex {
        rows.push(vec![InlineKeyboardButton::callback(
            "🤖 Codex Subscription Models",
            "model:provider:codex",
        )]);
    }
    rows
}

fn parse_provider(value: &str) -> Option<crate::config::LlmProvider> {
    match value {
        "openrouter" => Some(crate::config::LlmProvider::Openrouter),
        "codex" => Some(crate::config::LlmProvider::Codex),
        _ => None,
    }
}

fn is_known_codex_model(model_id: &str) -> bool {
    crate::codex::KNOWN_MODELS
        .iter()
        .any(|(id, _)| *id == model_id)
}

/// Send a provider chooser before browsing models. A selection is temporary and
/// applies to this chat until `/model_default` or restart.
pub(crate) async fn send_provider_model_menu(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    bot: teloxide::Bot,
) -> anyhow::Result<()> {
    let chat = ChatId(chat_id.parse::<i64>()?);
    let has_codex = state.lock().await.config.codex.is_some();
    let text = format!(
        "{}\n\nChoose a provider to browse:",
        format_model_status(state, chat_id).await
    );
    bot.send_message(chat, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(InlineKeyboardMarkup::new(provider_menu_rows(has_codex)))
        .await?;
    Ok(())
}

/// Cache the known Codex model metadata for interactive selection and status.
pub(crate) async fn cache_codex_models(state: &Arc<Mutex<BotState>>) {
    let mut state = state.lock().await;
    for (id, _) in crate::codex::KNOWN_MODELS {
        if !state.model_metadata.contains_key(*id) {
            state.model_order.push((*id).into());
            state
                .model_metadata
                .insert((*id).into(), crate::codex::model_info(id));
        }
    }
}

/// Cache metadata for a directly specified Codex model.
pub(crate) fn cache_codex_model(state: &mut BotState, model: &str) {
    if !state.model_metadata.contains_key(model) {
        state.model_order.push(model.into());
        state
            .model_metadata
            .insert(model.into(), crate::codex::model_info(model));
    }
}

/// Send Codex model info with picker access (for `/model` without arguments).
pub(crate) async fn send_codex_model_info(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    bot: teloxide::Bot,
) -> anyhow::Result<()> {
    cache_codex_models(state).await;
    let chat = ChatId(chat_id.parse::<i64>()?);
    let text = format!(
        "{}\n\nSet model: `/model <codex-model>`",
        format_model_status(state, chat_id).await
    );
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "🔍 Browse Codex Models",
            "model:provider:codex",
        )],
        vec![InlineKeyboardButton::callback(
            "🔁 Change Provider",
            "model:menu",
        )],
    ]);
    bot.send_message(chat, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

/// Send a model info message with specifier buttons (for /model without args).
pub(crate) async fn send_model_info(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    bot: teloxide::Bot,
) -> anyhow::Result<()> {
    let chat = ChatId(chat_id.parse::<i64>()?);

    let status_text = format_model_status(state, chat_id).await;
    let help = "\nSet model: `/model <model-id>`\nSwitch routing:";
    let text = format!("{}{}", status_text, help);

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    let mut btn_row: Vec<InlineKeyboardButton> = Vec::new();
    for (spec, label) in crate::openrouter::SPECIFIER_BUTTONS {
        btn_row.push(InlineKeyboardButton::callback(
            label.to_string(),
            format!("model:spec:{}", spec),
        ));
    }
    rows.push(btn_row);
    rows.push(vec![InlineKeyboardButton::callback(
        "🔍 Browse All Models",
        "model:provider:openrouter",
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "🔁 Change Provider",
        "model:menu",
    )]);

    let keyboard = InlineKeyboardMarkup::new(rows);

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
            if let Err(e) = edit_to_provider_menu(state, chat_id, bot, msg_id).await {
                log::error!("Failed to edit to provider menu: {}", e);
            }
        }
        "provider" => {
            let Some(provider) = parts.get(2).and_then(|value| parse_provider(value)) else {
                return;
            };
            {
                let mut s = state.lock().await;
                if provider == crate::config::LlmProvider::Codex && s.config.codex.is_none() {
                    return;
                }
                s.picker_providers.insert(chat_id.to_string(), provider);
            }
            let result = match provider {
                crate::config::LlmProvider::Openrouter => {
                    edit_to_menu(state, chat_id, bot, msg_id).await
                }
                crate::config::LlmProvider::Codex => {
                    edit_to_codex_menu(state, chat_id, bot, msg_id).await
                }
            };
            if let Err(error) = result {
                log::error!("Failed to open provider model menu: {}", error);
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

            // Save the browse callback so the detail view's Back button can return here.
            {
                let mut s = state.lock().await;
                s.last_browse_cb
                    .insert(chat_id.to_string(), data.to_string());
            }

            if let Err(e) = edit_to_browse(
                state,
                chat_id,
                bot,
                msg_id,
                category,
                page,
                provider.as_deref(),
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
                data.split_once(':').map(|x| x.1).unwrap_or("")
            };
            if let Err(e) = edit_to_detail(state, chat_id, bot, msg_id, model_id).await {
                log::error!("Failed to edit to detail for model '{}': {}", model_id, e);
            }
        }
        "browseback" => {
            // Replay the last browse callback so Back from detail view returns
            // to the originating browse page instead of the root menu.
            let cb_data = {
                let s = state.lock().await;
                s.last_browse_cb.get(&chat_id.to_string()).cloned()
            };
            match cb_data {
                Some(cb) => {
                    Box::pin(handle_model_callback(state, &cb, bot, chat_id, msg_id)).await;
                }
                None => {
                    if let Err(e) = edit_to_provider_menu(state, chat_id, bot, msg_id).await {
                        log::error!("Failed to edit to provider menu: {}", e);
                    }
                }
            }
        }
        "select" | "s" => {
            // Same split logic as detail: "model:select:" uses splitn(3),
            // short prefix "s:" uses splitn(2).
            let model_id = if parts[1] == "select" {
                data.splitn(3, ':').nth(2).unwrap_or("")
            } else {
                data.split_once(':').map(|x| x.1).unwrap_or("")
            };
            if let Err(e) = select_model(state, chat_id, bot, msg_id, model_id).await {
                log::error!("Failed to select model '{}': {}", model_id, e);
            }
        }
        "provider_list" => {
            let page = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let _ = edit_to_provider_list(state, chat_id, bot, msg_id, page).await;
        }
        "spec" => {
            let specifier = parts.get(2).unwrap_or(&"");
            if let Err(e) = handle_spec_callback(state, chat_id, bot, msg_id, specifier).await {
                log::error!("Failed to handle spec callback '{}': {}", specifier, e);
            }
        }
        _ => {}
    }
}

/// Fetch models from OpenRouter if not already cached.
async fn fetch_models_if_needed(state: &Arc<Mutex<BotState>>) -> anyhow::Result<()> {
    let needs_fetch = {
        let s = state.lock().await;
        // Codex metadata shares this cache; it must not make the OpenRouter
        // catalog look loaded when a user switches providers in `/models`.
        !s.model_metadata
            .values()
            .any(|model| !model.name.starts_with("OpenAI Codex:"))
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

async fn edit_to_provider_menu(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
) -> anyhow::Result<()> {
    let has_codex = state.lock().await.config.codex.is_some();
    let text = format!(
        "{}\n\nChoose a provider to browse:",
        format_model_status(state, &chat_id.to_string()).await
    );
    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(InlineKeyboardMarkup::new(provider_menu_rows(has_codex)))
        .await?;
    Ok(())
}

/// Edit message to show the main OpenRouter menu.
async fn edit_to_menu(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
) -> anyhow::Result<()> {
    fetch_models_if_needed(state).await?;
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "🆓 Free Models",
            "model:browse:free:0",
        )],
        vec![InlineKeyboardButton::callback(
            "🏭 By Provider",
            "model:provider_list:0",
        )],
        vec![InlineKeyboardButton::callback(
            "🆕 Newest",
            "model:browse:newest:0",
        )],
        vec![InlineKeyboardButton::callback(
            "🔥 Popular",
            "model:browse:popular:0",
        )],
    ]);

    let text = format_model_status(state, &chat_id.to_string()).await;

    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Edit message to show the Codex model picker.
async fn edit_to_codex_menu(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
) -> anyhow::Result<()> {
    cache_codex_models(state).await;
    let text = format!(
        "{}\n\nSelect a Codex model:",
        format_model_status(state, &chat_id.to_string()).await
    );
    let rows: Vec<Vec<InlineKeyboardButton>> = crate::codex::KNOWN_MODELS
        .iter()
        .map(|(id, label)| vec![InlineKeyboardButton::callback(*label, detail_cb(id))])
        .collect();
    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;
    Ok(())
}

/// Edit message to show a list of providers (paginated).
async fn edit_to_provider_list(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
    page: usize,
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

    let total_pages = total.div_ceil(PROVIDERS_PER_PAGE);
    let clamped_page = page.min(total_pages.saturating_sub(1));
    let start = clamped_page * PROVIDERS_PER_PAGE;
    let end = (start + PROVIDERS_PER_PAGE).min(total);

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for provider in providers[start..end].iter() {
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

    // Navigation row
    let mut nav_row: Vec<InlineKeyboardButton> = Vec::new();
    if clamped_page > 0 {
        nav_row.push(InlineKeyboardButton::callback(
            "◀ Prev",
            format!("model:provider_list:{}", clamped_page - 1),
        ));
    }
    if total_pages > 1 {
        nav_row.push(InlineKeyboardButton::callback(
            format!("{}/{}", clamped_page + 1, total_pages),
            "noop",
        ));
    }
    if clamped_page + 1 < total_pages {
        nav_row.push(InlineKeyboardButton::callback(
            "Next ▶",
            format!("model:provider_list:{}", clamped_page + 1),
        ));
    }
    if !nav_row.is_empty() {
        rows.push(nav_row);
    }

    rows.push(vec![InlineKeyboardButton::callback("↩ Back", "model:menu")]);

    let keyboard = InlineKeyboardMarkup::new(rows);
    let text = format!("*Select Provider* \\({} available\\)", total);

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
    let mut filtered: Vec<&&ModelInfo> = all_models
        .iter()
        .filter(|m| match category {
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
        })
        .collect();

    // Sort
    match category {
        "newest" => filtered.sort_by_key(|b| std::cmp::Reverse(b.created)),
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
    let total_pages = if total == 0 {
        1
    } else {
        total.div_ceil(MODELS_PER_PAGE)
    };
    let clamped_page = page.min(total_pages.saturating_sub(1));

    let start = clamped_page * MODELS_PER_PAGE;
    let end = (start + MODELS_PER_PAGE).min(total);
    let page_models: Vec<(String, String)> = filtered[start..end]
        .iter()
        .map(|m| {
            (
                m.id.clone(),
                if m.name.is_empty() {
                    m.id.clone()
                } else {
                    m.name.clone()
                },
            )
        })
        .collect();
    drop(s);

    if total == 0 {
        let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
            "↩ Back",
            "model:menu",
        )]]);
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
        rows.push(vec![InlineKeyboardButton::callback(label, detail_cb(m_id))]);
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
    let back_cb = if provider.is_some() {
        "model:provider_list"
    } else {
        "model:menu"
    };
    rows.push(vec![InlineKeyboardButton::callback("↩ Back", back_cb)]);

    let keyboard = InlineKeyboardMarkup::new(rows);

    let category_label = match category {
        "free" => "Free Models",
        "provider" => {
            if let Some(p) = provider {
                if p == "all" {
                    "All Models"
                } else {
                    p
                }
            } else {
                "Provider"
            }
        }
        "newest" => "Newest",
        "popular" => "Popular",
        _ => "Models",
    };

    let escaped_label = crate::escape_v2_safe(category_label);
    let text = format!("*{}* \\({} models\\)", escaped_label, total);

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
            if m.name.is_empty() {
                m.id.clone()
            } else {
                m.name.clone()
            },
            m.context_length,
            m.pricing.is_free(),
            m.pricing.format_per_million(),
        )
    });
    let is_selected = s
        .model_overrides
        .get(&chat_id.to_string())
        .map(|m| m == model_id)
        .unwrap_or(false);
    let current_config_model = s.config.model_for_chat(&chat_id.to_string()).to_string();
    let is_codex = is_known_codex_model(model_id);
    drop(s);

    let (display_name, context_len, pricing_text) = match model_data {
        Some((name, ctx, is_free, formatted_pricing)) => {
            let ctx_str = if ctx == 0 {
                "unknown".to_string()
            } else if ctx >= 1_000_000 {
                format!("{:.1}M", ctx as f64 / 1_000_000.0)
            } else {
                format!("{}k", ctx / 1000)
            };
            let pricing = if is_codex {
                "Included with ChatGPT/Codex subscription".to_string()
            } else if is_free {
                "🆓 Free".to_string()
            } else {
                format!("💲 {} per 1M tokens", formatted_pricing)
            };
            (name, ctx_str, pricing)
        }
        None => (
            model_id.to_string(),
            "unknown".to_string(),
            "unknown".to_string(),
        ),
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
        if is_config_default {
            "\n📌 Config default"
        } else {
            ""
        },
        if is_selected {
            "\n✅ Currently selected"
        } else {
            ""
        },
    );

    let mut rows = vec![vec![InlineKeyboardButton::callback(
        if is_selected {
            "✅ Selected"
        } else {
            "📌 Select Model"
        },
        select_cb(model_id),
    )]];
    rows.push(vec![InlineKeyboardButton::callback(
        "↩ Back",
        "model:browseback",
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
    let chat_key = chat_id.to_string();
    let provider = {
        let s = state.lock().await;
        s.picker_providers
            .get(&chat_key)
            .copied()
            .unwrap_or_else(|| s.effective_provider(&chat_key))
    };
    if provider == crate::config::LlmProvider::Codex && !is_known_codex_model(model_id) {
        bot.edit_message_text(
            chat_id,
            msg_id,
            "This model is not available in the Codex picker\\. Use `/model codex <model>` to set another subscription model\\.",
        )
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
        return Ok(());
    }

    {
        let mut s = state.lock().await;
        if provider == crate::config::LlmProvider::Codex {
            cache_codex_model(&mut s, model_id);
        }
        s.provider_overrides.insert(chat_key.clone(), provider);
        s.model_overrides.insert(chat_key, model_id.to_string());
    }

    let display_name = {
        let s = state.lock().await;
        s.model_metadata
            .get(model_id)
            .map(|m| {
                if m.name.is_empty() {
                    m.id.clone()
                } else {
                    m.name.clone()
                }
            })
            .unwrap_or_else(|| model_id.to_string())
    };

    let text = format!(
        "✅ *Model Selected*\n\n`{}`\n\nUse `/model\\_default` to reset to config default\\.",
        display_name
    );

    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "🔙 Back to Menu",
        "model:menu",
    )]]);

    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Handle a specifier callback (model:spec:nitro, model:spec:floor, model:spec:free).
/// Applies the given specifier to the current effective model and shows confirmation.
async fn handle_spec_callback(
    state: &Arc<Mutex<BotState>>,
    chat_id: ChatId,
    bot: &teloxide::Bot,
    msg_id: MessageId,
    specifier: &str,
) -> anyhow::Result<()> {
    if state.lock().await.effective_provider(&chat_id.to_string())
        != crate::config::LlmProvider::Openrouter
    {
        bot.edit_message_text(
            chat_id,
            msg_id,
            "Routing specifiers are only available with OpenRouter\\.",
        )
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
        return Ok(());
    }

    // Validate specifier
    let valid: Vec<&str> = crate::openrouter::SPECIFIER_BUTTONS
        .iter()
        .map(|(s, _)| *s)
        .collect();
    if specifier.is_empty() || !valid.contains(&specifier) {
        bot.edit_message_text(chat_id, msg_id, "Unknown specifier.")
            .await?;
        return Ok(());
    }

    let new_model = {
        let s = state.lock().await;
        let current = s.effective_model(&chat_id.to_string());
        crate::openrouter::apply_specifier(&current, specifier)
    };

    {
        let mut s = state.lock().await;
        s.model_overrides
            .insert(chat_id.to_string(), new_model.clone());
    }

    let spec_label = crate::openrouter::SPECIFIER_BUTTONS
        .iter()
        .find(|(s, _)| *s == specifier)
        .map(|(_, label)| label.to_string())
        .unwrap_or_else(|| format!(":{}", specifier));

    let text = format!(
        "✅ {}\n\n`{}`\n\nUse `/model_default` to reset to config default, or `/model` to change again.",
        spec_label, new_model
    );

    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "🔙 Back to Menu",
        "model:menu",
    )]]);

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
    let provider = s.effective_provider(chat_id);
    let config_default = if s.provider_overrides.contains_key(chat_id) {
        s.config.default_model_for_provider(provider)
    } else {
        s.config.model_for_chat(chat_id)
    };
    let has_override =
        s.model_overrides.contains_key(chat_id) || s.provider_overrides.contains_key(chat_id);
    let provider_name = format!("{:?}", provider).to_lowercase();

    if has_override {
        format!(
            "🎯 *Current model:* `{}` \\(override\\)\n🏷 Provider: `{}`\n📌 Config default: `{}`\n\nBrowse models:",
            current, provider_name, config_default
        )
    } else {
        format!(
            "🎯 *Current model:* `{}`\n🏷 Provider: `{}`\n\nBrowse models:",
            current, provider_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChatConfig, CodexConfig, LlmProvider};
    use crate::llm::mock::MockLlmBackend;
    use std::collections::HashMap;
    use tempfile::TempDir;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn codex_state() -> (Arc<Mutex<BotState>>, TempDir) {
        let dir = TempDir::new().unwrap();
        let mut config = crate::config::basic_config();
        config.codex = Some(CodexConfig {
            model: "gpt-5.4".into(),
            auth_file: "auth.json".into(),
            reasoning_effort: None,
            base_url: "https://chatgpt.com/backend-api".into(),
        });
        config.chats.insert(
            "-123".into(),
            ChatConfig {
                provider: Some(LlmProvider::Codex),
                ..Default::default()
            },
        );
        (
            Arc::new(Mutex::new(BotState {
                config,
                skills: HashMap::new(),
                llm: Arc::new(MockLlmBackend::new()),
                data_dir: dir.path().to_path_buf(),
                db: crate::db::Database::open_in_memory().unwrap(),
                mcp_tools: vec![],
                _mcp_services: vec![],
                mcp_peers: HashMap::new(),
                model_metadata: HashMap::new(),
                model_order: vec![],
                last_usage: HashMap::new(),
                pending_config_changes: HashMap::new(),
                pending_model_changes: HashMap::new(),
                model_overrides: HashMap::new(),
                provider_overrides: HashMap::new(),
                picker_providers: HashMap::new(),
                last_browse_cb: HashMap::new(),
            })),
            dir,
        )
    }

    fn telegram_bot(server: &MockServer) -> teloxide::Bot {
        teloxide::Bot::new("test-token")
            .set_api_url(reqwest::Url::parse(&format!("{}/", server.uri())).unwrap())
    }

    fn telegram_message_response() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 1,
                "date": 0,
                "chat": {"id": -123, "type": "group"}
            }
        }))
    }

    #[tokio::test]
    async fn test_codex_picker_sends_subscription_models_and_caches_metadata() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(telegram_message_response())
            .mount(&server)
            .await;
        let (state, _dir) = codex_state().await;

        cache_codex_models(&state).await;
        send_provider_model_menu(&state, "-123", telegram_bot(&server))
            .await
            .unwrap();
        send_codex_model_info(&state, "-123", telegram_bot(&server))
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        let menu: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(menu["text"].as_str().unwrap().contains("Provider: `codex`"));
        assert!(menu["reply_markup"]["inline_keyboard"]
            .to_string()
            .contains("Codex Subscription Models"));
        let state = state.lock().await;
        assert!(state.model_metadata.contains_key("gpt-5.4"));
        assert!(state.model_metadata.contains_key("gpt-5.6-terra"));
    }

    #[tokio::test]
    async fn test_codex_picker_selects_known_models_only() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(telegram_message_response())
            .mount(&server)
            .await;
        let (state, _dir) = codex_state().await;
        let bot = telegram_bot(&server);

        select_model(&state, ChatId(-123), &bot, MessageId(1), "gpt-5.5")
            .await
            .unwrap();
        {
            let state = state.lock().await;
            assert_eq!(state.model_overrides.get("-123").unwrap(), "gpt-5.5");
            assert!(state.model_metadata.contains_key("gpt-5.5"));
        }

        select_model(&state, ChatId(-123), &bot, MessageId(1), "openai/gpt-4o")
            .await
            .unwrap();
        assert_eq!(
            state.lock().await.model_overrides.get("-123").unwrap(),
            "gpt-5.5"
        );

        state
            .lock()
            .await
            .picker_providers
            .insert("-123".into(), LlmProvider::Openrouter);
        select_model(&state, ChatId(-123), &bot, MessageId(1), "openai/gpt-4o")
            .await
            .unwrap();
        let state = state.lock().await;
        assert_eq!(state.model_overrides.get("-123").unwrap(), "openai/gpt-4o");
        assert_eq!(state.effective_provider("-123"), LlmProvider::Openrouter);
    }

    #[tokio::test]
    async fn test_codex_picker_rejects_openrouter_specifier_callbacks() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(telegram_message_response())
            .mount(&server)
            .await;
        let (state, _dir) = codex_state().await;
        handle_spec_callback(
            &state,
            ChatId(-123),
            &telegram_bot(&server),
            MessageId(1),
            "nitro",
        )
        .await
        .unwrap();
        assert!(state.lock().await.model_overrides.is_empty());
    }

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
