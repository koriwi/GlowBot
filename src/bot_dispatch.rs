use super::BotState;
use crate::openrouter::{ChatMessage, ToolCall};
use std::io::Write;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

#[path = "bot_dispatch_config.rs"]
pub mod bot_dispatch_config;
#[path = "bot_dispatch_describe.rs"]
mod bot_dispatch_describe;
#[path = "bot_dispatch_image.rs"]
pub(crate) mod bot_dispatch_image;
#[path = "bot_dispatch_media.rs"]
mod bot_dispatch_media;
#[path = "bot_dispatch_memory.rs"]
mod bot_dispatch_memory;
#[path = "bot_dispatch_model.rs"]
pub mod bot_dispatch_model;
#[path = "bot_dispatch_skills.rs"]
mod bot_dispatch_skills;

/// Log a tool call to `tool_calls.log` in the given data directory.
pub(crate) fn log_tool_call_to(
    data_dir: &std::path::Path,
    tool_name: &str,
    args: &str,
    result: &str,
) {
    let log_path = data_dir.join("tool_calls.log");
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let result_summary: String = result.chars().take(300).collect();
    let args_summary: String = args.chars().take(200).collect();
    let line = format!(
        "[{}] {} | args: {} | result: {}\n",
        timestamp, tool_name, args_summary, result_summary
    );
    // Best-effort append — don't block on log write errors
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut f| f.write_all(line.as_bytes()));

    let is_error = [
        "Error",
        "parse error",
        "HTTP",
        "request failed",
        "RPC error",
    ]
    .iter()
    .any(|pat| result.contains(pat));
    if is_error {
        log::warn!(
            "tool {} error (args: {}): {}",
            tool_name,
            args_summary,
            result_summary
        );
    } else {
        log::info!("tool {}: {}", tool_name, args_summary);
    }
}

/// Dispatch a batch of tool calls and return the result messages.
/// Optionally logs each call if `data_dir` is provided.
pub(crate) async fn dispatch_tool_calls(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    tool_calls: &[ToolCall],
    data_dir: Option<&std::path::Path>,
    tg_bot: Option<&teloxide::Bot>,
) -> Vec<ChatMessage> {
    let max_result_chars = state.lock().await.config.conversation.max_tool_result_chars;

    let mut results = Vec::new();
    for tc in tool_calls {
        let args: serde_json::Value =
            serde_json::from_str(&tc.function.arguments).unwrap_or_default();
        let result_text =
            dispatch_tool(state, chat_id, tc.function.name.as_str(), &args, tg_bot).await;
        if let Some(dir) = data_dir {
            log_tool_call_to(dir, &tc.function.name, &tc.function.arguments, &result_text);
        }
        let final_text = cap_tool_result(&result_text, max_result_chars);
        results.push(ChatMessage::tool_result(&tc.id, &final_text));
    }
    results
}

/// If `max_chars` is set and the result exceeds it, replace with an error
/// message telling the LLM to reduce the response size.
pub(crate) fn cap_tool_result(result: &str, max_chars: Option<usize>) -> String {
    let Some(limit) = max_chars else {
        return result.to_string();
    };
    if result.len() <= limit {
        return result.to_string();
    }
    format!(
        "Error: tool result exceeded the maximum size limit ({} chars, limit is {} chars). \
         Try reducing the output by filtering it (jq, grep, head, tail, awk), \
         narrowing your query parameters, or using a different tool.",
        result.len(),
        limit
    )
}

/// Shared tool dispatch — used by both normal messages and heartbeat tasks.
pub(crate) async fn dispatch_tool(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
    tg_bot: Option<&teloxide::Bot>,
) -> String {
    let cid = chat_id.to_string();
    match tool_name {
        "send_message" => {
            let text = args["text"].as_str().unwrap_or("");
            if text.is_empty() {
                return "Error: text required".into();
            }
            if let Some(bot) = tg_bot {
                let Ok(chat_id_i64) = cid.parse::<i64>() else {
                    return format!("Error: invalid chat_id '{}'", cid);
                };
                let chat = ChatId(chat_id_i64);
                crate::bot_send::send_message(bot, chat, text).await;
                "Message sent.".into()
            } else {
                "Error: send_message not available in this context.".into()
            }
        }
        "list_media" => bot_dispatch_media::tool_list_media(state, args).await,
        "send_media" => bot_dispatch_media::tool_send_media(state, &cid, args, tg_bot).await,
        "bash" => {
            if !state.lock().await.config.is_bash_enabled(&cid) {
                return "Error: bash is disabled for this chat. Enable it in config or ask an admin.".into();
            }
            let cmd = args["command"].as_str().unwrap_or("");
            let dir = { state.lock().await.data_dir.clone() };
            match crate::bash::execute_in_dir(cmd, &dir).await {
                Ok(r) => {
                    let mut out = String::new();
                    if !r.stdout.is_empty() {
                        out.push_str(&format!("stdout:\n{}", r.stdout));
                    }
                    if !r.stderr.is_empty() {
                        out.push_str(&format!("stderr:\n{}", r.stderr));
                    }
                    if r.stdout.is_empty() && r.stderr.is_empty() {
                        out.push_str(&format!("exit code: {}", r.exit_code));
                    }
                    out
                }
                Err(e) => format!("Error: {}", e),
            }
        }
        "read_memory" => bot_dispatch_memory::tool_read_memory(state, &cid, args).await,
        "update_memory" => bot_dispatch_memory::tool_update_memory(state, &cid, args).await,
        "read_chat_memory" => bot_dispatch_memory::tool_read_chat_memory(state, &cid).await,
        "update_chat_memory" => {
            bot_dispatch_memory::tool_update_chat_memory(state, &cid, args).await
        }
        "add_task" => {
            let d = args["description"].as_str().unwrap_or("");
            if d.is_empty() {
                return "Error: description required".into();
            }
            let s = state.lock().await;
            let mut list = crate::tasks::TaskList::load(&s.chats_dir(), &cid).unwrap_or_default();
            let id = list.add(d);
            let _ = list.save(&s.chats_dir(), &cid);
            format!("Task '{}' added: {}", id, d)
        }
        "list_tasks" => {
            let s = state.lock().await;
            let list = crate::tasks::TaskList::load(&s.chats_dir(), &cid).unwrap_or_default();
            if list.tasks.is_empty() {
                "No pending tasks.".into()
            } else {
                serde_json::to_string_pretty(&list.tasks).unwrap_or_default()
            }
        }
        "remove_task" => {
            let id = args["id"].as_str().unwrap_or("");
            if id.is_empty() {
                return "Error: id required".into();
            }
            let s = state.lock().await;
            let mut list = crate::tasks::TaskList::load(&s.chats_dir(), &cid).unwrap_or_default();
            if list.remove(id) {
                let _ = list.save(&s.chats_dir(), &cid);
                format!("Task '{}' removed. {} remaining.", id, list.tasks.len())
            } else {
                format!("Task '{}' not found.", id)
            }
        }
        "create_reminder" => {
            let desc = args["description"].as_str().unwrap_or("");
            let trigger = args["trigger_at"].as_str().unwrap_or("");
            if desc.is_empty() || trigger.is_empty() {
                return "Error: description and trigger_at required".into();
            }
            // Validate the timestamp
            if chrono::DateTime::parse_from_rfc3339(trigger).is_err() {
                return format!("Error: trigger_at must be a valid ISO 8601 timestamp in UTC (e.g. '2026-05-11T18:00:00Z'), got: {}", trigger);
            }
            let action = args["action"].as_str();
            let s = state.lock().await;
            let mut list =
                crate::reminders::ReminderList::load(&s.chats_dir(), &cid).unwrap_or_default();
            let id = list.add(desc, trigger, action);
            let _ = list.save(&s.chats_dir(), &cid);
            format!("Reminder '{}' created for {}: {}", id, trigger, desc)
        }
        "list_reminders" => {
            let s = state.lock().await;
            let list =
                crate::reminders::ReminderList::load(&s.chats_dir(), &cid).unwrap_or_default();
            if list.reminders.is_empty() {
                "No pending reminders.".into()
            } else {
                serde_json::to_string_pretty(&list.reminders).unwrap_or_default()
            }
        }
        "remove_reminder" => {
            let id = args["id"].as_str().unwrap_or("");
            if id.is_empty() {
                return "Error: id required".into();
            }
            let s = state.lock().await;
            let mut list =
                crate::reminders::ReminderList::load(&s.chats_dir(), &cid).unwrap_or_default();
            if list.remove(id) {
                let _ = list.save(&s.chats_dir(), &cid);
                format!(
                    "Reminder '{}' removed. {} remaining.",
                    id,
                    list.reminders.len()
                )
            } else {
                format!("Reminder '{}' not found.", id)
            }
        }
        "generate_image" => bot_dispatch_image::tool_generate_image(state, &cid, args).await,
        "describe_image" => bot_dispatch_describe::tool_describe_image(state, &cid, args).await,
        "create_skill" => bot_dispatch_skills::tool_create_skill(state, args).await,
        "read_skill" => bot_dispatch_skills::tool_read_skill(state, args).await,
        "update_skill" => bot_dispatch_skills::tool_update_skill(state, args).await,
        name if name.starts_with("mcp_") => {
            let mut args_clone = args.clone();
            // Look up tool info and peer under the state lock, then invoke outside it.
            let (server_name, bare_tool_name, peer) = {
                let s = state.lock().await;
                let idx = s
                    .mcp_tools
                    .iter()
                    .position(|t| format!("mcp_{}_{}", t.server_name, t.name) == name);
                let idx = match idx {
                    Some(i) => i,
                    None => return format!("MCP tool not found: {}", name),
                };
                let srv = s.mcp_tools[idx].server_name.clone();
                if !s.config.is_mcp_server_allowed(&cid, &srv) {
                    return format!("MCP tool blacklisted for this chat: {}", name);
                }
                // Workaround for Playwright MCP server bug: the server
                // doesn't respect the output dir for named fullpage
                // screenshots, so we prepend the pw-media path here.
                if name.contains("screenshot") {
                    for key in &["filename", "name"] {
                        if let Some(name_val) = args_clone.get(*key).and_then(|v| v.as_str()) {
                            if !name_val.is_empty()
                                && !name_val.starts_with('/')
                                && !name_val.contains('/')
                            {
                                args_clone[*key] = serde_json::json!(format!(
                                    "{}/pw-media/{}",
                                    s.config.media_dir, name_val
                                ));
                                break;
                            }
                        }
                    }
                }
                let bare_name = s.mcp_tools[idx].name.clone();
                let peer = s.mcp_peers.get(&srv).cloned();
                (srv, bare_name, peer)
            };
            match peer {
                Some(p) => crate::mcp::invoke_tool(&p, &bare_tool_name, &args_clone).await,
                None => format!("MCP server '{}' is not connected", server_name),
            }
        }
        "create_todo" => {
            let d = args["description"].as_str().unwrap_or("");
            if d.is_empty() {
                return "Error: description required".into();
            }
            let s = state.lock().await;
            let mut list = crate::todos::TodoList::load(&s.chats_dir(), &cid).unwrap_or_default();
            let id = list.add(d);
            let _ = list.save(&s.chats_dir(), &cid);
            format!("Todo created: {} — {}", id, d)
        }
        "list_todos" => {
            let s = state.lock().await;
            let list = crate::todos::TodoList::load(&s.chats_dir(), &cid).unwrap_or_default();
            if list.todos.is_empty() {
                "No todos yet.".into()
            } else {
                serde_json::to_string_pretty(&list.todos).unwrap_or_default()
            }
        }
        "edit_todo" => {
            let id = args["id"].as_str().unwrap_or("");
            if id.is_empty() {
                return "Error: id required".into();
            }
            let new_desc = args["description"].as_str();
            let completed = args.get("completed").and_then(|v| v.as_bool());

            if new_desc.is_none() && completed.is_none() {
                return "Error: at least one of 'description' or 'completed' must be provided"
                    .into();
            }

            let s = state.lock().await;
            let mut list = crate::todos::TodoList::load(&s.chats_dir(), &cid).unwrap_or_default();

            let mut result_parts: Vec<String> = Vec::new();

            if let Some(desc) = new_desc {
                if desc.is_empty() {
                    return "Error: description must not be empty".into();
                }
                if list.edit(id, desc) {
                    result_parts.push(format!("description updated to '{}'", desc));
                } else {
                    return format!("Todo '{}' not found.", id);
                }
            }

            if completed.is_some() {
                match list.toggle(id) {
                    Some(new_status) => {
                        let status_word = if new_status {
                            "completed"
                        } else {
                            "not completed"
                        };
                        result_parts.push(format!("marked as {}", status_word));
                    }
                    None => return format!("Todo '{}' not found.", id),
                }
            }

            let _ = list.save(&s.chats_dir(), &cid);
            format!("Todo '{}': {}", id, result_parts.join(", "))
        }
        "delete_todo" => {
            let id = args["id"].as_str().unwrap_or("");
            if id.is_empty() {
                return "Error: id required".into();
            }
            let s = state.lock().await;
            let mut list = crate::todos::TodoList::load(&s.chats_dir(), &cid).unwrap_or_default();
            if list.remove(id) {
                let _ = list.save(&s.chats_dir(), &cid);
                format!("Todo '{}' deleted. {} remaining.", id, list.todos.len())
            } else {
                format!("Todo '{}' not found.", id)
            }
        }
        "get_recent_messages" => {
            let count = args["count"].as_i64().unwrap_or(10) as usize;
            let count = count.clamp(1, 50);
            let history = {
                let s = state.lock().await;
                let cutoff = s.db.get_cutoff(&cid).unwrap_or(None);
                match s.db.load_messages(&cid, count, cutoff) {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        log::error!(
                            "Failed to load messages for get_recent_messages tool: {}",
                            e
                        );
                        Vec::new()
                    }
                }
            };
            let items: Vec<_> = history
                .iter()
                .filter(|m| m.role != "tool")
                .filter(|m| {
                    // Skip messages with no visible text (empty content or media placeholders).
                    let text = m.text_content();
                    !text.trim().is_empty() && text != "[image]" && text != "[audio]"
                })
                .map(|m| {
                    serde_json::json!({
                        "role": &m.role,
                        "content": m.text_content(),
                        "name": m.name.as_deref().unwrap_or("")
                    })
                })
                .collect();
            serde_json::json!({"messages": items}).to_string()
        }
        "read_config_schema" => bot_dispatch_config::tool_read_config_schema().await,
        "read_config" => bot_dispatch_config::tool_read_config(state).await,
        "edit_config" => bot_dispatch_config::tool_edit_config(state, &cid, args, tg_bot).await,
        "get_model_info" => bot_dispatch_model::tool_get_model_info(state, &cid).await,
        "propose_model_change" => {
            bot_dispatch_model::tool_propose_model_change(state, &cid, args, tg_bot).await
        }
        "ask_advisor" => {
            let query = args["query"].as_str().unwrap_or("");
            if query.is_empty() {
                return "Error: query required".into();
            }

            let (advice_model, window_size, db) = {
                let s = state.lock().await;
                let advice_model = match s.config.advice_model_for_chat(&cid) {
                    Some(m) => m.to_string(),
                    None => return "Error: advice model not configured — the ask_advisor tool is disabled.".into(),
                };
                let window_size = s.config.conversation.advice_recent_messages_window_size;
                let db = s.db.clone();
                (advice_model, window_size, db)
            };

            let recent_messages = if window_size > 0 {
                match db.load_messages(&cid, window_size, None) {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        log::error!("ask_advisor: failed to load messages: {}", e);
                        vec![]
                    }
                }
            } else {
                vec![]
            };

            let mut advice_messages: Vec<ChatMessage> = vec![
                ChatMessage::system(
                    "You are an advisor model. A smaller/cheaper AI model is asking for your private help \
                     with a question in an ongoing conversation. Below is the recent conversation \
                     history, followed by the specific question. Respond helpfully with your best \
                     analysis, opinion, or recommendation. Be concise and direct.\n\n\
                     Important: Your response goes back to the calling model — NOT directly to the end user. \
                     The calling model will decide whether to relay parts of your response verbatim or \
                     use it as internal guidance to continue the conversation on its own."
                ),
            ];

            for msg in &recent_messages {
                // Relabel all messages as "user" so the advisor never sees
                // "assistant" messages it didn't produce. Use the `name` field
                // to distinguish human users from the calling model so the
                // advisor can track who said what.
                let speaker = if msg.role == "user" {
                    msg.name.clone().unwrap_or_else(|| "human".into())
                } else {
                    "calling_model".into()
                };
                advice_messages.push(ChatMessage {
                    role: "user".into(),
                    content: msg.content.clone(),
                    name: Some(speaker),
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning: msg.reasoning.clone(),
                    provider_data: None,
                });
            }

            advice_messages.push(ChatMessage::user_with_name(
                &format!(
                    "Here is my question. Please give me your best analysis and advice:\n\n{}",
                    query
                ),
                "calling_model",
            ));

            let request = crate::openrouter::ChatCompletionRequest {
                model: advice_model,
                messages: advice_messages,
                tools: None,
                tool_choice: None,
                modalities: None,
                image_config: None,
            };

            let llm = { state.lock().await.llm.clone() };
            match llm.chat_completion(&request).await {
                Ok(resp) => {
                    let text = resp
                        .choices
                        .into_iter()
                        .next()
                        .and_then(|c| c.message.content)
                        .unwrap_or_default();
                    let usage_info = match &resp.usage {
                        Some(u) if u.total_tokens > 0 => {
                            format!("\n\n[Advisor model usage: {} prompt + {} completion = {} total tokens]",
                                u.prompt_tokens, u.completion_tokens, u.total_tokens)
                        }
                        _ => String::new(),
                    };
                    format!("Advisor response:\n{}{}", text, usage_info)
                }
                Err(e) => format!("Error calling advisor model: {}", e),
            }
        }
        "search_conversations" => {
            let query = args["query"].as_str().unwrap_or("");
            if query.is_empty() {
                return "Error: query required".into();
            }
            let count = args["count"].as_i64().unwrap_or(5) as usize;
            let count = count.clamp(1, 10);

            let (embedding_model, search_limit) = {
                let s = state.lock().await;
                let cfg = &s.config;
                (
                    match &cfg.openrouter.embedding_model {
                        Some(m) => m.clone(),
                        None => return "Error: embedding model not configured".into(),
                    },
                    cfg.embedding.search_limit,
                )
            };

            let llm = { state.lock().await.llm.clone() };
            let query_embedding = match llm.embeddings(&embedding_model, query).await {
                Ok(e) => e,
                Err(e) => return format!("Error embedding query: {}", e),
            };

            let results = {
                let s = state.lock().await;
                s.db.search_embeddings(
                    &cid,
                    &query_embedding,
                    &embedding_model,
                    count,
                    search_limit,
                )
                .unwrap_or_default()
            };

            // search_embeddings already excludes tool messages at the SQL level.
            let top_results: Vec<_> = results
                .into_iter()
                .filter(|(_id, _score, text)| {
                    // Extra safety: skip messages that are just media placeholders.
                    let t = text.trim();
                    !t.is_empty() && t != "[image]" && t != "[audio]"
                })
                .map(|(_id, score, text)| {
                    serde_json::json!({
                        "similarity": format!("{:.4}", score),
                        "content": text
                    })
                })
                .collect();

            if top_results.is_empty() {
                "No similar messages found.".into()
            } else {
                serde_json::json!({"results": top_results}).to_string()
            }
        }
        _ => format!("Unknown tool: {}", tool_name),
    }
}
