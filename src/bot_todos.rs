//! Todo list inline keyboard — button-triggered paginated todo management.

use crate::bot::BotState;
use crate::todos::TodoList;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use tokio::sync::Mutex;

const TODOS_PER_PAGE: usize = 8;

/// Send the initial todos message with a "✅ Mark done" button.
/// When the user clicks it, a paginated list of todos as buttons is shown.
pub async fn send_todos_message(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    details: bool,
    tg_bot: &teloxide::Bot,
) {
    let chat_id_i64 = match chat_id.parse::<i64>() {
        Ok(id) => ChatId(id),
        Err(e) => {
            log::error!("Invalid chat_id in send_todos_message: {} ({})", chat_id, e);
            return;
        }
    };

    let (text, has_items) = {
        let s = state.lock().await;
        let list = TodoList::load(&s.chats_dir(), chat_id).unwrap_or_default();
        let has = !list.todos.is_empty();
        let text = if !has {
            "No todos for this chat.".to_string()
        } else if details {
            let mut lines = vec![format!("*{} todo(s) (detailed):*", list.todos.len())];
            for (i, t) in list.todos.iter().enumerate() {
                let status = if t.completed { "✅" } else { "⬜" };
                let updated = t.updated_at.as_deref().unwrap_or("never");
                lines.push(format!(
                    "{}. {} `{}` — {}\n   Created: {} | Updated: {}",
                    i + 1,
                    status,
                    t.id,
                    t.description,
                    t.created_at,
                    updated
                ));
            }
            lines.join("\n")
        } else {
            let mut lines = vec![format!("*{} todo(s):*", list.todos.len())];
            for (i, t) in list.todos.iter().enumerate() {
                let status = if t.completed { "✅" } else { "⬜" };
                lines.push(format!("{}. {} {}", i + 1, status, t.description));
            }
            lines.join("\n")
        };
        (text, has)
    };

    let escaped = crate::escape_v2_safe(&text);

    if has_items {
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("✅ Mark done", "todo:menu:0"),
        ]]);

        let result = tg_bot
            .send_message(chat_id_i64, &escaped)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .reply_markup(keyboard)
            .await;
        if let Err(e) = result {
            log::warn!("Failed to send todos message with keyboard: {}", e);
            let _ = tg_bot.send_message(chat_id_i64, &text).await;
        }
    } else {
        let _ = tg_bot
            .send_message(chat_id_i64, &escaped)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .await;
    }
}

/// Show a paginated list of todos as inline buttons.
async fn show_todo_menu_page(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    page: usize,
    tg_bot: &teloxide::Bot,
    chat: ChatId,
    msg_id: teloxide::types::MessageId,
) {
    let list = {
        let s = state.lock().await;
        TodoList::load(&s.chats_dir(), chat_id).unwrap_or_default()
    };

    let total = list.todos.len();
    let total_pages = total.div_ceil(TODOS_PER_PAGE);
    let page = page.min(total_pages.saturating_sub(1));

    if list.todos.is_empty() {
        let text = "✅ *All done\\!* No todos remaining.";
        let _ = tg_bot
            .edit_message_text(chat, msg_id, text)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .await;
        return;
    }

    let start = page * TODOS_PER_PAGE;
    let end = (start + TODOS_PER_PAGE).min(total);
    let page_items = &list.todos[start..end];

    let header = format!(
        "*Todos \\({}—{} of {}\\):*",
        start + 1,
        end,
        total
    );

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for t in page_items {
        let status = if t.completed { "✅" } else { "⬜" };
        let label = format!("{} {}", status, t.description);
        // Truncate long labels (Telegram button limit is 64 bytes)
        let label = truncate_button_label(&label);
        rows.push(vec![InlineKeyboardButton::callback(
            label,
            format!("todo:toggle:{}", t.id),
        )]);
    }

    // Navigation row
    let mut nav_row: Vec<InlineKeyboardButton> = Vec::new();
    if page > 0 {
        nav_row.push(InlineKeyboardButton::callback(
            "◀️ Prev",
            format!("todo:menu:{}", page.saturating_sub(1)),
        ));
    }
    nav_row.push(InlineKeyboardButton::callback(
        format!("{}/{}", page + 1, total_pages),
        "noop",
    ));
    if page + 1 < total_pages {
        nav_row.push(InlineKeyboardButton::callback(
            "Next ▶️",
            format!("todo:menu:{}", page + 1),
        ));
    }
    rows.push(nav_row);

    // Close button
    rows.push(vec![InlineKeyboardButton::callback(
        "✖️ Close",
        "todo:close",
    )]);

    let keyboard = InlineKeyboardMarkup::new(rows);

    let result = tg_bot
        .edit_message_text(chat, msg_id, &header)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await;

    if let Err(e) = result {
        log::warn!("Failed to edit todos menu: {}", e);
        let _ = tg_bot
            .edit_message_text(chat, msg_id, "Error showing todos.")
            .await;
    }
}

/// Handle a todo-related callback query.
/// Returns `true` if the callback was handled.
pub async fn handle_todo_callback(
    state: &Arc<Mutex<BotState>>,
    data: &str,
    callback_id: &str,
    tg_bot: &teloxide::Bot,
    chat: ChatId,
    msg_id: teloxide::types::MessageId,
) -> bool {
    if !data.starts_with("todo:") {
        return false;
    }

    if data == "todo:close" {
        let _ = tg_bot
            .edit_message_reply_markup(chat, msg_id)
            .await;
        return true;
    }

    if let Some(page_str) = data.strip_prefix("todo:menu:") {
        if let Ok(page) = page_str.parse::<usize>() {
            let cid = chat.0.to_string();
            show_todo_menu_page(state, &cid, page, tg_bot, chat, msg_id).await;
        }
        return true;
    }

    if let Some(todo_id) = data.strip_prefix("todo:toggle:") {
        let chat_id = chat.0.to_string();
        let (found, new_status, description) = {
            let s = state.lock().await;
            let mut list = TodoList::load(&s.chats_dir(), &chat_id).unwrap_or_default();
            let result = list.toggle(todo_id);
            let desc = list
                .todos
                .iter()
                .find(|t| t.id == todo_id)
                .map(|t| t.description.clone());
            if result.is_some() {
                let _ = list.save(&s.chats_dir(), &chat_id);
            }
            (result.is_some(), result, desc)
        };

        if !found {
            let _ = tg_bot
                .answer_callback_query(callback_id)
                .text("Todo not found")
                .await;
            return true;
        }

        // Show a brief confirmation
        let status_word = if new_status == Some(true) {
            "done ✅"
        } else {
            "reopened 🔄"
        };
        let desc = description.unwrap_or_default();
        let _ = tg_bot
            .answer_callback_query(callback_id)
            .text(&format!("Marked {}: {}", status_word, desc))
            .show_alert(false)
            .await;

        // Refresh the current page — we don't know which page, so show page 0
        // (the item may have moved, but this is simple and works)
        show_todo_menu_page(state, &chat_id, 0, tg_bot, chat, msg_id).await;
        return true;
    }

    false
}

/// Truncate a button label to fit Telegram's 64-byte limit.
fn truncate_button_label(label: &str) -> String {
    if label.len() <= 62 {
        return label.to_string();
    }
    // Find a valid UTF-8 boundary within 62 bytes
    let mut end = 59;
    while end > 0 && !label.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &label[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_button_label_short() {
        assert_eq!(truncate_button_label("hello"), "hello");
    }

    #[test]
    fn test_truncate_button_label_long() {
        let long = "a".repeat(80);
        let result = truncate_button_label(&long);
        assert!(result.len() <= 63); // 59 + "…" = up to 63 bytes
        assert!(result.ends_with('…'));
    }

    #[test]
    fn test_truncate_button_label_emoji_boundary() {
        // "😀" is 4 bytes, so 15 emojis = 60 bytes
        let text = "😀".repeat(20);
        let result = truncate_button_label(&text);
        assert!(result.len() <= 63);
    }
}
