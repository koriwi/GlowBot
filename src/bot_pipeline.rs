use super::BotState;
use crate::db::Database;
use crate::git::GitRepo;
use crate::memory::{save_memory, Memory};
use crate::openrouter::{ChatCompletionRequest, ChatMessage, OpenRouterClient};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Process a message through the LLM pipeline (free function).
pub(crate) async fn process_with_llm_impl(
    state: &Arc<Mutex<BotState>>,
    _git_repo: &GitRepo,
    stop_signals: &Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    chat_id: &str,
    user_id: &str,
    username: &str,
    text: &str,
    tools_enabled: bool,
    tg_bot: Option<&teloxide::Bot>,
) -> anyhow::Result<Option<String>> {
    // Set up stop signal for this chat (clear any previous signal)
    {
        let mut signals = stop_signals.lock().unwrap();
        signals
            .entry(chat_id.to_string())
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    let check_stopped = || -> bool {
        if let Ok(signals) = stop_signals.lock() {
            signals
                .get(chat_id)
                .map(|s| s.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(false)
        } else {
            false
        }
    };

    let (system_prompt, model) = {
        let s = state.lock().await;
        (
            s.assemble_system_prompt(chat_id, tools_enabled, user_id),
            s.effective_model(chat_id),
        )
    };

    // Ensure user has a memory file
    ensure_memory_exists_impl(state, chat_id, user_id, username).await?;

    // Read existing conversation history upfront
    let (history, include_reasoning) = {
        let s = state.lock().await;
        let win = s.config.conversation.recent_messages_window_size;
        let include = s.config.conversation.include_reasoning;
        let hist = match s.db.load_messages(chat_id, win) {
            Ok(msgs) => msgs,
            Err(e) => {
                log::error!(
                    "Failed to load conversation history for chat {}: {}",
                    chat_id,
                    e
                );
                Vec::new()
            }
        };
        (hist, include)
    };

    let current_msg = ChatMessage::user_with_name(text, username);
    let mut turn_messages = vec![current_msg.clone()];

    let tools: Vec<crate::openrouter::ToolDefinition> = if tools_enabled {
        let s = state.lock().await;
        let bash_enabled = s.config.is_bash_enabled(chat_id);
        s.build_tools(bash_enabled)
    } else {
        vec![]
    };

    let context_limit = {
        let s = state.lock().await;
        s.model_context_lengths.get(&model).copied().unwrap_or(0)
    };

    let max_tool_rounds = 64;

    let (result, final_reasoning) = {
        let mut final_text = None;
        let mut final_reasoning = None;
        for _round in 0..max_tool_rounds {
            if check_stopped() {
                return Ok(Some("⏹ Stopped.".into()));
            }

            let (messages, _trimmed) = crate::openrouter::build_trimmed_request(
                context_limit,
                &[ChatMessage::system(&system_prompt)],
                &history,
                &turn_messages,
                &tools,
            );

            let request = ChatCompletionRequest {
                model: model.clone(),
                messages,
                tools: Some(tools.clone()),
                tool_choice: None,
            };

            let (response, usage) = {
                let s = state.lock().await;
                let resp = s.llm.chat_completion(&request).await?;
                let usage = resp.usage.clone().unwrap_or_default();
                (resp, usage)
            };
            {
                let mut s = state.lock().await;
                s.last_usage.insert(chat_id.to_string(), usage);
            }

            if check_stopped() {
                return Ok(Some("⏹ Stopped.".into()));
            }

            let choice = match response.choices.into_iter().next() {
                Some(c) => c,
                None => break,
            };

            if let Some(tool_calls) = &choice.message.tool_calls {
                if tool_calls.is_empty() {
                    final_text = Some(choice.message.content.clone().unwrap_or_default());
                    break;
                }

                // Record assistant's tool call message in the turn
                if let (Some(reasoning), true) = (&choice.message.reasoning, include_reasoning) {
                    turn_messages.push(ChatMessage::assistant_tool_calls_with_reasoning(
                        tool_calls.clone(),
                        reasoning.clone(),
                    ));
                } else {
                    turn_messages.push(ChatMessage::assistant_tool_calls(tool_calls.clone()));
                }

                let data_dir = { state.lock().await.data_dir.clone() };
                let results = super::bot_dispatch::dispatch_tool_calls(
                    state,
                    chat_id,
                    tool_calls,
                    Some(&data_dir),
                    tg_bot,
                )
                .await;
                turn_messages.extend(results);

                if check_stopped() {
                    return Ok(Some("⏹ Stopped.".into()));
                }
                continue;
            }

            final_text = Some(choice.message.content.clone().unwrap_or_default());
            final_reasoning = choice.message.reasoning;
            break;
        }

        (
            final_text.unwrap_or_else(|| {
                "I ran into a loop processing your request. Please try again.".into()
            }),
            final_reasoning,
        )
    };

    // Record final assistant message in the turn
    if let (Some(reasoning), true) = (&final_reasoning, include_reasoning) {
        turn_messages.push(ChatMessage::assistant_with_reasoning(
            &result,
            reasoning.clone(),
        ));
    } else {
        turn_messages.push(ChatMessage::assistant(&result));
    }

    // Store the completed turn in conversation history
    let message_ids = {
        let s = state.lock().await;
        s.db.save_messages(chat_id, &turn_messages)
            .unwrap_or_default()
    };

    // Embed messages in the background if embedding model is configured
    {
        let s = state.lock().await;
        if let Some(ref embed_model) = s.config.embedding.model {
            if !message_ids.is_empty() {
                let api_key = s.config.openrouter.api_key.clone();
                let db = s.db.clone();
                let embed_model = embed_model.clone();
                let max_chars = s.config.embedding.max_chars;
                let allow_split = s.config.embedding.allow_split;
                let turn_messages = turn_messages.clone();
                drop(s);

                tokio::spawn(async move {
                    embed_turn(
                        &db,
                        &api_key,
                        &embed_model,
                        max_chars,
                        allow_split,
                        &message_ids,
                        &turn_messages,
                    )
                    .await;
                });
            }
        }
    }

    Ok(Some(result))
}

/// Embed each message in a turn and store the vectors.
/// Runs as a background task — failures are logged but don't affect the user.
async fn embed_turn(
    db: &Database,
    api_key: &str,
    embed_model: &str,
    max_chars: usize,
    allow_split: bool,
    message_ids: &[i64],
    turn_messages: &[ChatMessage],
) {
    let client = OpenRouterClient::new(api_key.to_string());
    for (i, msg) in turn_messages.iter().enumerate() {
        if i >= message_ids.len() {
            break;
        }
        let text = msg.text_content();
        if text.is_empty() {
            continue;
        }
        let chunks = chunk_for_embedding(&text, max_chars, allow_split);
        for chunk in &chunks {
            let text_preview: String = chunk.chars().take(80).collect();
            match client.embeddings(embed_model, chunk).await {
                Ok(vec) => {
                    if let Err(e) = db.save_embedding(message_ids[i], &vec, embed_model) {
                        log::warn!(
                            "Failed to save embedding for message {} (model={}, text=\"{}\"): {}",
                            message_ids[i],
                            embed_model,
                            text_preview,
                            e
                        );
                    }
                }
                Err(e) => {
                    log::warn!(
                        "Failed to embed message {} (model={}, text=\"{}\"): {}",
                        message_ids[i],
                        embed_model,
                        text_preview,
                        e
                    );
                }
            }
        }
    }
}

/// Split text into chunks for embedding based on max_chars and allow_split.
/// Returns a Vec of strings — always at least one element.
pub(crate) fn chunk_for_embedding(text: &str, max_chars: usize, allow_split: bool) -> Vec<String> {
    if max_chars == 0 {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return vec![text.to_string()];
    }
    if allow_split {
        chars
            .chunks(max_chars)
            .map(|c| c.iter().collect())
            .collect()
    } else {
        vec![chars[..max_chars].iter().collect()]
    }
}

pub(crate) async fn ensure_memory_exists_impl(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    user_id: &str,
    username: &str,
) -> anyhow::Result<()> {
    let s = state.lock().await;
    let existing = crate::memory::load_memory(&s.chats_dir(), chat_id, user_id);
    if existing.is_none() {
        let mem = Memory::new(user_id, username);
        save_memory(&s.chats_dir(), chat_id, user_id, &mem)?;
    }
    Ok(())
}
