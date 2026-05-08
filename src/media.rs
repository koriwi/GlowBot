use crate::openrouter::ContentPart;

/// Ingested media extracted from a Telegram message.
#[derive(Debug, Clone)]
pub enum IngestedMedia {
    Photo {
        file_id: String,
        width: u32,
        height: u32,
    },
    Voice {
        file_id: String,
        duration: u32,
    },
    Audio {
        file_id: String,
        duration: u32,
        title: Option<String>,
    },
}

impl IngestedMedia {
    /// Extract ingestable media from a teloxide `Message`.
    /// Returns `None` if the message doesn't contain supported media.
    /// Video and documents are skipped (out of scope for now).
    pub fn try_from_message(msg: &teloxide::types::Message) -> Option<Self> {
        // Photos: pick the largest one (last in the array by resolution)
        if let Some(photos) = msg.photo() {
            let largest = photos.last()?;
            return Some(IngestedMedia::Photo {
                file_id: largest.file.id.clone(),
                width: largest.width,
                height: largest.height,
            });
        }

        // Voice messages
        if let Some(voice) = msg.voice() {
            return Some(IngestedMedia::Voice {
                file_id: voice.file.id.clone(),
                duration: voice.duration.seconds(),
            });
        }

        // Audio files (non-voice)
        if let Some(audio) = msg.audio() {
            return Some(IngestedMedia::Audio {
                file_id: audio.file.id.clone(),
                duration: audio.duration.seconds(),
                title: audio.title.clone(),
            });
        }

        // Documents, video, video_note — out of scope
        None
    }
}

/// Result of processing ingested media for the LLM.
pub struct ProcessedMedia {
    /// Text content for the user message (caption, fallback model output, metadata).
    pub text: Option<String>,
    /// Multimodal content parts to include alongside text.
    /// ImageUrl for native images, InputAudio for native audio.
    pub content_parts: Vec<ContentPart>,
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
