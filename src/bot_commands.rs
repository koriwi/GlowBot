use super::BotState;
use crate::config::McpServer;
use crate::git::GitRepo;
use crate::mcp::McpToolInfo;
use crate::openrouter::{OpenRouterClient, ToolDefinition};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle a bot command (free function).
pub(crate) async fn handle_bot_command_impl(
    state: &Arc<Mutex<BotState>>,
    stop_signals: &Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    chat_id: &str,
    user_id: &str,
    command: &crate::commands::Command,
    tg_bot: Option<&teloxide::Bot>,
    _git_repo: &GitRepo,
) -> anyhow::Result<Option<String>> {
    // Commands that are always allowed in DMs regardless of commands_enabled.
    let is_always_allowed = matches!(
        command,
        crate::commands::Command::Todos(_)
            | crate::commands::Command::Tasks
            | crate::commands::Command::Reminders
            | crate::commands::Command::Stop
    );

    let allowed = if is_always_allowed {
        true
    } else {
        let s = state.lock().await;
        let is_dm = !chat_id.starts_with('-');
        if is_dm {
            s.config
                .dm_config(chat_id)
                .map(|d| d.commands_enabled)
                .unwrap_or(false)
        } else {
            let chat_config = s.config.chat_config(chat_id);
            crate::commands::can_run_command(&chat_config, user_id)
        }
    };

    if !allowed {
        return Ok(Some("You are not authorized to run bot commands.".into()));
    }

    // /stop sets the stop signal and returns immediately
    if matches!(command, crate::commands::Command::Stop) {
        if let Ok(signals) = stop_signals.lock() {
            if let Some(signal) = signals.get(chat_id) {
                signal.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        return Ok(Some(
            "Stop signal sent. Current operations will be cancelled.".into(),
        ));
    }

    if matches!(command, crate::commands::Command::Tasks) {
        let s = state.lock().await;
        let list = crate::tasks::TaskList::load(&s.chats_dir(), chat_id).unwrap_or_default();
        let response = if list.tasks.is_empty() {
            "No pending tasks for this chat.".to_string()
        } else {
            let mut lines = vec![format!("*{} pending task(s):*", list.tasks.len())];
            for (i, t) in list.tasks.iter().enumerate() {
                lines.push(format!("{}. `{}` — {}", i + 1, t.id, t.description));
            }
            lines.join("\n")
        };
        return Ok(Some(response));
    }

    if matches!(command, crate::commands::Command::Todos(_)) {
        let details = matches!(command, crate::commands::Command::Todos(true));
        // If we have a tg_bot, use inline keyboard buttons
        if let Some(bot) = tg_bot {
            let state_clone = Arc::clone(state);
            let cid = chat_id.to_string();
            let tg_clone = bot.clone();
            tokio::spawn(async move {
                crate::bot::bot_todos::send_todos_message(
                    &state_clone, &cid, details, &tg_clone,
                )
                .await;
            });
            return Ok(None);
        }
        // Fallback: plain text (no tg_bot available)
        let s = state.lock().await;
        let list = crate::todos::TodoList::load(&s.chats_dir(), chat_id).unwrap_or_default();
        let items = list.display_items(3);
        let response = if items.is_empty() {
            "No todos for this chat.".to_string()
        } else if details {
            let mut lines = vec![format!("*{} todo(s) (detailed):*", items.len())];
            for (i, t) in items.iter().enumerate() {
                let status = if t.completed { "✅" } else { "⬜" };
                let updated = t.updated_at.as_deref().unwrap_or("never");
                lines.push(format!(
                    "{}. {} `{}` — {}\n   Created: {} | Updated: {}",
                    i + 1, status, t.id, t.description, t.created_at, updated
                ));
            }
            lines.join("\n")
        } else {
            let mut lines = vec![format!("*{} todo(s):*", items.len())];
            for (i, t) in items.iter().enumerate() {
                let status = if t.completed { "✅" } else { "⬜" };
                lines.push(format!("{}. {} {}", i + 1, status, t.description));
            }
            lines.join("\n")
        };
        return Ok(Some(response));
    }

    if matches!(command, crate::commands::Command::Reminders) {
        let s = state.lock().await;
        let list =
            crate::reminders::ReminderList::load(&s.chats_dir(), chat_id).unwrap_or_default();
        let response = if list.reminders.is_empty() {
            "No pending reminders for this chat.".to_string()
        } else {
            let mut lines = vec![format!(
                "*{} pending reminder(s):*",
                list.reminders.len()
            )];
            for (i, r) in list.reminders.iter().enumerate() {
                let action_note = if r.action.is_some() {
                    " [has action]"
                } else {
                    ""
                };
                lines.push(format!(
                    "{}. `{}` — {} @ {}{}",
                    i + 1,
                    r.id,
                    r.description,
                    r.trigger_at,
                    action_note
                ));
            }
            lines.join("\n")
        };
        return Ok(Some(response));
    }

    // /new sets a forget cutoff timestamp — messages before this are excluded from context
    if matches!(command, crate::commands::Command::New) {
        let ts = chrono::Utc::now().timestamp();
        {
            let s = state.lock().await;
            s.db.set_cutoff(chat_id, ts)?;
        }
        let formatted = chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| ts.to_string());
        return Ok(Some(format!(
            "🆕 Context reset. All messages before {} are now excluded from future conversation context.",
            formatted
        )));
    }

    // /prompt shows the system prompt that would be sent to the LLM
    if matches!(command, crate::commands::Command::Prompt) {
        let prompt_text = {
            let s = state.lock().await;
            let is_dm = !chat_id.starts_with('-');
            let tools_enabled = if is_dm {
                s.config.dm_config(chat_id).is_some()
            } else {
                true
            };
            s.assemble_system_prompt(chat_id, tools_enabled, user_id)
        };

        return Ok(Some(prompt_text));
    }

    // /tools — list all tools available in this chat
    if matches!(command, crate::commands::Command::Tools) {
        let output = {
            let s = state.lock().await;
            let is_dm = !chat_id.starts_with('-');
            let tools_enabled = if is_dm {
                s.config.dm_config(chat_id).is_some()
            } else {
                true
            };
            let bash_enabled = if tools_enabled {
                s.config.is_bash_enabled(chat_id)
            } else {
                false
            };
            let builtin_tools = s.build_tools(bash_enabled, chat_id);
            let mcp_tools: Vec<_> = s
                .mcp_tools
                .iter()
                .filter(|mt| s.config.is_mcp_server_allowed(chat_id, &mt.server_name))
                .collect();
            let mcp_blacklisted: Vec<_> = s
                .mcp_tools
                .iter()
                .filter(|mt| !s.config.is_mcp_server_allowed(chat_id, &mt.server_name))
                .collect();
            format_tools_output(&builtin_tools, &mcp_tools, &s.mcp_tools, &mcp_blacklisted, &s.config.mcp_servers)
        };
        return Ok(Some(output));
    }

    // /config — show the current config with sensitive fields redacted
    if matches!(command, crate::commands::Command::Config) {
        let yaml = {
            let s = state.lock().await;
            let redacted = s.config.redacted();
            serde_yaml::to_string(&redacted).unwrap_or_else(|e| format!("Error: {}", e))
        };
        return Ok(Some(format!("```yaml\n{}\n```", yaml)));
    }

    // /config_schema — show the JSON Schema for all config fields
    if matches!(command, crate::commands::Command::ConfigSchema) {
        let schema = super::bot_dispatch::bot_dispatch_config::tool_read_config_schema().await;
        return Ok(Some(format!("```json\n{}\n```", schema)));
    }

    // /model_default (alias /model_reset) — reset temporary model override
    if matches!(command, crate::commands::Command::ModelDefault) {
        {
            let mut s = state.lock().await;
            s.model_overrides.remove(chat_id);
        }
        let model = {
            let s = state.lock().await;
            s.config.model_for_chat(chat_id).to_string()
        };
        return Ok(Some(format!("🔁 Model reset to config default: `{}`", model)));
    }

    // /model [model-id|:specifier] — set model override, apply specifier, or show info
    if let crate::commands::Command::Model(arg) = command {
        match arg {
            None => {
                // No args: show current model info with specifier buttons
                let bot = match tg_bot {
                    Some(b) => b.clone(),
                    None => {
                        let (current, default) = {
                            let s = state.lock().await;
                            let cur = s.effective_model(chat_id);
                            let def = s.config.model_for_chat(chat_id).to_string();
                            (cur, def)
                        };
                        let has_override = {
                            let s = state.lock().await;
                            s.model_overrides.contains_key(chat_id)
                        };
                        let msg = format!(
                            "🎯 Current model: `{}`{}\n📌 Config default: `{}`\n\nSet model: `/model <model-id>`\nSwitch routing: `/model :nitro` | `:floor` | `:free`",
                            current,
                            if has_override { " (override)" } else { "" },
                            default
                        );
                        return Ok(Some(msg));
                    }
                };
                let state_clone = Arc::clone(state);
                let cid = chat_id.to_string();
                tokio::spawn(async move {
                    let _ = super::bot_models::send_model_info(&state_clone, &cid, bot).await;
                });
                return Ok(None);
            }
            Some(ref model_arg) if model_arg.starts_with(':') => {
                // Specifier switch: apply :nitro, :floor, :free to current model
                let specifier = &model_arg[1..]; // strip the leading ':'

                // Validate specifier
                let valid_specifiers: Vec<&str> = crate::openrouter::SPECIFIER_BUTTONS
                    .iter()
                    .map(|(s, _)| *s)
                    .collect();
                if !valid_specifiers.contains(&specifier) {
                    let list = valid_specifiers
                        .iter()
                        .map(|s| format!("`:{}`", s))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Ok(Some(format!(
                        "Unknown specifier `:{}`. Valid specifiers: {}",
                        specifier, list
                    )));
                }

                let new_model = {
                    let s = state.lock().await;
                    let current = s.effective_model(chat_id);
                    crate::openrouter::apply_specifier(&current, specifier)
                };
                {
                    let mut s = state.lock().await;
                    s.model_overrides
                        .insert(chat_id.to_string(), new_model.clone());
                }
                return Ok(Some(format!(
                    "✅ Model set to `{}` (specifier `:{}` applied)",
                    new_model, specifier
                )));
            }
            Some(ref model_id) => {
                // Direct model override
                {
                    let mut s = state.lock().await;
                    s.model_overrides
                        .insert(chat_id.to_string(), model_id.clone());
                }
                return Ok(Some(format!("✅ Model set to `{}`", model_id)));
            }
        }
    }

    // /models — browse and switch models via inline keyboard
    if matches!(command, crate::commands::Command::Models) {
        let bot = match tg_bot {
            Some(b) => b.clone(),
            None => return Ok(Some("Models command cannot be used in this context (no Telegram bot available).".into())),
        };
        let state_clone = Arc::clone(state);
        let cid = chat_id.to_string();
        tokio::spawn(async move {
            let _ = super::bot_models::send_model_menu(&state_clone, &cid, bot).await;
        });
        return Ok(None); // Response is sent via the spawned task
    }

    // /run triggers the heartbeat task agent immediately for this chat
    if matches!(command, crate::commands::Command::Run) {
        if let Some(bot) = tg_bot {
            let state_clone = Arc::clone(state);
            let git_clone = _git_repo.clone();
            let stop_clone = Arc::clone(stop_signals);
            let cid = chat_id.to_string();
            let tg_clone = bot.clone();
            tokio::spawn(async move {
                crate::bot::run_heartbeat_task(state_clone, git_clone, stop_clone, &cid, tg_clone).await;
            });
            return Ok(Some("🔄 Running task agent for this chat now...".into()));
        }
        return Ok(Some("Run command cannot be used in this context.".into()));
    }

    // /status — show effective model (including temporary overrides)
    if matches!(command, crate::commands::Command::Status) {
        // If the model's context length isn't cached, fetch it on-demand
        {
            let s = state.lock().await;
            let model = s.effective_model(chat_id);
            let needs_fetch = !s.model_metadata.contains_key(crate::openrouter::normalize_model_id(&model));
            let api_key = s.config.openrouter.api_key.clone();
            drop(s);

            if needs_fetch {
                let client = OpenRouterClient::new(api_key);
                match client.fetch_models().await {
                    Ok(models) => {
                        let mut s = state.lock().await;
                        let old_count = s.model_metadata.len();
                        for m in models {
                            s.model_order.push(m.id.clone());
                            s.model_metadata.insert(m.id.clone(), m);
                        }
                        log::info!(
                            "Fetched {} model metadata entries on-demand for /status (had {} before)",
                            s.model_metadata.len() - old_count,
                            old_count
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to fetch model metadata for /status: {}", e);
                    }
                }
            }
        }

        let response = {
            let mut s = state.lock().await;
            let usage = s.context_usage(chat_id);
            let model = s.effective_model(chat_id);
            crate::commands::handle_command_with_model(command, &mut s.config, chat_id, &usage, Some(&model))
        };
        return Ok(Some(response));
    }

    let response = {
        let mut s = state.lock().await;
        let usage = s.context_usage(chat_id);
        crate::commands::handle_command(command, &mut s.config, chat_id, &usage)
    };

    Ok(Some(response))
}

/// Format the /tools command output: MCP server summary at top, then grouped tool list.
fn format_tools_output(
    builtin_tools: &[ToolDefinition],
    mcp_tools: &[&McpToolInfo],
    all_mcp_tools: &[McpToolInfo],
    mcp_blacklisted: &[&McpToolInfo],
    mcp_servers: &[McpServer],
) -> String {
    let mut out = String::new();
    out.push_str("🛠 *Available Tools*\n");

    // --- MCP Servers summary ---
    if !mcp_servers.is_empty() {
        out.push_str("\n*MCP Servers:*\n");
        for server in mcp_servers {
            let tool_count = all_mcp_tools
                .iter()
                .filter(|t| t.server_name == server.name)
                .count();
            let blacklisted_count = mcp_blacklisted
                .iter()
                .filter(|t| t.server_name == server.name)
                .count();
            let effective = tool_count - blacklisted_count;
            let status = if blacklisted_count > 0 {
                format!(
                    " — {} available, {} blacklisted for this chat",
                    effective, blacklisted_count
                )
            } else {
                format!(" — {} tool(s)", tool_count)
            };
            out.push_str(&format!(
                "• `{}` ({}, {}){}\n",
                server.name, server.url, server.transport, status
            ));
        }
    }

    // --- Built-in tools ---
    // Separate MCP tools from built-in by checking name prefix
    let builtins: Vec<_> = builtin_tools
        .iter()
        .filter(|t| !t.function.name.starts_with("mcp_"))
        .collect();
    let mcp: Vec<_> = builtin_tools
        .iter()
        .filter(|t| t.function.name.starts_with("mcp_"))
        .collect();

    out.push_str("\n*Built-in:*\n");
    for t in &builtins {
        out.push_str(&format!(
            "• `{}` — {}\n",
            t.function.name, t.function.description
        ));
    }

    // --- MCP tools grouped by server ---
    if !mcp.is_empty() {
        // Group by server name
        let mut mcp_by_server: HashMap<String, Vec<&ToolDefinition>> = HashMap::new();
        for t in &mcp {
            let server_name = mcp_tools
                .iter()
                .find(|mt| format!("mcp_{}_{}", mt.server_name, mt.name) == t.function.name)
                .map(|mt| mt.server_name.as_str())
                .unwrap_or("unknown");
            mcp_by_server
                .entry(server_name.to_string())
                .or_default()
                .push(t);
        }
        let mut server_names: Vec<_> = mcp_by_server.keys().collect();
        server_names.sort();
        for server_name in server_names {
            let tools = &mcp_by_server[server_name];
            out.push_str(&format!("\n*MCP: {}*\n", server_name));
            for t in tools {
                out.push_str(&format!(
                    "• `{}` — {}\n",
                    t.function.name, t.function.description
                ));
            }
        }
    }

    out
}
