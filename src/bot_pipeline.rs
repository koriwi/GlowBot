use super::BotState;
use crate::db::Database;
use crate::git::GitRepo;
use crate::memory::{save_memory, Memory};
use crate::openrouter::{ChatCompletionRequest, ChatMessage, OpenRouterClient};
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
    tools_enabled: bool,
    tg_bot: Option<&teloxide::Bot>,
) -> anyhow::Result<Option<String>> {
    log::info!(
        "pipeline: starting LLM processing for chat={}, user={}, text=\"{}\", has_media={}",
        chat_id,
        user_id,
        text.chars().take(100).collect::<String>(),
        media.is_some()
    );

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
    let history = {
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

    let current_msg =
        build_user_message_full(state, chat_id, text, caption, media, username, tg_bot).await;
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
                let resp = llm.chat_completion(&request).await?;
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
                if let Some(reasoning) = &choice.message.reasoning {
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
        s.db.save_messages(chat_id, &turn_messages)
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

    log::info!(
        "pipeline: done, returning response (len={}) for chat={}",
        result.len(),
        chat_id
    );
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

/// Build the user message for the LLM, handling media ingestion:
/// - Native image: downloads image, encodes as data-URL, builds user_multimodal
/// - Non-native image: metadata + file path so the LLM can use the describe_image tool
/// - Native audio: downloads audio, encodes as base64, builds user_multimodal
/// - Non-native audio: calls audio_fallback_model to transcribe, builds text message
async fn build_user_message_full(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    text: &str,
    caption: Option<&str>,
    media: Option<&crate::media::IngestedMedia>,
    username: &str,
    tg_bot: Option<&teloxide::Bot>,
) -> ChatMessage {
    let media = match media {
        Some(m) => m,
        None => return ChatMessage::user_with_name(text, username),
    };

    let is_image = matches!(media, crate::media::IngestedMedia::Photo { .. });

    // Get model capabilities and config
    let (supports_modality, image_fallback_exists, audio_fallback_model, token, media_dir, api_key) = {
        let s = state.lock().await;
        let model_id = s.effective_model(chat_id);
        let normalized = crate::openrouter::normalize_model_id(&model_id);
        let meta = s.model_metadata.get(normalized);
        let modality = if is_image { "image" } else { "audio" };
        let supports_modality = meta.map(|m| m.supports_modality(modality)).unwrap_or(false);
        let image_fallback_exists = s.config.image_fallback_model_for_chat(chat_id).is_some();
        let audio_fallback_model = s
            .config
            .audio_fallback_model_for_chat(chat_id)
            .map(String::from);
        (
            supports_modality,
            image_fallback_exists,
            audio_fallback_model,
            s.config.telegram_token.clone(),
            s.config.media_dir.clone(),
            s.config.openrouter.api_key.clone(),
        )
    };

    // Download the file from Telegram
    let file_id = match media {
        crate::media::IngestedMedia::Photo { file_id, .. } => file_id.as_str(),
        crate::media::IngestedMedia::Voice { file_id, .. } => file_id.as_str(),
        crate::media::IngestedMedia::Audio { file_id, .. } => file_id.as_str(),
    };

    let dest_dir = crate::media::ingest_dir(&media_dir);

    let file_path = match tg_bot {
        Some(bot) => {
            use teloxide::prelude::*;
            match bot.get_file(file_id).send().await {
                Ok(file) => match crate::media::download_file(&file, &token, &dest_dir).await {
                    Ok(p) => Some(p),
                    Err(e) => {
                        log::warn!("Media: failed to download {}: {}", file_id, e);
                        None
                    }
                },
                Err(e) => {
                    log::warn!("Media: get_file failed for {}: {}", file_id, e);
                    None
                }
            }
        }
        None => {
            log::info!(
                "Media: no tg_bot available, skipping download for {}",
                file_id
            );
            None
        }
    };

    // Build the user message based on capabilities
    if let Some(fp) = file_path {
        if supports_modality {
            build_native_message(media, caption, text, username, &fp)
        } else if is_image {
            build_image_metadata_message(media, caption, text, username, &fp, image_fallback_exists)
        } else if let Some(ref fb_model) = audio_fallback_model {
            build_audio_fallback_message(media, caption, text, username, &fp, fb_model, &api_key)
                .await
        } else {
            build_text_metadata_message(media, caption, text, username)
        }
    } else {
        build_text_metadata_message(media, caption, text, username)
    }
}

/// Build a ChatMessage with native multimodal content parts.
fn build_native_message(
    media: &crate::media::IngestedMedia,
    caption: Option<&str>,
    text: &str,
    username: &str,
    file_path: &std::path::Path,
) -> ChatMessage {
    use crate::openrouter::ContentPart;
    let mut parts: Vec<ContentPart> = Vec::new();

    // Tell the LLM where the ingested file is saved so it can use it
    // as a reference_image for generate_image or pass it to other tools.
    parts.push(ContentPart::Text {
        text: format!("[Ingested file saved to: {}]", file_path.display()),
    });

    match media {
        crate::media::IngestedMedia::Photo { .. } => {
            match crate::media::image_to_data_url(file_path) {
                Ok(data_url) => {
                    parts.push(ContentPart::ImageUrl {
                        image_url: crate::openrouter::ImageUrlDetail {
                            url: data_url,
                            detail: None,
                        },
                    });
                }
                Err(e) => {
                    log::warn!("Media: failed to encode image: {}", e);
                }
            }
        }
        crate::media::IngestedMedia::Voice { .. } | crate::media::IngestedMedia::Audio { .. } => {
            match crate::media::audio_to_base64(file_path) {
                Ok((data, format)) => {
                    parts.push(ContentPart::InputAudio {
                        input_audio: crate::openrouter::InputAudioDetail { data, format },
                    });
                }
                Err(e) => {
                    log::warn!("Media: failed to encode audio: {}", e);
                }
            }
        }
    }

    // Add text parts: caption first, then user text
    if let Some(cap) = caption {
        if !cap.is_empty() {
            parts.push(ContentPart::Text {
                text: cap.to_string(),
            });
        }
    }
    if !text.is_empty() {
        parts.push(ContentPart::Text {
            text: text.to_string(),
        });
    }

    ChatMessage::user_multimodal_with_name(parts, username)
}

/// Build a ChatMessage where audio is transcribed via a fallback model.
async fn build_audio_fallback_message(
    media: &crate::media::IngestedMedia,
    caption: Option<&str>,
    text: &str,
    username: &str,
    file_path: &std::path::Path,
    fallback_model: &str,
    api_key: &str,
) -> ChatMessage {
    let client = OpenRouterClient::new(api_key.to_string());

    let fallback_text = call_audio_fallback(&client, fallback_model, file_path).await;

    let metadata = media_metadata_text(media);
    let mut combined = format!("{} File saved to: {}", metadata, file_path.display());
    if let Some(cap) = caption {
        if !cap.is_empty() {
            combined.push_str(&format!("\nCaption: {}", cap));
        }
    }
    if let Ok(ft) = &fallback_text {
        combined.push_str(&format!("\n\n{}", ft));
    } else if let Err(ref e) = fallback_text {
        log::warn!("Media: fallback conversion failed: {}", e);
        combined.push_str("\n(Conversion failed)");
    }
    if !text.is_empty() {
        combined.push_str(&format!("\n\n{}", text));
    }

    ChatMessage::user_with_name(&combined, username)
}

/// Call an audio-capable fallback model to transcribe audio.
async fn call_audio_fallback(
    client: &OpenRouterClient,
    model: &str,
    audio_path: &std::path::Path,
) -> anyhow::Result<String> {
    let (base64_data, format) = crate::media::audio_to_base64(audio_path)?;
    let parts = vec![
        crate::openrouter::ContentPart::Text {
            text: "Please transcribe this audio file.".into(),
        },
        crate::openrouter::ContentPart::InputAudio {
            input_audio: crate::openrouter::InputAudioDetail {
                data: base64_data,
                format,
            },
        },
    ];
    let msg = ChatMessage::user_multimodal(parts);
    let request = ChatCompletionRequest {
        model: model.to_string(),
        messages: vec![msg],
        tools: None,
        tool_choice: None,
        modalities: None,
        image_config: None,
    };
    let response = client.chat_completion(&request).await?;
    let text = response
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .unwrap_or_default();
    Ok(text)
}

/// Build a metadata message for images when the model doesn't support them natively.
/// Includes file path so the LLM can use the describe_image tool.
fn build_image_metadata_message(
    media: &crate::media::IngestedMedia,
    caption: Option<&str>,
    text: &str,
    username: &str,
    file_path: &std::path::Path,
    has_fallback: bool,
) -> ChatMessage {
    let metadata = media_metadata_text(media);
    let mut combined = format!("{} File saved to: {}", metadata, file_path.display());
    if has_fallback {
        combined.push_str(" Use the describe_image tool with a specific prompt to get visual details (e.g. portion sizes, text reading, object identification, layout).");
    }
    if let Some(cap) = caption {
        if !cap.is_empty() {
            combined.push_str(&format!("\nCaption: {}", cap));
        }
    }
    if !text.is_empty() {
        combined.push_str(&format!("\n\n{}", text));
    }
    ChatMessage::user_with_name(&combined, username)
}

/// Build a text-only metadata message (when download fails or no native/fallback available).
fn build_text_metadata_message(
    media: &crate::media::IngestedMedia,
    caption: Option<&str>,
    text: &str,
    username: &str,
) -> ChatMessage {
    let metadata = media_metadata_text(media);
    let mut combined = metadata;
    if let Some(cap) = caption {
        if !cap.is_empty() {
            combined.push_str(&format!("\nCaption: {}", cap));
        }
    }
    if !text.is_empty() {
        combined.push_str(&format!("\n\n{}", text));
    }
    ChatMessage::user_with_name(&combined, username)
}

/// Produce a metadata prefix for ingested media.
fn media_metadata_text(media: &crate::media::IngestedMedia) -> String {
    match media {
        crate::media::IngestedMedia::Photo { width, height, .. } => {
            format!("[This image ({}x{}) was sent by the user.]", width, height)
        }
        crate::media::IngestedMedia::Voice { duration, .. } => {
            format!(
                "[This voice message ({}s) was sent by the user and was automatically transcribed for you.]",
                duration
            )
        }
        crate::media::IngestedMedia::Audio {
            duration, title, ..
        } => {
            if let Some(t) = title {
                format!(
                    "[This audio file \"{}\" ({}s) was sent by the user and was automatically transcribed for you.]",
                    t, duration
                )
            } else {
                format!(
                    "[This audio file ({}s) was sent by the user and was automatically transcribed for you.]",
                    duration
                )
            }
        }
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
