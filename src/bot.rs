use crate::commands::{can_interact, can_run_command, handle_command, parse_command};
use crate::config::Config;
use crate::git::GitRepo;
use crate::llm::LlmBackend;
use crate::memory::{save_memory, Memory};
use crate::openrouter::{ChatCompletionRequest, ChatMessage, ToolCall};
use crate::skills::{load_all_skills, Skill};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

/// Shared bot state accessible from all handlers.
pub struct BotState {
    pub config: Config,
    pub skills: HashMap<String, Skill>,
    pub llm: Arc<dyn LlmBackend>,
    pub data_dir: std::path::PathBuf,
    /// Per-chat conversation history (sliding window of recent messages).
    pub conversation_history: HashMap<String, Vec<ChatMessage>>,
    /// Tools discovered from MCP servers.
    pub mcp_tools: Vec<crate::mcp::McpTool>,
}

impl BotState {
    /// Get the path to the chats directory.
    pub fn chats_dir(&self) -> std::path::PathBuf {
        self.data_dir.join("chats")
    }

    /// Get the path to the skills directory.
    pub fn skills_dir(&self) -> std::path::PathBuf {
        self.data_dir.join("skills")
    }

    /// Get the path to the config file.
    pub fn config_path(&self) -> std::path::PathBuf {
        self.data_dir.join("config.yaml")
    }

    /// Assemble the full system prompt for a chat, loading memories and skills.
    pub fn assemble_system_prompt(
        &self,
        chat_id: &str,
        tools_enabled: bool,
        user_id: &str,
    ) -> String {
        let skills = &self.skills;
        let memories =
            crate::memory::load_chat_memories(&self.chats_dir(), chat_id).unwrap_or_default();
        let chat_memory = crate::memory::load_chat_memory(&self.chats_dir(), chat_id);
        let chat_config = self.config.chat_config(chat_id);
        crate::system_prompt::assemble(
            chat_id,
            &chat_config.system_prompt,
            skills,
            chat_memory.as_ref(),
            &memories,
            tools_enabled,
            user_id,
        )
    }

    /// Get the effective model for a chat.
    pub fn effective_model(&self, chat_id: &str) -> String {
        self.config.model_for_chat(chat_id).to_string()
    }

    /// Build the full list of tool definitions including MCP tools.
    /// `include_send_message` controls whether the `send_message` tool is
    /// included (used by heartbeat tasks); normal conversation filters it out
    /// because the assistant reply itself is the message.
    pub fn build_tools(&self, include_send_message: bool) -> Vec<crate::openrouter::ToolDefinition> {
        let mut t = crate::openrouter::all_tool_definitions();
        if !include_send_message {
            t.retain(|tool| tool.function.name != "send_message");
        }
        for mt in &self.mcp_tools {
            t.push(crate::openrouter::ToolDefinition {
                def_type: "function".into(),
                function: crate::openrouter::FunctionDef {
                    name: format!("mcp_{}_{}", mt.server_name, mt.name),
                    description: format!("[MCP: {}] {}", mt.server_name, mt.description),
                    parameters: mt.input_schema.clone(),
                },
            });
        }
        t
    }

    /// Check if a chat has pending tasks.
    pub fn has_pending_tasks(&self, chat_id: &str) -> bool {
        let list = crate::tasks::TaskList::load(&self.chats_dir(), chat_id).unwrap_or_default();
        list.has_tasks()
    }
}

/// Main GlowBot orchestrator.
pub struct GlowBot {
    pub state: Arc<Mutex<BotState>>,
    pub git_repo: GitRepo,
}

impl GlowBot {
    // Methods below are tested directly; dispatch_tool is the canonical path.
    #[allow(dead_code)]
    /// Create a new GlowBot instance with the given LLM backend.
    pub async fn new_with_llm(data_dir: &Path, llm: Arc<dyn LlmBackend>) -> anyhow::Result<Self> {
        let config_path = data_dir.join("config.yaml");
        let config = Config::load(&config_path)?;
        let skills_dir = data_dir.join("skills");
        let skills = load_all_skills(&skills_dir)?;
        let git_repo = GitRepo::new(data_dir);

        // Initialize git if needed
        if !git_repo.is_repo() {
            git_repo.init()?;
        }

        // Discover MCP server tools
        let mcp_tools = crate::mcp::discover_all(&config.mcp_servers).await?;
        if !mcp_tools.is_empty() {
            log::info!(
                "Loaded {} MCP tools from {} server(s)",
                mcp_tools.len(),
                config.mcp_servers.len()
            );
        }

        let state = BotState {
            config,
            skills,
            llm,
            data_dir: data_dir.to_path_buf(),
            conversation_history: HashMap::new(),
            mcp_tools,
        };

        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            git_repo,
        })
    }

    /// Reload skills from disk.
    pub async fn reload_skills(&self) -> anyhow::Result<()> {
        let mut state = self.state.lock().await;
        let skills_dir = state.skills_dir();
        state.skills = load_all_skills(&skills_dir)?;
        Ok(())
    }

    /// Save config to disk and auto-commit.
    pub async fn save_config(&self) -> anyhow::Result<()> {
        let state = self.state.lock().await;
        let path = state.config_path();
        state.config.save(&path)?;
        drop(state);
        self.git_repo
            .auto_commit("Update configuration via /command")?;
        Ok(())
    }

    /// Process an incoming message and return the response.
    /// Returns None if no response should be sent.
    pub async fn process_message(
        &self,
        chat_id: &str,
        user_id: &str,
        username: &str,
        text: &str,
        bot_username: &str,
    ) -> anyhow::Result<Option<String>> {
        let is_command = text.trim().starts_with('/');
        let is_mention = text.contains(&format!("@{}", bot_username));

        // Check if it's a bot command
        if let Some(command) = parse_command(text) {
            return self.handle_bot_command(&command, chat_id, user_id).await;
        }

        // Check interaction permissions
        let chat_config = {
            let state = self.state.lock().await;
            state.config.chat_config(chat_id)
        };

        if !can_interact(&chat_config, user_id) {
            return Ok(None); // User not allowed to interact
        }

        // In mention_only mode, only respond to mentions (groups only; DMs always respond)
        let is_dm = !chat_id.starts_with('-');
        if !is_dm
            && matches!(
                chat_config.interaction_mode,
                crate::config::InteractionMode::MentionOnly
            )
            && !is_command
            && !is_mention
        {
            return Ok(None);
        }

        // If it's a plain command (not a bot command), ignore
        if is_command && !is_mention {
            return Ok(None);
        }

        // DM whitelist check: if non-empty and user not listed, block entirely
        let (tools_enabled, dm_blocked) = {
            let state = self.state.lock().await;
            let config = &state.config;
            if is_dm {
                if config.dm_whitelist.is_empty() {
                    // Empty whitelist = respond but no tools
                    (false, false)
                } else if config.dm_whitelist.contains(&user_id.to_string()) {
                    // User in whitelist = full access
                    (true, false)
                } else {
                    // Non-empty whitelist, user not in it = blocked
                    (false, true)
                }
            } else {
                // Groups always have tools enabled
                (true, false)
            }
        };

        if dm_blocked {
            return Ok(Some(
                "Sorry, you're not authorized to interact with me in DMs.".into(),
            ));
        }

        // Process with LLM
        self.process_with_llm(chat_id, user_id, username, text, tools_enabled)
            .await
    }

    /// Handle a bot command (/model, /mode, /reload, /status).
    async fn handle_bot_command(
        &self,
        command: &crate::commands::Command,
        chat_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<String>> {
        // Check command permissions.
        // In DMs, also allow users in the global dm_whitelist to run commands.
        let allowed = {
            let state = self.state.lock().await;
            let chat_config = state.config.chat_config(chat_id);
            let is_dm = !chat_id.starts_with('-');
            if is_dm && state.config.dm_whitelist.contains(&user_id.to_string()) {
                true
            } else {
                can_run_command(&chat_config, user_id)
            }
        };

        if !allowed {
            return Ok(Some("You are not authorized to run bot commands.".into()));
        }

        let needs_save = matches!(
            command,
            crate::commands::Command::Model(_) | crate::commands::Command::Mode(_)
        );
        let reload_needed = matches!(command, crate::commands::Command::Reload);

        let response = {
            let mut state = self.state.lock().await;
            handle_command(command, &mut state.config, chat_id)
        };

        if needs_save {
            self.save_config().await?;
        }

        if reload_needed {
            self.reload_skills().await?;
        }

        Ok(Some(response))
    }

    /// Process a message through the LLM pipeline.
    async fn process_with_llm(
        &self,
        chat_id: &str,
        user_id: &str,
        username: &str,
        text: &str,
        tools_enabled: bool,
    ) -> anyhow::Result<Option<String>> {
        let (system_prompt, model) = {
            let state = self.state.lock().await;
            let system_prompt =
                state.assemble_system_prompt(chat_id, tools_enabled, user_id);
            let model = state.effective_model(chat_id);
            (system_prompt, model)
        };

        // Ensure user has a memory file
        self.ensure_memory_exists(chat_id, user_id, username)
            .await?;

        // Build messages: system prompt + current message only (history on demand)
        let current_msg = ChatMessage::user_with_name(text, username);
        let mut messages = vec![
            ChatMessage::system(&system_prompt),
            current_msg.clone(),
        ];

        let tools: Vec<crate::openrouter::ToolDefinition> = if tools_enabled {
            let state = self.state.lock().await;
            state.build_tools(false)
        } else {
            vec![]
        };
        let max_tool_rounds = 10;

        // Run the LLM tool-use loop, capturing the final response.
        let result = {
            let mut result = None;
            for _round in 0..max_tool_rounds {
                let request = ChatCompletionRequest {
                    model: model.clone(),
                    messages: messages.clone(),
                    tools: Some(tools.clone()),
                    tool_choice: None,
                };

                let response = {
                    let state = self.state.lock().await;
                    state.llm.chat_completion(&request).await?
                };

                let choice = match response.choices.into_iter().next() {
                    Some(c) => c,
                    None => break,
                };

                // Check for tool calls
                if let Some(tool_calls) = &choice.message.tool_calls {
                    if tool_calls.is_empty() {
                        result = Some(choice.message.content.clone().unwrap_or_default());
                        break;
                    }

                    // Add the assistant message with tool calls
                    messages.push(ChatMessage::assistant_tool_calls(tool_calls.clone()));

                    // Dispatch all tool calls (with logging)
                    let data_dir = { self.state.lock().await.data_dir.clone() };
                    let results = dispatch_tool_calls(
                        &self.state,
                        chat_id,
                        tool_calls,
                        Some(&data_dir),
                        None,
                    )
                    .await;
                    messages.extend(results);

                    // Auto-commit after tool execution (tools may have modified files)
                    self.git_repo
                        .auto_commit("Auto-commit after tool execution")?;
                    continue;
                }

                // No tool calls — final response
                result = Some(choice.message.content.clone().unwrap_or_default());
                break;
            }

            // If we exhausted rounds, give a loop error
            result.unwrap_or_else(|| {
                "I ran into a loop processing your request. Please try again.".into()
            })
        };

        // Store in conversation history
        {
            let mut state = self.state.lock().await;
            let window = state.config.conversation_window;
            let history = state
                .conversation_history
                .entry(chat_id.to_string())
                .or_default();
            history.push(current_msg);
            history.push(ChatMessage::assistant(&result));
            while history.len() > window {
                history.remove(0);
            }
        }

        Ok(Some(result))
    }

    /// Ensure a memory file exists for the given user in the given chat.
    async fn ensure_memory_exists(
        &self,
        chat_id: &str,
        user_id: &str,
        username: &str,
    ) -> anyhow::Result<()> {
        let state = self.state.lock().await;
        let existing = crate::memory::load_memory(&state.chats_dir(), chat_id, user_id);
        if existing.is_none() {
            let mem = Memory::new(user_id, username);
            save_memory(&state.chats_dir(), chat_id, user_id, &mem)?;
        }
        Ok(())
    }

}

/// Run a heartbeat background task for a chat. Uses the state directly
/// Run a heartbeat background task for a chat. Processes every pending task
/// at most once per invocation. Cycles through tasks in order; if we loop back
/// to a task already handled this cycle, or the queue becomes empty, we exit.
pub async fn run_heartbeat_task(
    state: Arc<Mutex<BotState>>,
    git_repo: crate::git::GitRepo,
    chat_id: &str,
    tg_bot: teloxide::Bot,
) {
    let cid = chat_id.to_string();
    let mut tried_this_cycle: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        let (task_id, task_desc) = {
            let s = state.lock().await;
            let list = crate::tasks::TaskList::load(&s.chats_dir(), &cid).unwrap_or_default();
            match list.oldest() {
                Some(t) => (t.id.clone(), t.description.clone()),
                None => break,
            }
        };

        if tried_this_cycle.contains(&task_id) {
            break;
        }
        tried_this_cycle.insert(task_id.clone());

        log::info!("Heartbeat chat {}: working on task '{}'", cid, task_id);

        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let task_header = format!(
            "## Background Task\n\
            You are processing a scheduled task for this chat.\n\
            Task: {task_desc}\n\
            Instructions:\n\
            - Use your available tools to complete the task.\n\
            - When done, call remove_task(\"{task_id}\") to mark it complete.\n\
            - If the task spawns follow-up work, call add_task(\"...\") for each.\n\
            - If the task cannot be completed yet (e.g. download still in progress, waiting for external event),\n\
              just leave it — do NOT remove it. It will run again next cycle.\n\
            - You may send at most ONE message to the chat to report completion or deliver results, using the send_message tool. Do NOT spam progress updates.\n\
            Current date: {date}",
            task_desc = task_desc,
            task_id = task_id,
            date = date,
        );

        let (system_prompt, model) = {
            let s = state.lock().await;
            let base = s.assemble_system_prompt(&cid, true, "");
            let model = s.effective_model(&cid);
            (base, model)
        };

        let tools = {
            let s = state.lock().await;
            s.build_tools(true)
        };
        let mut messages = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(&task_header),
        ];

        for _ in 0..10 {
            let request = ChatCompletionRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: Some(tools.clone()),
                tool_choice: None,
            };
            let response = {
                let s = state.lock().await;
                match s.llm.chat_completion(&request).await {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("Heartbeat LLM error: {}", e);
                        let msg = format!("⚠️ Task '{}' failed: LLM error — {}", task_id, e);
                        let _ = tg_bot
                            .send_message(
                                teloxide::types::ChatId(cid.parse().unwrap_or_default()),
                                &msg,
                            )
                            .await;
                        break;
                    }
                }
            };
            let choice = match response.choices.into_iter().next() {
                Some(c) => c,
                None => break,
            };
            if let Some(tcs) = &choice.message.tool_calls {
                if tcs.is_empty() {
                    break;
                }
                messages.push(ChatMessage::assistant_tool_calls(tcs.clone()));
                messages.extend(dispatch_tool_calls(&state, &cid, tcs, None, Some(&tg_bot)).await);
                let _ = git_repo.auto_commit("Heartbeat");
                continue;
            }
            break;
        }
        log::info!("Heartbeat chat {}: task '{}' done", cid, task_id);
    }

    if !tried_this_cycle.is_empty() {
        log::info!("Heartbeat chat {}: processed {} task(s) this cycle", cid, tried_this_cycle.len());
    }
}

/// Log a tool call to `tool_calls.log` in the given data directory.
fn log_tool_call_to(data_dir: &std::path::Path, tool_name: &str, args: &str, result: &str) {
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
async fn dispatch_tool_calls(
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
async fn dispatch_tool(
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
                s.conversation_history.get(&cid).cloned().unwrap_or_default()
            };
            let start = history.len().saturating_sub(count);
            let items: Vec<_> = history[start..].iter()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::MockLlmBackend;
    use crate::openrouter::{
        AssistantMessage, ChatCompletionResponse, Choice, FunctionCall, ToolCall,
    };
    use tempfile::TempDir;

    async fn setup_test_bot() -> (GlowBot, TempDir, Arc<MockLlmBackend>) {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let config = crate::config::basic_config();
        let config_path = data_dir.join("config.yaml");
        config.save(&config_path).unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm.clone())
            .await
            .unwrap();
        (bot, dir, mock_llm)
    }

    async fn setup_test_bot_with_whitelisted_chat() -> (GlowBot, TempDir, Arc<MockLlmBackend>) {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let mut config = crate::config::basic_config();
        config.chats.insert(
            "-123".into(),
            crate::config::ChatConfig {
                interaction_mode: crate::config::InteractionMode::EveryMessage,
                command_whitelist: vec!["456".into()],
                interaction_whitelist: vec!["456".into()],
                ..Default::default()
            },
        );
        let config_path = data_dir.join("config.yaml");
        config.save(&config_path).unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm.clone())
            .await
            .unwrap();
        (bot, dir, mock_llm)
    }

    #[tokio::test]
    async fn test_bot_creation() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        crate::config::basic_config().save(&data_dir.join("config.yaml")).unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
        let state = bot.state.lock().await;
        assert_eq!(state.config.telegram_token, "test-token");
    }

    #[tokio::test]
    async fn test_bot_creation_nonexistent_config() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let mock_llm = Arc::new(MockLlmBackend::new());
        let result = GlowBot::new_with_llm(&data_dir, mock_llm).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ensure_memory_exists() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        bot.ensure_memory_exists("-123", "456", "@testuser")
            .await
            .unwrap();

        let state = bot.state.lock().await;
        let mem = crate::memory::load_memory(&state.chats_dir(), "-123", "456");
        assert!(mem.is_some());
        assert_eq!(mem.unwrap().frontmatter.username, "@testuser");
    }

    #[tokio::test]
    async fn test_reload_skills() {
        let (bot, dir, _mock) = setup_test_bot().await;

        use crate::skills::{write_skill, SkillFrontmatter};
        let skills_dir = dir.path().join("glowbot_data").join("skills");
        let fm = SkillFrontmatter {
            name: "test-skill".into(),
            description: "A test".into(),
        };
        write_skill(&skills_dir, "test-skill", &fm, "body text").unwrap();

        bot.reload_skills().await.unwrap();
        let state = bot.state.lock().await;
        assert!(state.skills.contains_key("test-skill"));
    }

    #[tokio::test]
    async fn test_process_message_mention_only_ignores() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        // Default is MentionOnly, so non-mention messages should be ignored
        let result = bot
            .process_message("-123", "456", "@testuser", "Hello world", "mybot")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_process_message_mention_responds() {
        let (bot, _dir, mock) = setup_test_bot().await;

        // Set up mock to return a simple response
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("Hello, I'm GlowBot!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let result = bot
            .process_message("-123", "456", "@testuser", "@mybot Hello!", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("Hello, I'm GlowBot!".into()));
    }

    #[tokio::test]
    async fn test_process_message_every_message_mode() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("Got your message!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let result = bot
            .process_message("-123", "456", "@testuser", "Hello", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("Got your message!".into()));
    }

    #[tokio::test]
    async fn test_process_message_interaction_whitelist_blocks() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        // User "789" is not in interaction_whitelist
        let result = bot
            .process_message("-123", "789", "@other", "Hello", "mybot")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_process_message_command_unauthorized() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        // Default: command_whitelist is empty, so nobody can run commands
        let result = bot
            .process_message("-123", "456", "@testuser", "/status", "mybot")
            .await
            .unwrap();
        assert_eq!(
            result,
            Some("You are not authorized to run bot commands.".into())
        );
    }

    #[tokio::test]
    async fn test_process_message_command_authorized() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        // User "456" is in the command whitelist
        let result = bot
            .process_message("-123", "456", "@testuser", "/status", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("Chat ID:"));
    }

    #[tokio::test]
    async fn test_process_message_with_tool_call() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

        // First response: tool call
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_1".into(),
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: "bash".into(),
                            arguments: r#"{"command":"echo hello from bash"}"#.into(),
                        },
                    }]),
                    role: Some("assistant".into()),
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        // Second response: final text after tool result
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("The bash command succeeded!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let result = bot
            .process_message("-123", "456", "@testuser", "Run echo", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("The bash command succeeded!".into()));
    }

    #[tokio::test]
    async fn test_process_message_empty_choices() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

        mock.add_response(ChatCompletionResponse { choices: vec![] });

        let result = bot
            .process_message("-123", "456", "@testuser", "Hello", "mybot")
            .await
            .unwrap();
        // Empty choices falls through to the loop error message
        assert!(result.unwrap().contains("loop"));
    }

    #[tokio::test]
    async fn test_process_message_empty_tool_calls() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("Final answer".into()),
                    tool_calls: Some(vec![]),
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let result = bot
            .process_message("-123", "456", "@testuser", "Hello", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("Final answer".into()));
    }

    #[tokio::test]
    async fn test_process_message_loop_limit() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

        // Continuously return tool calls to trigger the loop limit
        for _ in 0..10 {
            mock.add_response(ChatCompletionResponse {
                choices: vec![Choice {
                    message: AssistantMessage {
                        content: None,
                        tool_calls: Some(vec![ToolCall {
                            id: "call_x".into(),
                            call_type: "function".into(),
                            function: FunctionCall {
                                name: "bash".into(),
                                arguments: r#"{"command":"echo loop"}"#.into(),
                            },
                        }]),
                        role: Some("assistant".into()),
                    },
                    finish_reason: Some("tool_calls".into()),
                }],
            });
        }

        let result = bot
            .process_message("-123", "456", "@testuser", "Loop test", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("loop"));
    }

    #[tokio::test]
    async fn test_process_message_command_model() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        let result = bot
            .process_message("-123", "456", "@testuser", "/model custom/model", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("custom/model"));
    }

    #[tokio::test]
    async fn test_process_message_command_mode() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        let result = bot
            .process_message("-123", "456", "@testuser", "/mode every_message", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("EveryMessage"));
    }

    #[tokio::test]
    async fn test_process_message_command_reload() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        let result = bot
            .process_message("-123", "456", "@testuser", "/reload", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("Skills reloaded successfully.".into()));
    }

    #[tokio::test]
    async fn test_process_message_command_invalid_mode() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        let result = bot
            .process_message("-123", "456", "@testuser", "/mode invalid", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("Unknown mode"));
    }

    #[tokio::test]
    async fn test_save_config() {
        let (bot, _dir, _mock) = setup_test_bot().await;
        // Saving should work without errors
        bot.save_config().await.unwrap();
    }

    #[tokio::test]
    async fn test_process_message_bash_tool_error() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_err".into(),
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: "bash".into(),
                            arguments: r#"{"command":"nonexistent_command_xyz"}"#.into(),
                        },
                    }]),
                    role: Some("assistant".into()),
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("Command failed, but I handled it.".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let result = bot
            .process_message("-123", "456", "@testuser", "Run bad command", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("Command failed, but I handled it.".into()));
    }

    #[tokio::test]
    async fn test_process_message_with_chat_system_prompt() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let mut config = crate::config::basic_config();
        config.chats.insert(
            "-123".into(),
            crate::config::ChatConfig {
                interaction_mode: crate::config::InteractionMode::EveryMessage,
                interaction_whitelist: vec![],
                command_whitelist: vec![],
                system_prompt: "Custom system prompt".into(),
                ..Default::default()
            },
        );
        config.save(&data_dir.join("config.yaml")).unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        mock_llm.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("Response with custom prompt".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
        let result = bot
            .process_message("-123", "456", "@testuser", "Hello", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("Response with custom prompt".into()));
    }

    #[tokio::test]
    async fn test_new_with_llm_with_skills_dir_with_empty_subdirs() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        crate::config::basic_config().save(&data_dir.join("config.yaml")).unwrap();

        // Create a skills dir with a subdirectory that has no skill.md
        let skills_dir = data_dir.join("skills");
        std::fs::create_dir_all(skills_dir.join("empty_skill")).unwrap();
        // Also create a file directly (not a directory)
        std::fs::write(skills_dir.join("some_file.txt"), "not a skill").unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
        let state = bot.state.lock().await;
        assert!(state.skills.is_empty());
    }

    #[tokio::test]
    async fn test_dm_always_responds_even_in_mention_only_mode() {
        // DMs have positive chat IDs (not starting with '-')
        // They should always respond, even with mention_only default
        let (bot, _dir, mock) = setup_test_bot().await;

        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("DM response!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        // Positive chat ID = DM, default mention_only mode
        let result = bot
            .process_message("123456789", "456", "@testuser", "Hello in DM", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("DM response!".into()));
    }

    #[tokio::test]
    async fn test_process_message_with_read_memory_tool() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

        // First ensure memory exists
        bot.ensure_memory_exists("-123", "456", "@testuser")
            .await
            .unwrap();

        // LLM calls read_memory
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_read".into(),
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: "read_memory".into(),
                            arguments: r#"{"user_id":"456"}"#.into(),
                        },
                    }]),
                    role: Some("assistant".into()),
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        // Then LLM responds after reading memory
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("I remember you, @testuser!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let result = bot
            .process_message("-123", "456", "@testuser", "Who am I?", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("I remember you, @testuser!".into()));
    }

    #[tokio::test]
    async fn test_process_message_with_update_memory_tool() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

        // LLM calls update_memory
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_update".into(),
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: "update_memory".into(),
                            arguments: r#"{"user_id":"456","call_name":"Learned","log_entry":"user said hello"}"#.into(),
                        },
                    }]),
                    role: Some("assistant".into()),
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        // LLM confirms
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("I've noted that!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let result = bot
            .process_message("-123", "456", "@testuser", "My name is Learned", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("I've noted that!".into()));

        // Verify memory was actually updated
        let state = bot.state.lock().await;
        let mem = crate::memory::load_memory(&state.chats_dir(), "-123", "456").unwrap();
        assert_eq!(mem.frontmatter.call_name, "Learned");
        assert!(mem.body.contains("user said hello"));
    }

    #[tokio::test]
    async fn test_log_tool_call() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();

        log_tool_call_to(&data_dir, "bash", r#"{"command":"echo hi"}"#, "stdout: hi\n");

        let log_path = data_dir.join("tool_calls.log");
        assert!(log_path.exists());
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("bash"));
        assert!(content.contains("echo hi"));
    }

    #[tokio::test]
    async fn test_dm_tools_disabled_by_default() {
        let (bot, _dir, mock) = setup_test_bot().await;

        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("Text-only response".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        // DM (positive chat ID), default empty whitelist = tools disabled
        let result = bot
            .process_message("123", "456", "@test", "Hello", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("Text-only response".into()));
    }

    #[tokio::test]
    async fn test_dm_blocked_when_whitelist_nonempty_and_user_not_in_it() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let mut config = crate::config::basic_config();
        config.dm_whitelist = vec!["999".into()]; // only user 999
        config.save(&data_dir.join("config.yaml")).unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();

        // User 456 is NOT in whitelist = blocked
        let result = bot
            .process_message("123", "456", "@test", "Hello", "mybot")
            .await
            .unwrap();
        assert!(result.unwrap().contains("not authorized"));
    }

    #[tokio::test]
    async fn test_dm_allowed_when_whitelisted() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let mut config = crate::config::basic_config();
        config.dm_whitelist = vec!["456".into()];
        config.save(&data_dir.join("config.yaml")).unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        mock_llm.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("Full access!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
        let result = bot
            .process_message("123", "456", "@test", "Hello", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("Full access!".into()));
    }

    #[tokio::test]
    async fn test_heartbeat_disabled_when_zero() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let mut config = crate::config::basic_config();
        config.chats.insert(
            "-123".into(),
            crate::config::ChatConfig {
                heartbeat_interval_minutes: Some(0),
                ..Default::default()
            },
        );
        config.save(&data_dir.join("config.yaml")).unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
        let state = bot.state.lock().await;
        assert_eq!(state.config.heartbeat_interval("-123"), None);
    }

    #[tokio::test]
    async fn test_heartbeat_has_pending_tasks() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        crate::config::basic_config()
            .save(&data_dir.join("config.yaml"))
            .unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();

        let state = bot.state.lock().await;
        assert!(!state.has_pending_tasks("-123"));

        let mut list = crate::tasks::TaskList::default();
        list.add("test task");
        list.save(&state.chats_dir(), "-123").unwrap();

        assert!(state.has_pending_tasks("-123"));
    }

    #[tokio::test]
    async fn test_build_tools_includes_mcp() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        crate::config::basic_config()
            .save(&data_dir.join("config.yaml"))
            .unwrap();

        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
        let mut state = bot.state.lock().await;

        // No MCP tools yet — normal conversation set (send_message excluded)
        let tools = state.build_tools(false);
        assert_eq!(tools.len(), 12);

        // Add a fake MCP tool
        state.mcp_tools.push(crate::mcp::McpTool {
            server_name: "test-srv".into(),
            name: "test_tool".into(),
            description: "A test".into(),
            input_schema: serde_json::json!({"type": "object"}),
            server_url: "https://example.com".into(),
            api_key: None,
            session_id: None,
            transport: "streamable".into(),
        });

        let tools = state.build_tools(false);
        assert_eq!(tools.len(), 13);
        assert!(tools.iter().any(|t| t.function.name == "mcp_test-srv_test_tool"));

        // Heartbeat set includes send_message
        let hb_tools = state.build_tools(true);
        assert_eq!(hb_tools.len(), 14);
        assert!(hb_tools.iter().any(|t| t.function.name == "send_message"));
    }

    #[tokio::test]
    async fn test_get_recent_messages_tool() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;

        // Pre-seed some conversation history
        {
            let mut state = bot.state.lock().await;
            let history = state
                .conversation_history
                .entry("-123".to_string())
                .or_default();
            history.push(ChatMessage::user_with_name("Hello bot", "Alice"));
            history.push(ChatMessage::assistant("Hi Alice!"));
            history.push(ChatMessage::user_with_name("What's my name?", "Alice"));
            history.push(ChatMessage::assistant("Your name is Alice."));
        }

        // LLM calls get_recent_messages(count: 2)
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_recent".into(),
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: "get_recent_messages".into(),
                            arguments: r#"{"count":2}"#.into(),
                        },
                    }]),
                    role: Some("assistant".into()),
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        // After reading context, LLM responds
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("I recall our conversation!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let result = bot
            .process_message("-123", "456", "@alice", "Recall what I said", "mybot")
            .await
            .unwrap();
        assert_eq!(result, Some("I recall our conversation!".into()));
    }

    // ---------- dispatch_tool edge-case tests ----------

    #[tokio::test]
    async fn test_dispatch_send_message_empty_text() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "send_message", &serde_json::json!({"text":""}), None).await;
        assert_eq!(out, "Error: text required");
    }

    #[tokio::test]
    async fn test_dispatch_send_message_no_tg_bot() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "send_message", &serde_json::json!({"text":"hi"}), None).await;
        assert_eq!(out, "Error: send_message not available in this context.");
    }

    #[tokio::test]
    async fn test_dispatch_bash_empty_command() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "bash", &serde_json::json!({"command":""}), None).await;
        assert!(out.contains("exit code"));
    }

    #[tokio::test]
    async fn test_dispatch_read_memory_missing() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "read_memory", &serde_json::json!({"user_id":"999"}), None).await;
        assert!(out.contains("No memory file found"));
    }

    #[tokio::test]
    async fn test_dispatch_update_memory_no_fields() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "update_memory", &serde_json::json!({"user_id":"999"}), None).await;
        assert_eq!(out, "No fields to update.");
    }

    #[tokio::test]
    async fn test_dispatch_add_task_empty() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "add_task", &serde_json::json!({"description":""}), None).await;
        assert_eq!(out, "Error: description required");
    }

    #[tokio::test]
    async fn test_dispatch_list_tasks_non_empty() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir: data_dir.clone(),
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        // Add a task first
        dispatch_tool(&state, "-123", "add_task", &serde_json::json!({"description":"do the thing"}), None).await;
        let out = dispatch_tool(&state, "-123", "list_tasks", &serde_json::json!({}), None).await;
        assert!(out.contains("do the thing"));
    }

    #[tokio::test]
    async fn test_dispatch_remove_task_empty_and_not_found() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir: data_dir.clone(),
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "remove_task", &serde_json::json!({"id":""}), None).await;
        assert_eq!(out, "Error: id required");
        let out = dispatch_tool(&state, "-123", "remove_task", &serde_json::json!({"id":"nope"}), None).await;
        assert!(out.contains("not found"));
    }

    #[tokio::test]
    async fn test_dispatch_create_skill_validation() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir: data_dir.clone(),
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "create_skill", &serde_json::json!({"name":"","description":"","body":""}), None).await;
        assert!(out.contains("required"));
    }

    #[tokio::test]
    async fn test_dispatch_read_skill_not_found() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "read_skill", &serde_json::json!({"name":"ghost"}), None).await;
        assert!(out.contains("not found"));
    }

    #[tokio::test]
    async fn test_dispatch_update_skill_not_found() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "update_skill", &serde_json::json!({"name":"missing","description":"d","body":"b"}), None).await;
        assert!(out.contains("not found"));
    }

    #[tokio::test]
    async fn test_dispatch_unknown_tool() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "narnia", &serde_json::json!({}), None).await;
        assert!(out.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_dispatch_mcp_tool_not_found() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "mcp_no_no", &serde_json::json!({}), None).await;
        assert!(out.contains("MCP tool not found"));
    }

    #[tokio::test]
    async fn test_get_recent_messages_empty_history() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let cfg = crate::config::basic_config();
        cfg.save(&data_dir.join("config.yaml")).unwrap();
        let state = Arc::new(Mutex::new(BotState {
            config: cfg,
            skills: HashMap::new(),
            llm: Arc::new(MockLlmBackend::new()),
            data_dir,
            conversation_history: HashMap::new(),
            mcp_tools: vec![],
        }));
        let out = dispatch_tool(&state, "-123", "get_recent_messages", &serde_json::json!({"count":5}), None).await;
        assert!(out.contains("messages"));
    }

    #[tokio::test]
    async fn test_process_message_plain_command_ignored() {
        let (bot, _dir, _mock) = setup_test_bot_with_whitelisted_chat().await;
        // "/notabotcommand" -> starts with / but isn't a recognised command -> ignored
        let result = bot
            .process_message("-123", "456", "@testuser", "/notabotcommand", "mybot")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_conversation_history_window_trims() {
        let (bot, _dir, mock) = setup_test_bot_with_whitelisted_chat().await;
        // Seed history with exactly 20 items (default conversation_window=20)
        {
            let mut state = bot.state.lock().await;
            let h = state.conversation_history.entry("-123".into()).or_default();
            for i in 0..20 {
                h.push(ChatMessage::user(&format!("msg{i}")));
                h.push(ChatMessage::assistant(&format!("reply{i}")));
            }
        }
        // one more exchange will trigger trimming
        mock.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("ok".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });
        let _ = bot.process_message("-123", "456", "user", "hello", "mybot").await.unwrap();
        let h_len = {
            let state = bot.state.lock().await;
            state.conversation_history.get("-123").unwrap().len()
        };
        assert_eq!(h_len, 20);
    }

    // ---------- heartbeat tests ----------

    #[tokio::test]
    async fn test_heartbeat_no_tasks() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        crate::config::basic_config().save(&data_dir.join("config.yaml")).unwrap();
        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm).await.unwrap();
        let tg_bot = teloxide::Bot::new("ignored");
        run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), "-123", tg_bot).await;
    }

    #[tokio::test]
    async fn test_heartbeat_completes_task() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        crate::config::basic_config().save(&data_dir.join("config.yaml")).unwrap();
        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm.clone()).await.unwrap();

        // add a pending task
        let mut list = crate::tasks::TaskList::default();
        let id = list.add("heartbeat task");
        list.save(&data_dir.join("chats"), "-123").unwrap();

        // LLM returns a remove_task call to complete it
        mock_llm.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_rm".into(),
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: "remove_task".into(),
                            arguments: format!(r##"{{"id":"{}"}}"##, id),
                        },
                    }]),
                    role: Some("assistant".into()),
                },
                finish_reason: Some("tool_calls".into()),
            }],
        });

        mock_llm.add_response(ChatCompletionResponse {
            choices: vec![Choice {
                message: AssistantMessage {
                    content: Some("Done!".into()),
                    tool_calls: None,
                    role: Some("assistant".into()),
                },
                finish_reason: Some("stop".into()),
            }],
        });

        let tg_bot = teloxide::Bot::new("ignored");
        run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), "-123", tg_bot).await;

        let list = crate::tasks::TaskList::load(&data_dir.join("chats"), "-123").unwrap_or_default();
        assert!(list.tasks.is_empty());
    }

    #[tokio::test]
    async fn test_heartbeat_llm_error() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("glowbot_data");
        std::fs::create_dir_all(&data_dir).unwrap();
        crate::config::basic_config().save(&data_dir.join("config.yaml")).unwrap();
        let mock_llm = Arc::new(MockLlmBackend::new());
        let bot = GlowBot::new_with_llm(&data_dir, mock_llm.clone()).await.unwrap();

        let mut list = crate::tasks::TaskList::default();
        list.add("error task");
        list.save(&data_dir.join("chats"), "-123").unwrap();

        // configure mock to error
        mock_llm.set_error(true);

        let tg_bot = teloxide::Bot::new("ignored");
        run_heartbeat_task(bot.state.clone(), bot.git_repo.clone(), "-123", tg_bot).await;

        // task should still be there after error
        let list = crate::tasks::TaskList::load(&data_dir.join("chats"), "-123").unwrap_or_default();
        assert_eq!(list.tasks.len(), 1);
    }

}
