use super::BotState;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

/// Format a byte count into a human-readable size string.
pub(crate) fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}

/// Handle the `list_media` tool — list files in the media directory.
pub(crate) async fn tool_list_media(
    state: &Arc<Mutex<BotState>>,
    args: &serde_json::Value,
) -> String {
    let subpath = args["subpath"].as_str().unwrap_or("");
    let media_dir = { state.lock().await.config.media_dir.clone() };
    // Canonicalize base early so all containment checks below are anchored
    // against the real path (resolves symlinks, normalizes trailing slashes, etc.).
    // If canonicalize fails (e.g. directory doesn't exist yet), fall back to the
    // raw path — the walk will still canonicalize individual entries.
    let base = std::path::PathBuf::from(&media_dir);
    let base = base.canonicalize().unwrap_or_else(|_| base.clone());
    let target = if subpath.is_empty() {
        base.clone()
    } else {
        // Prevent path traversal: only allow relative paths that stay inside media_dir
        let normalized = subpath.trim_start_matches('/').trim_end_matches('/');
        let resolved = base.join(normalized);
        if !resolved.exists() {
            return format!("Error: directory not found: {}", resolved.display());
        }
        match resolved.canonicalize() {
            Ok(p) if p.starts_with(&base) => p,
            _ => {
                return format!(
                    "Error: invalid subpath '{}' — must be inside the media directory '{}'",
                    subpath, media_dir
                );
            }
        }
    };
    if !target.exists() {
        return format!("Error: directory not found: {}", target.display());
    }
    if !target.is_dir() {
        return format!(
            "Error: '{}' is a file, not a directory. Use send_media to send it.",
            target.display()
        );
    }
    let max_entries = 500usize;
    let mut entries: Vec<String> = Vec::new();
    let mut overflow = false;
    fn walk(
        dir: &std::path::Path,
        base: &std::path::Path,
        prefix: &str,
        entries: &mut Vec<String>,
        max: usize,
        overflow: &mut bool,
    ) {
        if *overflow {
            return;
        }
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return,
        };
        let mut items: Vec<std::path::PathBuf> = Vec::new();
        for entry in read_dir.flatten() {
            items.push(entry.path());
        }
        items.sort();
        for path in items {
            if *overflow {
                return;
            }
            // Canonicalize every entry to resolve symlinks and prevent traversal
            // outside the base directory. Entries that escape are silently skipped.
            let canonical = match path.canonicalize() {
                Ok(p) if p.starts_with(base) => p,
                _ => continue,
            };
            let rel = canonical.strip_prefix(base).unwrap_or(&canonical);
            let name = canonical
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("???");
            if canonical.is_dir() {
                if entries.len() >= max {
                    *overflow = true;
                    return;
                }
                entries.push(format!("{}📁 {}/", prefix, name));
                walk(
                    &canonical,
                    base,
                    &format!("{}  ", prefix),
                    entries,
                    max,
                    overflow,
                );
            } else {
                if entries.len() >= max {
                    *overflow = true;
                    return;
                }
                let size = std::fs::metadata(&canonical)
                    .ok()
                    .map(|m| m.len())
                    .unwrap_or(0);
                entries.push(format!(
                    "{}📄 {} ({})",
                    prefix,
                    rel.display(),
                    human_size(size)
                ));
            }
        }
    }
    walk(&target, &base, "", &mut entries, max_entries, &mut overflow);
    let mut out = format!("Media directory listing for '{}':\n", media_dir);
    if entries.is_empty() {
        out.push_str("(empty)");
    } else {
        for e in &entries {
            out.push_str(e);
            out.push('\n');
        }
        if overflow {
            out.push_str("... (truncated at 500 entries)\n");
        }
    }
    out
}

/// Handle the `send_media` tool — send a file as a photo/video/document.
pub(crate) async fn tool_send_media(
    state: &Arc<Mutex<BotState>>,
    chat_id: &str,
    args: &serde_json::Value,
    tg_bot: Option<&teloxide::Bot>,
) -> String {
    let file_path = args["file_path"].as_str().unwrap_or("");
    if file_path.is_empty() {
        return "Error: file_path required".into();
    }
    let caption = args["caption"].as_str().unwrap_or("");
    let original_quality = args["original_quality"].as_bool().unwrap_or(false);
    let (data_dir, media_dir) = {
        let s = state.lock().await;
        (s.data_dir.clone(), s.config.media_dir.clone())
    };
    let resolved = if std::path::Path::new(file_path).is_absolute() {
        std::path::PathBuf::from(file_path)
    } else {
        data_dir.join(file_path)
    };
    if !resolved.exists() {
        return format!("Error: file not found: {}", resolved.display());
    }
    // Canonicalize to resolve symlinks and prevent path traversal.
    // Only allow files inside data_dir or media_dir (the two user-accessible
    // directories). Everything else is rejected.
    let full_path = match resolved.canonicalize() {
        Ok(p) => p,
        Err(e) => return format!("Error: cannot resolve path '{}': {}", file_path, e),
    };
    let media_base = std::path::PathBuf::from(&media_dir);
    let media_base = media_base.canonicalize().unwrap_or(media_base);
    let data_base = data_dir.canonicalize().unwrap_or(data_dir);
    if !full_path.starts_with(&data_base) && !full_path.starts_with(&media_base) {
        return format!(
            "Error: file path '{}' resolves outside allowed directories (data: {}, media: {})",
            file_path,
            data_base.display(),
            media_base.display()
        );
    }
    if let Some(bot) = tg_bot {
        let Ok(chat_id_i64) = chat_id.parse::<i64>() else {
            return format!("Error: invalid chat_id '{}'", chat_id);
        };
        let chat = ChatId(chat_id_i64);
        let input = teloxide::types::InputFile::file(&full_path);
        let ext = full_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let result = if original_quality {
            bot.send_document(chat, input)
                .caption(caption)
                .await
                .map(|_| ())
        } else {
            match ext.as_str() {
                "jpg" | "jpeg" | "png" | "gif" | "webp" => bot
                    .send_photo(chat, input)
                    .caption(caption)
                    .await
                    .map(|_| ()),
                "mp4" | "mov" | "avi" | "webm" => bot
                    .send_video(chat, input)
                    .caption(caption)
                    .await
                    .map(|_| ()),
                "mp3" | "ogg" | "wav" | "flac" => bot
                    .send_audio(chat, input)
                    .caption(caption)
                    .await
                    .map(|_| ()),
                _ => bot
                    .send_document(chat, input)
                    .caption(caption)
                    .await
                    .map(|_| ()),
            }
        };
        match result {
            Ok(()) => format!("Media sent: {}", full_path.display()),
            Err(e) => format!("Failed to send media: {}", e),
        }
    } else {
        "Error: send_media not available in this context.".into()
    }
}
