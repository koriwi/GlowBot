#[path = "bot_embeddings.rs"]
mod bot_embeddings;
pub(crate) use bot_embeddings::chunk_for_embedding;
use bot_embeddings::embed_turn;
#[path = "bot_media.rs"]
mod bot_media;
use super::BotState;
use crate::git::GitRepo;
use crate::memory::{save_memory, Memory};
use crate::openrouter::{ChatCompletionRequest, ChatMessage};
use bot_media::build_user_message_full;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

/// RAII guard that stops the typing indicator refresher on drop.
struct TypingGuard {
    flag: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.flag.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Process a message through the LLM pipeline (free function).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_with_llm_impl(
    state: &Arc<Mutex<BotState>>,
    _git_repo: &GitRepo,
    stop_signals: &Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    chat_id: &str,
    user_id: &str,
    username: &str,
    text: &str,
    caption: Option<&str>,
    media: Option<&crate::media::IngestedMedia>,
    sender_name: Option<&str>,
    sent_at: Option<chrono::DateTime<chrono::Utc>>,
    tools_enabled: bool,
    tg_bot: Option<&teloxide::Bot>,
    interaction: Option<(bool, &str)>,
) -> anyhow::Result<Option<String>> {
    log::info!(
        "pipeline: starting LLM processing for chat={}, user={}, text=\"{}\", has_media={}",
        chat_id,
        user_id,
        text.chars().take(100).collect::<String>(),
        media.is_some()
    );

    // Set up stop signal for this chat (clear any previous signal)
    {
        let mut signals = stop_signals.lock().unwrap_or_else(|e| e.into_inner());
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

    let (system_prompt, model, provider) = {
        let s = state.lock().await;
        (
            s.assemble_system_prompt(chat_id, tools_enabled, user_id),
            s.effective_model(chat_id),
            s.effective_provider(chat_id),
        )
    };

    let auto_detect = chat_id.starts_with('-') && {
        state
            .lock()
            .await
            .config
            .chat_config(chat_id)
            .interaction_mode
            == crate::config::InteractionMode::AutoDetect
    };
    let needs_decision = auto_detect && !interaction.is_some_and(|(mention, _)| mention);
    let decider = if needs_decision {
        match super::bot_decider::model_for_decision(state).await {
            Ok(model) => Some(model),
            Err(e) => {
                log::error!("decider: unavailable for chat {}: {}", chat_id, e);
                None
            }
        }
    } else {
        None
    };

    // Ensure user has a memory file
    ensure_memory_exists_impl(state, chat_id, user_id, username).await?;

    // Read existing conversation history upfront
    let mut history = {
        let s = state.lock().await;
        let win = s.config.conversation.recent_messages_window_size;
        let cutoff = s.db.get_cutoff(chat_id).unwrap_or(None);
        let hist = match s.db.load_messages(chat_id, win, cutoff) {
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
        // Strip orphaned tool results that can occur when the sliding
        // window drops an assistant_tool_calls message but keeps its
        // subsequent tool_result messages.
        crate::openrouter::strip_orphaned_tool_results(&hist)
    };

    let (current_msg, media_complete) = if needs_decision && decider.is_none() {
        let body = format!(
            "User: {} (ID: {})\n{}\n{}\n{}",
            username,
            user_id,
            text,
            caption.unwrap_or(""),
            if media.is_some() {
                "[Media unavailable for decision]"
            } else {
                ""
            }
        );
        (ChatMessage::user_with_name(&body, username), false)
    } else {
        build_user_message_full(
            state,
            chat_id,
            user_id,
            username,
            sender_name,
            sent_at,
            text,
            caption,
            media,
            tg_bot,
            decider.as_deref(),
        )
        .await
    };
    if auto_detect {
        let ids = state
            .lock()
            .await
            .db
            .save_messages(chat_id, std::slice::from_ref(&current_msg))?;
        bot_embeddings::embed_saved(state, ids, vec![current_msg.clone()]).await;
    }
    if needs_decision {
        if !media_complete {
            log::error!(
                "decider: media preparation incomplete for chat {}; staying silent",
                chat_id
            );
            return Ok(None);
        }
        if check_stopped() {
            return Ok(None);
        }
        let Some(decider_model) = decider else {
            return Ok(None);
        };
        let bot_username = interaction.map(|(_, name)| name).unwrap_or("bot");
        match super::bot_decider::judge(
            state,
            chat_id,
            &decider_model,
            bot_username,
            &history,
            &current_msg,
        )
        .await
        {
            Ok(true) => {
                if check_stopped() {
                    return Ok(None);
                }
            }
            Ok(false) => return Ok(None),
            Err(e) => {
                log::error!("decider: failed for chat {}: {}", chat_id, e);
                return Ok(None);
            }
        }
    }
    let current_msg = if auto_detect {
        for message in &mut history {
            *message = super::bot_decider::for_agent(state, chat_id, message.clone()).await;
        }
        super::bot_decider::for_agent(state, chat_id, current_msg).await
    } else {
        current_msg
    };
    // Start a background typing indicator refresher that sends ChatAction::Typing
    // every 4 seconds so long-running LLM sessions don't look frozen.
    let _typing_guard = tg_bot.map(|bot| {
        let bot = bot.clone();
        let keep_running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let keep_clone = Arc::clone(&keep_running);
        if let Ok(parsed) = chat_id.parse::<i64>() {
            let cid = teloxide::types::ChatId(parsed);
            tokio::spawn(async move {
                while keep_clone.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = bot
                        .send_chat_action(cid, teloxide::types::ChatAction::Typing)
                        .await;
                    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                }
            });
        }
        TypingGuard { flag: keep_running }
    });

    let mut turn_messages = vec![current_msg.clone()];

    let tools: Vec<crate::openrouter::ToolDefinition> = if tools_enabled {
        let s = state.lock().await;
        let bash_enabled = s.config.is_bash_enabled(chat_id);
        s.build_tools(bash_enabled, chat_id)
    } else {
        vec![]
    };

    let context_limit = {
        let s = state.lock().await;
        s.model_metadata
            .get(crate::openrouter::normalize_model_id(&model))
            .map(|m| m.context_length)
            .unwrap_or(0)
    };

    let max_tool_rounds = 64;

    let (result, final_reasoning) = {
        let mut final_text = None;
        let mut final_reasoning = None;
        for round in 0..max_tool_rounds {
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
                modalities: None,
                image_config: None,
            };
            let msg_count = request.messages.len();

            let (response, _usage) = {
                let llm = { state.lock().await.llm.clone() };
                log::info!(
                    "pipeline: calling LLM model={}, round={}, messages={}",
                    model,
                    round,
                    msg_count
                );
                let resp = llm.chat_completion_for_provider(provider, &request).await?;
                let usage = resp.usage.clone().unwrap_or_default();
                log::info!(
                    "pipeline: LLM response received, prompt_tokens={}, completion_tokens={}, has_tool_calls={}",
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    resp.choices.first().and_then(|c| c.message.tool_calls.as_ref()).map(|t| t.len()).unwrap_or(0)
                );
                let mut s = state.lock().await;
                s.last_usage.insert(chat_id.to_string(), usage.clone());
                (resp, usage)
            };

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
                let assistant_message = if let Some(reasoning) = &choice.message.reasoning {
                    ChatMessage::assistant_tool_calls_with_reasoning(
                        tool_calls.clone(),
                        reasoning.clone(),
                    )
                } else {
                    ChatMessage::assistant_tool_calls(tool_calls.clone())
                }
                .with_provider_data(choice.message.provider_data.clone());
                turn_messages.push(assistant_message);

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
    if let Some(reasoning) = &final_reasoning {
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
        log::info!(
            "pipeline: saving turn to DB ({} messages)",
            turn_messages.len()
        );
        s.db.save_messages(
            chat_id,
            if auto_detect {
                &turn_messages[1..]
            } else {
                &turn_messages
            },
        )
        .unwrap_or_default()
    };
    log::info!(
        "pipeline: stored {} messages in DB for chat {}",
        message_ids.len(),
        chat_id
    );

    // Embed messages in the background if embedding model is configured
    {
        let s = state.lock().await;
        if let Some(ref embed_model) = s.config.openrouter.embedding_model {
            if !message_ids.is_empty() {
                let api_key = s.config.openrouter.api_key.clone();
                let db = s.db.clone();
                let embed_model = embed_model.clone();
                let max_chars = s.config.embedding.max_chars;
                let allow_split = s.config.embedding.allow_split;
                let turn_messages = if auto_detect {
                    turn_messages[1..].to_vec()
                } else {
                    turn_messages.clone()
                };
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

    log::info!(
        "pipeline: done, returning response (len={}) for chat={}",
        result.len(),
        chat_id
    );
    Ok(Some(result))
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
