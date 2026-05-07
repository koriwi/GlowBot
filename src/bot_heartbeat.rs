use super::bot_dispatch::dispatch_tool_calls;
use super::BotState;
use crate::openrouter::{ChatCompletionRequest, ChatMessage};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Run a heartbeat background task for a chat. Uses the state directly.
/// Each task is processed at most once per cycle. We iterate through
/// all tasks, skipping any already handled this cycle. The loop exits
/// when every remaining task has been tried.
pub async fn run_heartbeat_task(
    state: Arc<Mutex<BotState>>,
    _git_repo: crate::git::GitRepo,
    chat_id: &str,
    tg_bot: teloxide::Bot,
) {
    let cid = chat_id.to_string();
    let mut tried_this_cycle: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        let (task_id, task_desc) = {
            let s = state.lock().await;
            let list = crate::tasks::TaskList::load(&s.chats_dir(), &cid).unwrap_or_default();
            match list
                .tasks
                .iter()
                .find(|t| !tried_this_cycle.contains(&t.id))
            {
                Some(t) => (t.id.clone(), t.description.clone()),
                None => break,
            }
        };

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
              just leave it — do NOT remove it, do NOT add a new identical one. It will automatically run again next cycle.\n\
            - You may send at most ONE message to the chat to report completion or deliver results, using the send_message tool. Do NOT spam progress updates.\n\
            - If the task has already been completed (e.g. file already downloaded, action already performed, nothing left to do),\n\
              quietly call remove_task(\"{task_id}\") and exit — do NOT send any message.\n\
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
            let bash_enabled = s.config.is_bash_enabled(&cid);
            s.build_tools(bash_enabled, &cid)
        };

        let context_limit = {
            let s = state.lock().await;
            s.model_context_lengths.get(crate::openrouter::normalize_model_id(&model)).copied().unwrap_or(0)
        };

        let mut messages = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(&task_header),
        ];

        for _ in 0..10 {
            // Trim heartbeat messages if accumulated tool rounds are getting too long
            let request_messages = if context_limit > 0 {
                let tools_tokens = crate::openrouter::estimate_tools_tokens(&tools);
                let head_tokens = crate::openrouter::estimate_messages_tokens(&messages[..2]);
                let tail_tokens = crate::openrouter::estimate_messages_tokens(&messages[2..]);
                let fixed = head_tokens
                    .saturating_add(tail_tokens)
                    .saturating_add(tools_tokens)
                    .saturating_add(crate::openrouter::RESPONSE_RESERVE_TOKENS);
                let effective_limit =
                    (context_limit as f64 * crate::openrouter::TOKEN_ESTIMATE_MARGIN) as u64;
                if fixed > effective_limit && messages.len() > 4 {
                    log::info!(
                        "Heartbeat chat {}: trimming {} old tool rounds to fit context",
                        cid,
                        messages.len() - 4
                    );
                    crate::openrouter::trim_message_list(&messages, 2, 2)
                } else {
                    messages.clone()
                }
            } else {
                messages.clone()
            };

            let request = ChatCompletionRequest {
                model: model.clone(),
                messages: request_messages,
                tools: Some(tools.clone()),
                tool_choice: None,
            };
            let (response, usage) = {
                let s = state.lock().await;
                match s.llm.chat_completion(&request).await {
                    Ok(r) => {
                        let usage = r.usage.clone().unwrap_or_default();
                        (r, usage)
                    }
                    Err(e) => {
                        log::error!("Heartbeat LLM error: {}", e);
                        let msg = format!("⚠️ Task '{}' failed: LLM error — {}", task_id, e);
                        crate::bot_send::send_message(
                            &tg_bot,
                            teloxide::types::ChatId(cid.parse().unwrap_or_default()),
                            &msg,
                        )
                        .await;
                        break;
                    }
                }
            };
            {
                let mut s = state.lock().await;
                s.last_usage.insert(cid.clone(), usage);
            }
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
                continue;
            }
            break;
        }
        log::info!("Heartbeat chat {}: task '{}' done", cid, task_id);
    }

    if !tried_this_cycle.is_empty() {
        log::info!(
            "Heartbeat chat {}: processed {} task(s) this cycle",
            cid,
            tried_this_cycle.len()
        );
    }
}
