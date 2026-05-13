use super::BotState;
use crate::openrouter::{ChatMessage, ToolCall};
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

#[path = "bot_dispatch_media.rs"]
mod bot_dispatch_media;
#[path = "bot_dispatch_memory.rs"]
mod bot_dispatch_memory;
#[path = "bot_dispatch_image.rs"]
pub(crate) mod bot_dispatch_image;
#[path = "bot_dispatch_skills.rs"]
mod bot_dispatch_skills;
#[path = "bot_dispatch_config.rs"]
pub mod bot_dispatch_config;
#[path = "bot_dispatch_model.rs"]
pub mod bot_dispatch_model;

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
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.as_bytes())
        });

    let is_error = result.starts_with("Error")
        || result.contains("parse error")
        || result.contains("HTTP")
        || result.contains("request failed")
        || result.contains("RPC error");
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
    let mut results = Vec::new();
    for tc in tool_calls {
        let args: serde_json::Value =
            serde_json::from_str(&tc.function.arguments).unwrap_or_default();
        let result_text =
            dispatch_tool(state, chat_id, tc.function.name.as_str(), &args, tg_bot).await;
        if let Some(dir) = data_dir {
            log_tool_call_to(dir, &tc.function.name, &tc.function.arguments, &result_text);
        }
        results.push(ChatMessage::tool_result(&tc.id, &result_text));
    }
    results
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
        "list_media" => {
            bot_dispatch_media::tool_list_media(state, args).await
        }
        "send_media" => {
            bot_dispatch_media::tool_send_media(state, &cid, args, tg_bot).await
        }
        "bash" => {
            if !state.lock().await.config.is_bash_enabled(&cid) {
                return format!(
                    "Error: bash is disabled for this chat. Enable it in config or ask an admin."
                );
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
        "read_memory" => {
            bot_dispatch_memory::tool_read_memory(state, &cid, args).await
        }
        "update_memory" => {
            bot_dispatch_memory::tool_update_memory(state, &cid, args).await
        }
        "read_chat_memory" => {
            bot_dispatch_memory::tool_read_chat_memory(state, &cid).await
        }
        "update_chat_memory" => {
            bot_dispatch_memory::tool_update_chat_memory(state, &cid, args).await
        }
        "add_task" => {
            let d = args["description"].as_str().unwrap_or("");
            if d.is_empty() {
                return "Error: description required".into();
            }
            let s = state.lock().await;
            let mut list =
                crate::tasks::TaskList::load(&s.chats_dir(), &cid).unwrap_or_default();
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
            let mut list =
                crate::tasks::TaskList::load(&s.chats_dir(), &cid).unwrap_or_default();
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
                format!("Reminder '{}' removed. {} remaining.", id, list.reminders.len())
            } else {
                format!("Reminder '{}' not found.", id)
            }
        }
        "generate_image" => {
            bot_dispatch_image::tool_generate_image(state, &cid, args).await
        }
        "create_skill" => {
            bot_dispatch_skills::tool_create_skill(state, args).await
        }
        "read_skill" => {
            bot_dispatch_skills::tool_read_skill(state, args).await
        }
        "update_skill" => {
            bot_dispatch_skills::tool_update_skill(state, args).await
        }
        name if name.starts_with("mcp_") => {
            let tool_name = name.to_string();
            let mut args_clone = args.clone();
            let result = {
                // Phase 1: under state lock — get server name, per-server lock, blacklist check
                let (_srv_name, server_lock, blacklisted) = {
                    let mut s = state.lock().await;
                    let tool_idx = s
                        .mcp_tools
                        .iter()
                        .position(|t| format!("mcp_{}_{}", t.server_name, t.name) == tool_name);
                    match tool_idx {
                        Some(idx) => {
                            let srv = s.mcp_tools[idx].server_name.clone();
                            let blacklisted = !s.config.is_mcp_server_allowed(chat_id, &srv);
                            let server_lock = s
                                .mcp_server_locks
                                .entry(srv.clone())
                                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                                .clone();
                            (srv, server_lock, blacklisted)
                        }
                        None => return format!("MCP tool not found: {}", tool_name),
                    }
                };
                if blacklisted {
                    return format!("MCP tool blacklisted for this chat: {}", tool_name);
                }
                // Phase 2: under state lock — get tool, apply screenshot workaround, clone.
                // Per-server lock is NOT held here — normal tool calls are fully concurrent.
                let (mut tc, server_name) = {
                    let s = state.lock().await;
                    let tool_idx = s
                        .mcp_tools
                        .iter()
                        .position(|t| format!("mcp_{}_{}", t.server_name, t.name) == tool_name)
                        .expect("tool vanished between lock acquisitions");
                    // Workaround for Playwright MCP server bug: the server
                    // doesn't respect the output dir for named fullpage
                    // screenshots, so we prepend the pw-media path here.
                    if tool_name.contains("screenshot") {
                        for key in &["filename", "name"] {
                            if let Some(name_val) = args_clone.get(*key).and_then(|v| v.as_str()) {
                                if !name_val.is_empty() && !name_val.starts_with('/') && !name_val.contains('/') {
                                    let media_dir = &s.config.media_dir;
                                    args_clone[*key] =
                                        serde_json::json!(format!("{}/pw-media/{}", media_dir, name_val));
                                    break;
                                }
                            }
                        }
                    }
                    let tc = s.mcp_tools[tool_idx].clone();
                    let server_name = tc.server_name.clone();
                    (tc, server_name)
                };
                // Phase 3: invoke (outside state lock so HTTP calls don't block other ops).
                // The per-server lock is passed through but only acquired inside
                // invoke_tool_impl during the rare session re-init path.
                let result =
                    crate::mcp::invoke_tool_impl(&mut tc, &args_clone, Some(&server_lock)).await;
                // Phase 4: under state lock — propagate updated session_id, if any
                if tc.session_id.is_some() {
                    let mut s = state.lock().await;
                    for t in &mut s.mcp_tools {
                        if t.server_name == server_name {
                            t.session_id = tc.session_id.clone();
                        }
                    }
                }
                result
            };
            result
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
        "read_config_schema" => {
            bot_dispatch_config::tool_read_config_schema().await
        }
        "read_config" => {
            bot_dispatch_config::tool_read_config(state).await
        }
        "edit_config" => {
            bot_dispatch_config::tool_edit_config(state, &cid, args, tg_bot).await
        }
        "get_model_info" => {
            bot_dispatch_model::tool_get_model_info(state, &cid).await
        }
        "propose_model_change" => {
            bot_dispatch_model::tool_propose_model_change(state, &cid, args, tg_bot).await
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
                s.db
                    .search_embeddings(&cid, &query_embedding, &embedding_model, search_limit)
                    .unwrap_or_default()
            };

            let top_results: Vec<_> = results
                .into_iter()
                .take(count)
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
