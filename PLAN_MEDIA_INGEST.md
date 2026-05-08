# Media Ingest — Feature Plan

## Goal

Enable GlowBot to understand and respond to **documents**, **images**, **voice messages**, and **videos** sent by users — not just text.

Currently, `handle_message` in `main.rs` bails out early for non-text messages:

```rust
let text = match msg.text() {
    Some(t) => t,
    None => return,  // <-- ignores all media
};
```

The bot has **zero awareness** of photos, documents, voice notes, or videos users send.

---

## 1. What "Ingest" Means

For each media type, "understanding" means converting it into a form the LLM can consume:

| Media Type | Conversion | Output |
|------------|-----------|--------|
| **Photo / Image** | For vision models: include as `image_url` content part in the ChatMessage. For non-vision models: describe metadata (dimensions, caption) as text. | Multi-modal content part or text description |
| **Document** (PDF, txt, Word, code, etc.) | Download → extract text via bash tools (`pdftotext`, `pandoc`, `file`, `cat`). | Text appended to user message |
| **Voice Message** | Download .ogg → transcribe via Whisper (bash or API). | Transcription replaces as user text |
| **Video** | Download → for now, describe metadata only. Vision models could see key frames later. | Text description or image frame |
| **Audio** (non-voice) | Download → similar to voice but lower priority. | Transcription or metadata |

---

## 2. Architecture Changes

### 2.1 New Types in `media.rs` (new file)

```rust
/// Ingested media extracted from a Telegram message.
pub enum IngestedMedia {
    Photo {
        file_id: String,
        width: u32,
        height: u32,
        file_unique_id: String,
    },
    Document {
        file_id: String,
        mime_type: Option<String>,
        file_name: Option<String>,
        file_size: u64,
    },
    Voice {
        file_id: String,
        duration: u32,
        mime_type: Option<String>,
    },
    Video {
        file_id: String,
        duration: u32,
        width: u32,
        height: u32,
        file_name: Option<String>,
    },
    Audio {
        file_id: String,
        duration: u32,
        title: Option<String>,
        performer: Option<String>,
    },
}

/// Result of processing ingested media for the LLM.
pub struct ProcessedMedia {
    /// Text content extracted or transcribed from the media.
    pub text: String,
    /// Image content parts for vision-capable models.
    pub image_parts: Vec<crate::openrouter::ContentPart>,
}
```

### 2.2 Changes to `ChatMessage` and `ContentPart` (`openrouter.rs`)

Add image support to `ContentPart`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl {
        image_url: ImageUrlDetail,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrlDetail {
    pub url: String,             // base64 data-URL or https URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,  // "low", "high", "auto"
}
```

Add convenience constructors to `ChatMessage`:

```rust
impl ChatMessage {
    /// Create a user message with mixed text and image content parts.
    pub fn user_multimodal(parts: Vec<ContentPart>) -> Self { ... }
    pub fn user_multimodal_with_name(parts: Vec<ContentPart>, name: &str) -> Self { ... }
}
```

### 2.3 Changes to `handle_message` (`main.rs`)

Replace the early `msg.text()` bail-out with media-aware extraction:

```rust
async fn handle_message(...) {
    let text = msg.text().map(|s| s.to_string());
    let media = IngestedMedia::try_from(&msg);
    let caption = msg.caption().map(|s| s.to_string());

    if text.is_none() && media.is_none() {
        return; // truly nothing to process
    }

    // ... extract bot components, acquire lock, etc.

    // Then call the updated pipeline
    process_message_impl(
        &state,
        &git_repo,
        &stop_signals,
        &chat_id,
        &user_id,
        username,
        &text,              // becomes Option<&str>
        &caption,            // media caption
        &media,              // Option<&IngestedMedia>
        bot_username,
        Some(&tg_bot),
    ).await
}
```

`IngestedMedia::try_from(&msg)` maps Telegram message fields:
- `msg.photo()` → pick largest `PhotoSize` → `Photo { file_id, width, height }`
- `msg.document()` → `Document { file_id, mime_type, file_name, file_size }`
- `msg.voice()` → `Voice { file_id, duration, mime_type }`
- `msg.video()` → `Video { file_id, duration, width, height }`
- `msg.audio()` → `Audio { file_id, duration, title, performer }`

### 2.4 Changes to `process_message_impl` (`bot.rs`)

Signature accepts media:

```rust
pub async fn process_message_impl(
    state: &Arc<Mutex<BotState>>,
    git_repo: &GitRepo,
    stop_signals: &Arc<...>,
    chat_id: &str,
    user_id: &str,
    username: &str,
    text: Option<&str>,
    caption: Option<&str>,
    media: Option<&IngestedMedia>,
    bot_username: &str,
    tg_bot: Option<&teloxide::Bot>,
) -> anyhow::Result<Option<String>>
```

Logic flow:
1. If media present → download & process via `media::process_ingested()`
2. Build the user message:
   - **Vision model + image**: use `ChatMessage::user_multimodal_with_name(parts, name)` with text parts + image parts
   - **All other cases**: use `ChatMessage::user_with_name(combined_text, name)` where `combined_text` = text + caption + extracted document text + transcription + media metadata
3. Continue with normal LLM pipeline

### 2.5 New Module: `media.rs`

```rust
/// Process ingested media: download from Telegram, extract/transcribe,
/// and return text + image parts for the LLM.
pub async fn process_ingested(
    bot: &teloxide::Bot,
    media: &IngestedMedia,
    caption: Option<&str>,
    data_dir: &Path,
    is_vision_model: bool,
) -> anyhow::Result<ProcessedMedia>
```

Internal functions:
- `download_file(bot, file_id, dest_path)` — downloads a Telegram file via `bot.get_file(id)` → `file_info.download()`
- `extract_document_text(path, mime_type)` → runs appropriate bash command, returns text
- `transcribe_voice(path)` → runs `whisper` via bash, returns transcription
- `image_to_content_part(path)` → reads image bytes, encodes as base64 data-URL, returns `ContentPart::ImageUrl`
- `describe_media(media, caption)` → returns a text description e.g. "User sent a photo (1024×768)"

### 2.6 Vision Model Detection

Add to `config.rs` or a utility in `openrouter.rs`:

```rust
/// Check if a model ID likely supports vision/images.
pub fn is_vision_model(model: &str) -> bool {
    let model_lower = model.to_lowercase();
    model_lower.contains("vision")
        || model_lower.contains("gpt-4o")
        || model_lower.contains("gpt-4-turbo")
        || model_lower.contains("claude-3")
        || model_lower.contains("claude-3.5")
        || model_lower.contains("claude-sonnet-4")
        || model_lower.contains("gemini-1.5")
        || model_lower.contains("gemini-2")
        || model_lower.contains("gemini-flash")
        || model_lower.contains("gemini-pro")
        || model_lower.contains("pixtral")
        || model_lower.contains("llava")
}
```

This is a heuristic. A better approach later: use OpenRouter's model metadata (if they expose vision capability) or maintain a configurable list.

### 2.7 Config Additions

Optional new section in `config.yaml`:

```yaml
# Media ingest settings
media_ingest:
  # Maximum file size in MB for download (default: 20)
  max_download_mb: 20
  # Where to cache downloaded files (relative to data dir, default: "media/ingest")
  cache_dir: "media/ingest"
  # Enable voice transcription (default: true)
  transcribe_voice: true
  # Enable document text extraction (default: true)
  extract_documents: true
  # Enable image ingestion for vision models (default: true)
  ingest_images: true
```

### 2.8 System Prompt Updates

Add a section to the base system prompt about media capabilities:

```
## Media Understanding
You can receive and understand images, documents, and voice messages.
- Images sent to you are included visually if the model supports vision.
  If not, the image metadata (dimensions) and caption are provided as text.
- Documents (PDF, text, Word, etc.) have their text extracted automatically.
  The content is included in the user message.
- Voice messages are transcribed to text automatically.
- You can also send media files using the `send_media` tool.
```

---

## 3. Implementation Phases

### Phase 1: Core Infrastructure (no media processing yet)

Files touched: `main.rs`, `bot.rs`, `bot_pipeline.rs`, new `media.rs` (stubs)

1. Add `IngestedMedia` enum and `TryFrom<&Message>` impl
2. Add `ProcessedMedia` struct  
3. Add `ContentPart::ImageUrl` variant + `ChatMessage::user_multimodal` constructors
4. Update `handle_message` to extract media from Telegram message
5. Update `process_message_impl` signature to accept `Option<&IngestedMedia>`
6. Write unit tests for `IngestedMedia::try_from` with mock Telegram message types

### Phase 2: Telegram File Download

Files touched: `media.rs`

1. Implement `download_file()` using teloxide's `Bot::get_file()` and file download
2. Implement `download_media()` that dispatches by media type
3. Tests with mocked teloxide `Bot` (or wiremock)

### Phase 3: Document Text Extraction

Files touched: `media.rs`

1. Implement `extract_document_text()`:
   - Plain text / code: read directly
   - PDF: shell out to `pdftotext` (from poppler-utils)
   - Word: shell out to `pandoc` or `antiword`
   - Fallback: `file` command for type detection + note in text
2. Integration with bash tool for commands
3. Tests with real small test files (temp dir)

### Phase 4: Voice Transcription

Files touched: `media.rs`

1. Implement `transcribe_voice()`:
   - Use `whisper` CLI if available
   - Or use OpenRouter audio API (if they expose it)
   - Or use OpenAI Whisper API directly
2. Fallback: note "Voice message received (voice transcription unavailable)"
3. Tests

### Phase 5: Vision Model Image Integration

Files touched: `media.rs`, `bot_pipeline.rs`, `openrouter.rs`

1. Implement `is_vision_model()` detection
2. Implement `image_to_content_part()` — read image, base64 encode, build data-URL
3. In pipeline: when media is a photo AND model supports vision:
   - Build `ChatMessage::user_multimodal` with text parts + image parts
4. Tests

### Phase 6: Config & System Prompt

Files touched: `config.rs`, `config.example.yaml`, `system_prompt.rs`

1. Add `MediaIngestConfig` struct
2. Update system prompt
3. Update `config.example.yaml`
4. Tests

### Phase 7: Integration & Polish

1. End-to-end tests with wiremock for Telegram API
2. Handle edge cases:
   - Very large files (respect `max_download_mb`)
   - Download failures (timeout, network)
   - Missing extraction tools (graceful fallback)
   - Multiple photos in one message (Telegram media groups)
   - Media in replies
3. Ensure coverage doesn't drop below 95%

---

## 4. Open Questions

1. **Vision model detection**: Rely on heuristics or source of truth? OpenRouter's model list endpoint *may* expose vision capability — investigate with a test call.

2. **Voice transcription provider**: 
   - Bash/whipser CLI: needs whisper model on disk, slow first run
   - OpenRouter audio API: check if available
   - OpenAI Whisper API: reliable, costs money, needs separate API key
   - Recommendation: start with bash/whisper (container can have whisper installed), make it configurable later

3. **Media groups**: Telegram sends multiple photos as an album (media group). Should we process all images or just the first?

4. **File retention**: Should downloaded files be cleaned up after processing? Or kept for reference? Current plan: keep in `media/ingest/` with auto-cleanup after N days (future).

5. **Document size limits**: PDFs can be enormous. Should we truncate extracted text? Current plan: extract full text but truncate if it would blow context window (>~50K chars).

6. **Video**: Worth doing frame extraction for vision models? Defer to later — metadata-only for MVP.

---

## 5. Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Large files crash the bot | Enforce `max_download_mb` before downloading |
| Missing extraction tools (no pdftotext) | Graceful fallback: "Document received: `filename.pdf` (text extraction unavailable)" |
| Voice transcription takes too long | Set timeout, fall back to "Voice message (duration: 45s)" |
| Image too large for vision model | Resize/compress before encoding; use `detail: "low"` |
| Base64 encoding bloats context | Already doing context trimming; vision models have large context windows |
| Non-vision models get confused by image mentions | Only describe media when it's not a vision model; keep it brief |
