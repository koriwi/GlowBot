use crate::openrouter::ChatMessage;
pub(super) fn message_metadata_prefix(
    user_id: &str,
    username: &str,
    sender_name: Option<&str>,
    sent_at: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    let sent_at = sent_at.unwrap_or_else(chrono::Utc::now).to_rfc3339();
    let sender_id = if user_id.trim().is_empty() {
        "unknown"
    } else {
        user_id
    };
    let sender_username = if username.trim().is_empty() {
        "unknown"
    } else {
        username
    };
    let sender_name = sender_name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("unknown");

    format!(
        "[Telegram message metadata]\nSent at: {sent_at}\nSender ID: {sender_id}\nSender name: {sender_name}\nSender username: {sender_username}"
    )
}

pub(super) fn format_user_text(metadata_prefix: &str, text: &str) -> String {
    if text.trim().is_empty() {
        metadata_prefix.to_string()
    } else {
        format!("{metadata_prefix}\n\nMessage:\n{text}")
    }
}

/// Build a ChatMessage with native multimodal content parts.
pub(super) fn build_native_message(
    media: &crate::media::IngestedMedia,
    caption: Option<&str>,
    text: &str,
    username: &str,
    metadata_prefix: &str,
    file_path: &std::path::Path,
) -> ChatMessage {
    use crate::openrouter::ContentPart;
    let mut parts: Vec<ContentPart> = Vec::new();

    // Tell the LLM where the ingested file is saved so it can use it
    // as a reference_image for generate_image or pass it to other tools.
    parts.push(ContentPart::Text {
        text: format!(
            "{}\n[Ingested file saved to: {}]\n",
            metadata_prefix,
            file_path.display()
        ),
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

/// Build a metadata message for images when the model doesn't support them natively.
/// Includes file path so the LLM can use the describe_image tool.
pub(super) fn build_image_metadata_message(
    media: &crate::media::IngestedMedia,
    caption: Option<&str>,
    text: &str,
    username: &str,
    metadata_prefix: &str,
    file_path: &std::path::Path,
    has_fallback: bool,
) -> ChatMessage {
    let metadata = media_metadata_text(media);
    let mut combined = format!(
        "{}\n{} File saved to: {}",
        metadata_prefix,
        metadata,
        file_path.display()
    );
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
pub(super) fn build_text_metadata_message(
    media: &crate::media::IngestedMedia,
    caption: Option<&str>,
    text: &str,
    username: &str,
    metadata_prefix: &str,
) -> ChatMessage {
    let metadata = media_metadata_text(media);
    let mut combined = format!("{}\n{}", metadata_prefix, metadata);
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
pub(super) fn media_metadata_text(media: &crate::media::IngestedMedia) -> String {
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
