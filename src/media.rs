use crate::openrouter::ContentPart;
use std::path::{Path, PathBuf};

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

/// Download a Telegram file and save it to `dest_dir`.
/// `file` is the result of `bot.get_file(file_id).send().await?`.
/// `token` is the Telegram bot token for constructing the download URL.
/// Returns the path to the downloaded file.
pub async fn download_file(
    file: &teloxide::types::File,
    token: &str,
    dest_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let file_path = &file.path;

    // Construct download URL
    let url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        token, file_path
    );

    // Download file bytes
    let client = reqwest::Client::new();
    let response = client.get(&url).send().await?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Telegram file download failed ({}): {}",
            response.status(),
            url
        );
    }
    let bytes = response.bytes().await?;

    // Determine output filename from the Telegram file path
    let filename = Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&file.meta.id);

    // Ensure dest dir exists and write file
    std::fs::create_dir_all(dest_dir)?;
    let dest_path = dest_dir.join(filename);
    tokio::fs::write(&dest_path, &bytes).await?;

    log::info!(
        "Media: downloaded {} ({} bytes) to {}",
        file.meta.id,
        bytes.len(),
        dest_path.display()
    );

    Ok(dest_path)
}

/// Get the ingest directory path under the configured media directory.
/// All ingested Telegram files (photos, voice, audio) are saved here.
pub fn ingest_dir(media_dir: &str) -> PathBuf {
    PathBuf::from(media_dir).join("ingest")
}

/// Encode an image file as a base64 data-URL for ContentPart::ImageUrl.
/// Detects MIME type from file extension.
pub fn image_to_data_url(path: &Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(path)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
        .to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/jpeg",
    };
    let b64 = base64_encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, b64))
}

/// Encode an audio file as raw base64 for ContentPart::InputAudio.
/// Returns (base64_data, format). Format is derived from file extension.
pub fn audio_to_base64(path: &Path) -> anyhow::Result<(String, String)> {
    let bytes = std::fs::read(path)?;
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("ogg")
        .to_lowercase();
    let b64 = base64_encode(&bytes);
    Ok((b64, format))
}

/// Base64-encode bytes using the standard engine (with padding).
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
