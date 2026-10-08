use super::super::BotState;
use crate::openrouter::{ChatCompletionRequest, ChatMessage, OpenRouterClient};
use std::sync::Arc;
use tokio::sync::Mutex;
#[path = "bot_media_format.rs"]
mod bot_media_format;
use bot_media_format::*;
/// Build the user message for the LLM, handling media ingestion:
/// - Native image: downloads image, encodes as data-URL, builds user_multimodal
/// - Non-native image: metadata + file path so the LLM can use the describe_image tool
/// - Native audio: downloads audio, encodes as base64, builds user_multimodal
/// - Non-native audio: calls audio_fallback_model to transcribe, builds text message
pub(crate) async fn build_user_message_full(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    user_id: &str,
    username: &str,
    sender_name: Option<&str>,
    sent_at: Option<chrono::DateTime<chrono::Utc>>,
    text: &str,
    caption: Option<&str>,
    media: Option<&crate::media::IngestedMedia>,
    tg_bot: Option<&teloxide::Bot>,
    decider_model: Option<&str>,
) -> (ChatMessage, bool) {
    let metadata_prefix = message_metadata_prefix(user_id, username, sender_name, sent_at);
    let media = match media {
        Some(m) => m,
        None => {
            return (
                ChatMessage::user_with_name(&format_user_text(&metadata_prefix, text), username),
                true,
            )
        }
    };

    let is_image = matches!(media, crate::media::IngestedMedia::Photo { .. });

    // Get model capabilities and config
    let (supports_modality, image_fallback_exists, audio_fallback_model, token, media_dir, api_key) = {
        let s = state.lock().await;
        let model_id = decider_model
            .map(String::from)
            .unwrap_or_else(|| s.effective_model(chat_id));
        let normalized = crate::openrouter::normalize_model_id(&model_id);
        let meta = if decider_model.is_some() {
            s.decision_metadata.get(&model_id)
        } else {
            s.model_metadata.get(normalized)
        };
        let modality = if is_image { "image" } else { "audio" };
        let supports_modality = native_supported(meta, modality, decider_model.is_some());
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
            let message =
                build_native_message(media, caption, text, username, &metadata_prefix, &fp);
            let complete = match &message.content {
                crate::openrouter::ChatContent::Parts(parts) => parts.iter().any(|p| {
                    matches!(
                        p,
                        crate::openrouter::ContentPart::ImageUrl { .. }
                            | crate::openrouter::ContentPart::InputAudio { .. }
                    )
                }),
                _ => false,
            };
            (message, complete)
        } else if is_image {
            let mut message = build_image_metadata_message(
                media,
                caption,
                text,
                username,
                &metadata_prefix,
                &fp,
                image_fallback_exists,
            );
            let mut complete = decider_model.is_none();
            if decider_model.is_some() {
                let fallback = state
                    .lock()
                    .await
                    .config
                    .image_fallback_model_for_chat(chat_id)
                    .map(String::from);
                if let Some(fallback) = fallback {
                    match super::super::bot_decider::describe(&api_key, &fallback, &fp).await {
                        Ok(description) => {
                            complete = true;
                            if let crate::openrouter::ChatContent::Text(ref mut text) =
                                message.content
                            {
                                text.push_str(&format!("\nImage description: {description}"));
                            }
                        }
                        Err(e) => log::error!("decider: image fallback failed: {}", e),
                    }
                }
            }
            (message, complete)
        } else if let Some(ref fb_model) = audio_fallback_model {
            build_audio_fallback_message(
                media,
                caption,
                text,
                username,
                &metadata_prefix,
                &fp,
                fb_model,
                &api_key,
            )
            .await
        } else {
            (
                build_text_metadata_message(media, caption, text, username, &metadata_prefix),
                decider_model.is_none(),
            )
        }
    } else {
        (
            build_text_metadata_message(media, caption, text, username, &metadata_prefix),
            decider_model.is_none(),
        )
    }
}

/// Build a ChatMessage where audio is transcribed via a fallback model.
async fn build_audio_fallback_message(
    media: &crate::media::IngestedMedia,
    caption: Option<&str>,
    text: &str,
    username: &str,
    metadata_prefix: &str,
    file_path: &std::path::Path,
    fallback_model: &str,
    api_key: &str,
) -> (ChatMessage, bool) {
    let client = OpenRouterClient::new(api_key.to_string());

    let fallback_text = call_audio_fallback(&client, fallback_model, file_path).await;

    let metadata = media_metadata_text(media);
    let mut combined = format!(
        "{}\n{} File saved to: {}",
        metadata_prefix,
        metadata,
        file_path.display()
    );
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

    (
        ChatMessage::user_with_name(&combined, username),
        fallback_text.as_ref().is_ok_and(|t| !t.trim().is_empty()),
    )
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

fn native_supported(
    meta: Option<&crate::openrouter::ModelInfo>,
    modality: &str,
    decider: bool,
) -> bool {
    meta.is_some_and(|m| m.supports_modality(modality)) && (!decider || modality == "image")
}

#[cfg(test)]
#[path = "bot_media_tests.rs"]
mod tests;
