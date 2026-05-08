use super::BotState;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle the `generate_image` tool — generate images via OpenRouter.
pub(crate) async fn tool_generate_image(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    args: &serde_json::Value,
) -> String {
    let prompt = args["prompt"].as_str().unwrap_or("");
    if prompt.is_empty() {
        return "Error: prompt required".into();
    }

    let image_gen_model = {
        let s = state.lock().await;
        match &s.config.openrouter.image_gen_model {
            Some(m) => m.clone(),
            None => return "Error: image generation model not configured".into(),
        }
    };

    let size = args["size"].as_str().map(|s| s.to_string());
    let n = args["n"]
        .as_u64()
        .map(|v| v.clamp(1, 4) as u32)
        .unwrap_or(1);

    let (api_key, data_dir, media_dir) = {
        let s = state.lock().await;
        (
            s.config.openrouter.api_key.clone(),
            s.data_dir.clone(),
            s.config.media_dir.clone(),
        )
    };

    // Resolve reference images to base64 data-URLs
    let reference_images = resolve_reference_images(args, &data_dir).await;

    // Build the request
    let request = crate::openrouter::ImageGenerationRequest {
        model: image_gen_model.clone(),
        prompt: prompt.to_string(),
        n: Some(n),
        size,
        image: reference_images,
        response_format: Some("b64_json".to_string()),
    };

    let client = crate::openrouter::OpenRouterClient::new(api_key);
    let response = match client.image_generation(&request).await {
        Ok(r) => r,
        Err(e) => return format!("Error: image generation failed: {}", e),
    };

    if response.data.is_empty() {
        return "Error: no images returned from generation API".into();
    }

    // Save generated images to the media directory
    let media_path = std::path::PathBuf::from(&media_dir);
    let gen_dir = media_path.join("generated");
    if let Err(e) = std::fs::create_dir_all(&gen_dir) {
        return format!("Error: failed to create media/generated directory: {}", e);
    }

    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let mut saved_paths: Vec<String> = Vec::new();

    for (i, img) in response.data.iter().enumerate() {
        let image_bytes = if let Some(ref b64) = img.b64_json {
            match base64_decode(b64) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("Failed to decode base64 image {}: {}", i, e);
                    continue;
                }
            }
        } else if let Some(ref url) = img.url {
            // Fallback: download from URL
            match download_image(url).await {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("Failed to download image {} from {}: {}", i, url, e);
                    continue;
                }
            }
        } else {
            log::warn!("Image {} has neither b64_json nor url, skipping", i);
            continue;
        };

        let ext = detect_image_format(&image_bytes).unwrap_or("png");
        let filename = if n == 1 {
            format!("generated/{}_{}.{}", image_gen_model_id(&image_gen_model), timestamp, ext)
        } else {
            format!(
                "generated/{}_{}_{}.{}",
                image_gen_model_id(&image_gen_model),
                timestamp,
                i + 1,
                ext
            )
        };
        let file_path = media_path.join(&filename);
        if let Err(e) = std::fs::write(&file_path, &image_bytes) {
            log::warn!("Failed to write generated image to {}: {}", file_path.display(), e);
            continue;
        }
        // Auto-commit the generated image (best-effort)
        let git_repo = crate::git::GitRepo::new(&data_dir);
        let _ = git_repo.auto_commit(&format!("generate_image: {}", filename));
        saved_paths.push(filename);
    }

    if saved_paths.is_empty() {
        return "Error: failed to save any generated images".into();
    }

    let paths_json =
        serde_json::json!({"generated_images": saved_paths, "media_dir": media_dir}).to_string();
    log::info!(
        "Generated {} image(s) with model {} in chat {}",
        saved_paths.len(),
        image_gen_model,
        chat_id
    );
    paths_json
}

/// Extract a short slug from a model ID for filenames.
fn image_gen_model_id(model: &str) -> String {
    model
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(model)
        .replace(':', "_")
        .replace('.', "_")
}

/// Decode a base64 string (with optional data-URL prefix) to bytes.
fn base64_decode(b64: &str) -> Result<Vec<u8>, String> {
    let encoded = if let Some(stripped) = b64.strip_prefix("data:") {
        // Skip the MIME type prefix, e.g. "data:image/png;base64,"
        match stripped.find(";base64,") {
            Some(pos) => &stripped[pos + 8..],
            None => return Err("Invalid data-URL prefix in base64 response".into()),
        }
    } else {
        b64
    };
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| format!("Base64 decode error: {}", e))
}

/// Detect image format from magic bytes.
fn detect_image_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 8 && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}

/// Download image bytes from a URL.
async fn download_image(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;
    if !response.status().is_success() {
        return Err(format!(
            "Download returned HTTP {}",
            response.status()
        ));
    }
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Failed to read download body: {}", e))
}

/// Resolve reference images from tool args to base64 data-URLs.
/// Accepts file paths relative to the data directory, absolute paths, or
/// paths inside the media directory.
async fn resolve_reference_images(
    args: &serde_json::Value,
    data_dir: &std::path::Path,
) -> Option<Vec<String>> {
    let paths = args["reference_images"].as_array()?;
    if paths.is_empty() {
        return None;
    }
    let mut images: Vec<String> = Vec::new();
    for p in paths {
        let path_str = p.as_str().unwrap_or("");
        if path_str.is_empty() {
            continue;
        }
        let full_path = resolve_file_path(path_str, data_dir);
        match std::fs::read(&full_path) {
            Ok(bytes) => {
                let mime = guess_mime(&bytes);
                let b64 = base64_encode(&bytes);
                images.push(format!("data:{};base64,{}", mime, b64));
            }
            Err(e) => {
                log::warn!(
                    "Failed to read reference image '{}': {}",
                    full_path.display(),
                    e
                );
            }
        }
    }
    if images.is_empty() { None } else { Some(images) }
}

/// Resolve a file path: try absolute, relative to data_dir, relative to media_dir.
fn resolve_file_path(path_str: &str, data_dir: &std::path::Path) -> std::path::PathBuf {
    let p = std::path::Path::new(path_str);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    // Try relative to data_dir first
    let candidate = data_dir.join(path_str);
    if candidate.exists() {
        return candidate;
    }
    // Try relative to media_dir (in case it's not under data_dir)
    let media_candidate = std::path::PathBuf::from("/media").join(path_str);
    if media_candidate.exists() {
        return media_candidate;
    }
    // Return the data_dir-relative path as default (error will surface on read)
    candidate
}

/// Guess MIME type from magic bytes.
fn guess_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 8 && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/png" // default
    }
}

/// Base64-encode bytes (no data-URL prefix).
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn base64_decode_for_test(b64: &str) -> Result<Vec<u8>, String> {
        base64_decode(b64)
    }

    pub(crate) fn detect_image_format_for_test(bytes: &[u8]) -> Option<&'static str> {
        detect_image_format(bytes)
    }

    pub(crate) fn image_gen_model_id_for_test(model: &str) -> String {
        image_gen_model_id(model)
    }

    pub(crate) fn guess_mime_for_test(bytes: &[u8]) -> &'static str {
        guess_mime(bytes)
    }
}
