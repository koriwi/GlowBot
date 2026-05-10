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

    // Emit a warning when tools return errors, info otherwise
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
                let chat = ChatId(cid.parse().unwrap_or_default());
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
                let s = state.lock().await;
                // Workaround for Playwright MCP server bug: the server
                // doesn't respect the output dir for named fullpage
                // screenshots, so we prepend the pw-media path here.
                // The tool uses either "name" or "filename" as the
                // parameter for the output filename.
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
                // Defense in depth: also check the blacklist at dispatch time
                let tool_idx = s
                    .mcp_tools
                    .iter()
                    .position(|t| format!("mcp_{}_{}", t.server_name, t.name) == tool_name);
                match tool_idx.map(|idx| (idx, s.config.is_mcp_server_allowed(chat_id, &s.mcp_tools[idx].server_name))) {
                    Some((_, false)) => {
                        format!("MCP tool blacklisted for this chat: {}", tool_name)
                    }
                    Some((idx, true)) => {
                        let mut tc = s.mcp_tools[idx].clone();
                        let server = tc.server_name.clone();
                        drop(s);
                        let result = crate::mcp::invoke_tool(&mut tc, &args_clone).await;
                        // After invoke_tool may have updated session_id via re-init.
                        // Propagate to ALL tools from the same server so subsequent
                        // calls don't each need their own re-initialization.
                        let mut s = state.lock().await;
                        if tc.session_id.is_some() {
                            for t in &mut s.mcp_tools {
                                if t.server_name == server {
                                    t.session_id = tc.session_id.clone();
                                }
                            }
                        }
                        result
                    }
                    None => {
                        format!("MCP tool not found: {}", tool_name)
                    }
                }
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
        "read_config" => {
            bot_dispatch_config::tool_read_config(state).await
        }
        "edit_config" => {
            bot_dispatch_config::tool_edit_config(state, &cid, args, tg_bot).await
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
