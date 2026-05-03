use super::BotState;
use crate::openrouter::{ChatMessage, ToolCall};
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

/// Log a tool call to `tool_calls.log` in the given data directory.
pub(crate) fn log_tool_call_to(data_dir: &std::path::Path, tool_name: &str, args: &str, result: &str) {
    let log_path = data_dir.join("tool_calls.log");
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let result_summary: String = result.chars().take(200).collect();
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
    log::info!("tool {}: {}", tool_name, args_summary);
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
                match bot.send_message(chat, text).await {
                    Ok(_) => "Message sent.".into(),
                    Err(e) => format!("Failed to send message: {}", e),
                }
            } else {
                "Error: send_message not available in this context.".into()
            }
        }
        "bash" => {
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
            let uid = args["user_id"].as_str().unwrap_or("");
            let s = state.lock().await;
            match crate::memory::load_memory(&s.chats_dir(), &cid, uid) {
                Some(m) => serde_json::json!({
                    "user_id": m.frontmatter.user_id,
                    "username": m.frontmatter.username,
                    "call_name": m.frontmatter.call_name,
                    "description": m.frontmatter.description,
                    "body": m.body,
                })
                .to_string(),
                None => format!("No memory file found for user_id={} in chat {}", uid, cid),
            }
        }
        "update_memory" => {
            let uid = args["user_id"].as_str().unwrap_or("");
            if uid.is_empty() {
                return "Error: user_id is required".into();
            }
            let s = state.lock().await;
            let chats_dir = s.chats_dir();
            let mut mem = crate::memory::load_memory(&chats_dir, &cid, uid)
                .unwrap_or_else(|| crate::memory::Memory::new(uid, ""));
            let mut changed = false;
            if let Some(v) = args["username"].as_str() {
                mem.frontmatter.username = v.into();
                changed = true;
            }
            if let Some(v) = args["call_name"].as_str() {
                mem.frontmatter.call_name = v.into();
                changed = true;
            }
            if let Some(v) = args["description"].as_str() {
                mem.frontmatter.description = v.into();
                changed = true;
            }
            if let Some(v) = args["log_entry"].as_str() {
                mem.append_log(v);
                changed = true;
            }
            if changed {
                match crate::memory::save_memory(&chats_dir, &cid, uid, &mem) {
                    Ok(()) => format!("Memory updated for {}", uid),
                    Err(e) => format!("Error: {}", e),
                }
            } else {
                "No fields to update.".into()
            }
        }
        "read_chat_memory" => {
            let s = state.lock().await;
            match crate::memory::load_chat_memory(&s.chats_dir(), &cid) {
                Some(m) => serde_json::json!({
                    "call_name": m.frontmatter.call_name,
                    "description": m.frontmatter.description,
                    "body": m.body,
                })
                .to_string(),
                None => format!("No chat memory for {}", cid),
            }
        }
        "update_chat_memory" => {
            let s = state.lock().await;
            let chats_dir = s.chats_dir();
            let mut mem = crate::memory::load_chat_memory(&chats_dir, &cid)
                .unwrap_or_else(crate::memory::Memory::new_chat);
            let mut changed = false;
            if let Some(v) = args["call_name"].as_str() {
                mem.frontmatter.call_name = v.into();
                changed = true;
            }
            if let Some(v) = args["description"].as_str() {
                mem.frontmatter.description = v.into();
                changed = true;
            }
            if let Some(v) = args["log_entry"].as_str() {
                mem.append_log(v);
                changed = true;
            }
            if changed {
                match crate::memory::save_chat_memory(&chats_dir, &cid, &mem) {
                    Ok(()) => "Chat memory updated".into(),
                    Err(e) => format!("Error: {}", e),
                }
            } else {
                "No fields to update.".into()
            }
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
        "create_skill" => {
            let name = args["name"].as_str().unwrap_or("");
            let desc = args["description"].as_str().unwrap_or("");
            let body = args["body"].as_str().unwrap_or("");
            if name.is_empty() || desc.is_empty() || body.is_empty() {
                return "Error: name, description, body required".into();
            }
            let s = state.lock().await;
            let fm = crate::skills::SkillFrontmatter {
                name: name.into(),
                description: desc.into(),
            };
            match crate::skills::write_skill(&s.skills_dir(), name, &fm, body) {
                Ok(_) => format!("Skill '{}' created", name),
                Err(e) => format!("Error: {}", e),
            }
        }
        "read_skill" => {
            let name = args["name"].as_str().unwrap_or("");
            if name.is_empty() {
                return "Error: name required".into();
            }
            let s = state.lock().await;
            let path = s.skills_dir().join(name).join("skill.md");
            match crate::skills::load_skill(&path) {
                Ok(skill) => serde_json::json!({
                    "name": skill.frontmatter.name,
                    "description": skill.frontmatter.description,
                    "body": skill.body,
                })
                .to_string(),
                Err(_) => format!("Skill '{}' not found", name),
            }
        }
        "update_skill" => {
            let name = args["name"].as_str().unwrap_or("");
            if name.is_empty() {
                return "Error: name required".into();
            }
            let s = state.lock().await;
            let path = s.skills_dir().join(name).join("skill.md");
            let mut skill = match crate::skills::load_skill(&path) {
                Ok(s) => s,
                Err(_) => return format!("Skill '{}' not found", name),
            };
            let mut changed = false;
            if let Some(v) = args["description"].as_str() {
                skill.frontmatter.description = v.into();
                changed = true;
            }
            if let Some(v) = args["body"].as_str() {
                skill.body = v.into();
                changed = true;
            }
            if !changed {
                return "No fields to update.".into();
            }
            let yaml = serde_yaml::to_string(&skill.frontmatter).unwrap_or_default();
            let content = format!("---\n{}---\n{}", yaml, skill.body);
            match std::fs::write(&path, &content) {
                Ok(()) => format!("Skill '{}' updated", name),
                Err(e) => format!("Error: {}", e),
            }
        }
        name if name.starts_with("mcp_") => {
            let s = state.lock().await;
            match s
                .mcp_tools
                .iter()
                .find(|t| format!("mcp_{}_{}", t.server_name, t.name) == name)
            {
                Some(t) => {
                    let tc = t.clone();
                    drop(s);
                    crate::mcp::invoke_tool(&tc, args).await
                }
                None => format!("MCP tool not found: {}", name),
            }
        }
        "get_recent_messages" => {
            let count = args["count"].as_i64().unwrap_or(10) as usize;
            let count = count.clamp(1, 50);
            let history = {
                let s = state.lock().await;
                s.db.load_messages(&cid, count).unwrap_or_default()
            };
            let items: Vec<_> = history.iter()
                .map(|m| serde_json::json!({
                    "role": &m.role,
                    "content": m.text_content(),
                    "name": m.name.as_deref().unwrap_or("")
                }))
                .collect();
            serde_json::json!({"messages": items}).to_string()
        }
        _ => format!("Unknown tool: {}", tool_name),
    }
}
