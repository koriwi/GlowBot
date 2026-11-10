use super::BotState;
use std::sync::Arc;
use tokio::sync::Mutex;

#[path = "bot_image_resize.rs"]
mod image_resize;

/// Handle the `generate_image` tool — generate images via the OpenRouter Image API.
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
        match select_image_model(args, s.config.image_gen_model_for_chat(chat_id)) {
            Ok(model) => model,
            Err(error) => return format!("Error: {error}"),
        }
    };

    let size = args["size"].as_str().map(|s| s.to_string());
    let reference_limit = match image_resize::reference_limit(args) {
        Ok(limit) => limit,
        Err(error) => return format!("Error: {error}"),
    };

    let (api_key, data_dir, media_dir) = {
        let s = state.lock().await;
        (
            s.config.openrouter.api_key.clone(),
            s.data_dir.clone(),
            s.config.media_dir.clone(),
        )
    };

    let mut input_references = Vec::new();

    // Attach reference images as image_url parts
    if let Some(ref_paths) = args["reference_images"].as_array() {
        for p in ref_paths {
            let path_str = p.as_str().unwrap_or("");
            if path_str.is_empty() {
                continue;
            }
            let full_path = resolve_file_path(path_str, &data_dir, &media_dir);
            match image_resize::read_reference(&full_path, reference_limit).await {
                Ok(bytes) => {
                    let mime = guess_mime(&bytes);
                    let b64 = base64_encode(&bytes);
                    input_references.push(crate::openrouter::ContentPart::ImageUrl {
                        image_url: crate::openrouter::ImageUrlDetail {
                            url: format!("data:{};base64,{}", mime, b64),
                            detail: None,
                        },
                    });
                }
                Err(e) => {
                    return format!(
                        "Error: failed to prepare reference image '{}': {e:#}",
                        full_path.display()
                    );
                }
            }
        }
    }

    let request = crate::openrouter::ImageGenerationRequest::new(
        image_gen_model.clone(),
        prompt.to_string(),
        size,
        input_references,
    );

    let client = crate::openrouter::OpenRouterClient::new(api_key);
    let response = match client.generate_images(&request).await {
        Ok(r) => r,
        Err(e) => return format!("Error: image generation failed: {}", e),
    };

    let images = response.data;

    if images.is_empty() {
        return "Error: no images returned from generation".into();
    }

    // Save generated images to the media directory
    let media_path = std::path::PathBuf::from(&media_dir);
    let gen_dir = media_path.join("generated");
    if let Err(e) = std::fs::create_dir_all(&gen_dir) {
        return format!("Error: failed to create media/generated directory: {}", e);
    }

    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let mut saved_paths: Vec<String> = Vec::new();

    for (i, img) in images.iter().enumerate() {
        let b64_data_url = &img.b64_json;
        let image_bytes = match base64_decode(b64_data_url) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("Failed to decode base64 image {}: {}", i, e);
                continue;
            }
        };

        let ext = match img.media_type.as_deref() {
            Some("image/svg+xml") => "svg",
            _ => detect_image_format(&image_bytes).unwrap_or("png"),
        };
        let total = images.len();
        let filename = if total == 1 {
            format!(
                "generated/{}_{}.{}",
                image_gen_model_id(&image_gen_model),
                timestamp,
                ext
            )
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
            log::warn!(
                "Failed to write generated image to {}: {}",
                file_path.display(),
                e
            );
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

    let absolute_paths: Vec<String> = saved_paths
        .iter()
        .map(|p| format!("{}/{}", media_dir.trim_end_matches('/'), p))
        .collect();
    let paths_json = serde_json::json!({"generated_images": absolute_paths}).to_string();
    log::info!(
        "Generated {} image(s) with model {} in chat {}",
        saved_paths.len(),
        image_gen_model,
        chat_id
    );
    paths_json
}

/// A request override never changes the configured default.
fn select_image_model(
    args: &serde_json::Value,
    configured: Option<&str>,
) -> Result<String, &'static str> {
    match args.get("model") {
        None | Some(serde_json::Value::Null) => configured
            .map(str::to_owned)
            .ok_or("image generation model not configured"),
        Some(serde_json::Value::String(model)) if !model.trim().is_empty() => {
            Ok(model.trim().to_owned())
        }
        _ => Err("model must be a non-empty string"),
    }
}

/// Extract a short slug from a model ID for filenames.
fn image_gen_model_id(model: &str) -> String {
    model
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(model)
        .replace([':', '.'], "_")
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
    } else if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}

/// Resolve a file path: try absolute, relative to data_dir, relative to media_dir.
fn resolve_file_path(
    path_str: &str,
    data_dir: &std::path::Path,
    media_dir: &str,
) -> std::path::PathBuf {
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
    let media_candidate = std::path::PathBuf::from(media_dir).join(path_str);
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
    } else if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
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

    #[test]
    fn image_model_override_is_per_request() {
        let configured = Some("default/image");
        assert_eq!(
            select_image_model(&serde_json::json!({}), configured),
            Ok("default/image".into())
        );
        assert_eq!(
            select_image_model(&serde_json::json!({"model":"special/image"}), configured),
            Ok("special/image".into())
        );
        assert_eq!(
            select_image_model(&serde_json::json!({}), configured),
            Ok("default/image".into())
        );
        assert!(select_image_model(&serde_json::json!({}), None).is_err());
        assert_eq!(
            select_image_model(&serde_json::json!({"model":null}), configured),
            Ok("default/image".into())
        );
        for model in [
            serde_json::json!(""),
            serde_json::json!("  "),
            serde_json::json!(42),
        ] {
            assert!(select_image_model(&serde_json::json!({"model":model}), configured).is_err());
        }
    }

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
