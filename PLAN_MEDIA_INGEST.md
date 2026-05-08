# Media Ingest — Feature Plan

## Goal

Enable GlowBot to understand and respond to **documents**, **images**, and **voice messages** sent by users — not just text.

Currently, `handle_message` in `main.rs` bails out early for non-text messages:

```rust
let text = match msg.text() {
    Some(t) => t,
    None => return,  // <-- ignores all media
};
```

The bot has **zero awareness** of photos, documents, or voice notes users send.

---

## 1. Design: Model-Aware Ingestion via OpenRouter

### 1.1 Key Insight

OpenRouter's `/api/v1/models` endpoint already returns per-model input capabilities:

```
GET /api/v1/models
→ .data[].architecture.input_modalities[] ∈ ["text", "image", "video", "file", "audio"]
```

This tells us exactly what each model can consume natively. **No heuristics needed.**

### 1.2 Conversion Strategy

For each incoming media type, check the **conversation model's** native `input_modalities`:

| Media | Native Modality? | Action |
|-------|-----------------|--------|
| **Image** | `image` in modalities | Include as `image_url` content part directly in the ChatMessage. No conversion. |
| **Image** | `image` NOT in modalities | Send image to `openrouter.image_fallback_model` → get text description. Inject into user message with metadata. |
| **Voice / Audio** | `audio` in modalities | Include as audio content part directly. (Unlikely via chat completions API — TBD.) |
| **Voice / Audio** | `audio` NOT in modalities | Send audio to `openrouter.audio_fallback_model` → get transcription. Inject into user message with metadata. |
| **Document / File** | `file` in modalities | Include as file content part directly. (Unlikely via chat completions API — TBD.) |
| **Document / File** | `file` NOT in modalities | Send file to `openrouter.file_fallback_model` → get extracted text. Inject into user message with metadata. |
| **Video** | — | **Disabled for now.** Ignored entirely. |

**Practical reality for MVP**: The chat completions API only supports `text` and `image_url` content parts. So:
- **Images**: native path works (vision models get the image directly).
- **Audio and Files**: will **always** go through fallback models, regardless of native modality claim.

### 1.3 Fallback Model Metadata

When a fallback model is used to convert media to text, the resulting text is prepended with metadata so the conversation model knows the context:

> *[This file was sent by the user and was automatically converted to text for you. The original file is named `report.pdf`.]*

The metadata includes the **original filename with extension** so the conversation model can infer the file type.

---

## 2. Architecture Changes

### 2.1 Config Additions (`config.rs` + `config.example.yaml`)

New fields on `OpenRouterConfig`:

```rust
pub struct OpenRouterConfig {
    pub api_key: String,
    pub model: String,
    // --- NEW ---
    /// Model used to convert images to text when the conversation model
    /// doesn't natively support image input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_fallback_model: Option<String>,
    /// Model used to transcribe audio/voice to text when the conversation
    /// model doesn't natively support audio input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_fallback_model: Option<String>,
    /// Model used to extract text from files when the conversation model
    /// doesn't natively support file input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_fallback_model: Option<String>,
}
```

Example `config.yaml`:

```yaml
openrouter:
  api_key: "..."
  model: "anthropic/claude-sonnet-4"
  image_fallback_model: "openai/gpt-4o"          # for describing images
  audio_fallback_model: "openai/whisper-large-v3" # for transcribing voice
  file_fallback_model: "openai/gpt-4o-mini"       # for extracting file content
```

### 2.2 Model Capability Cache (`bot_state.rs` + `openrouter.rs`)

Extend `ModelInfo` to parse `input_modalities`:

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub context_length: u64,
    #[serde(default)]
    pub architecture: ModelArchitecture,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ModelArchitecture {
    #[serde(default)]
    pub input_modalities: Vec<String>,
}

impl ModelInfo {
    /// Check if this model natively supports a given input modality.
    pub fn supports_modality(&self, modality: &str) -> bool {
        self.architecture.input_modalities.iter().any(|m| m == modality)
    }
}
```

Cache in `BotState`:

```rust
pub struct BotState {
    // ... existing fields ...
    /// Cached model metadata from OpenRouter (includes input_modalities).
    pub model_metadata: HashMap<String, ModelInfo>,
}
```

When fetching models on startup, store the full `ModelInfo` instead of just `context_length`. The existing `model_context_lengths` HashMap can be subsumed — replace it with `model_metadata: HashMap<String, ModelInfo>` and derive `context_length` from it.

Or keep them separate for minimal diff — add a new `model_input_modalities: HashMap<String, Vec<String>>` field.

### 2.3 New Types in `media.rs` (new file)

```rust
/// Ingested media extracted from a Telegram message.
#[derive(Debug, Clone)]
pub enum IngestedMedia {
    Photo {
        file_id: String,
        width: u32,
        height: u32,
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
    },
    Audio {
        file_id: String,
        duration: u32,
        title: Option<String>,
    },
    // Video is disabled for now — not included.
}

/// Result of processing ingested media.
pub struct ProcessedMedia {
    /// Text content for the user message (caption, fallback model output, metadata).
    pub text: Option<String>,
    /// Image content parts for vision-capable models (only when native).
    pub image_parts: Vec<ContentPart>,
}
```

### 2.4 Changes to `ContentPart` (`openrouter.rs`)

Add image support:

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
    pub url: String,             // base64 data-URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,  // "low", "high", "auto"
}
```

Add constructors to `ChatMessage`:

```rust
impl ChatMessage {
    /// Create a user message with mixed text + image content parts.
    pub fn user_multimodal(parts: Vec<ContentPart>) -> Self { ... }
    pub fn user_multimodal_with_name(parts: Vec<ContentPart>, name: &str) -> Self { ... }
}
```

### 2.5 Changes to `handle_message` (`main.rs`)

Replace the early `msg.text()` bail-out:

```rust
async fn handle_message(tg_bot: Bot, bot: Arc<Mutex<GlowBot>>, ...) {
    let text = msg.text().map(|s| s.to_string());
    let caption = msg.caption().map(|s| s.to_string());
    let media = IngestedMedia::try_from(&msg);

    if text.is_none() && media.is_none() {
        return; // truly nothing to process
    }

    // ... extract bot components, typing indicator, lock ...

    // Updated pipeline call with media
    process_message_impl(
        &state, &git_repo, &stop_signals,
        &chat_id, &user_id, username,
        text.as_deref(),
        caption.as_deref(),
        media.as_ref(),
        bot_username,
        Some(&tg_bot),
    ).await
}
```

`IngestedMedia::try_from(&msg)` extracts from the teloxide `Message`:
- `msg.photo()` → largest `PhotoSize` → `Photo { file_id, width, height }`
- `msg.document()` → `Document { file_id, mime_type, file_name, file_size }`
- `msg.voice()` → `Voice { file_id, duration }`
- `msg.audio()` → `Audio { file_id, duration, title }`
- `msg.video()` → **skipped** (video disabled)
- `msg.video_note()` → **skipped**

### 2.6 Changes to `process_message_impl` (`bot.rs` + `bot_pipeline.rs`)

Updated signature:

```rust
pub async fn process_message_impl(
    state: &Arc<Mutex<BotState>>,
    git_repo: &GitRepo,
    stop_signals: &Arc<...>,
    chat_id: &str,
    user_id: &str,
    username: &str,
    text: Option<&str>,          // was: text: &str
    caption: Option<&str>,       // NEW: media caption
    media: Option<&IngestedMedia>, // NEW: extracted media
    bot_username: &str,
    tg_bot: Option<&teloxide::Bot>,
) -> anyhow::Result<Option<String>>
```

Logic flow before entering the LLM loop:

```
1. If media is present:
   a. Download file from Telegram
   b. Determine conversation model's input_modalities
   c. Decide: native or fallback?
   d. If native (image + vision model):
      → Build ContentPart::ImageUrl from downloaded file
      → Combine with text/caption parts
      → Build ChatMessage::user_multimodal(parts)
   e. If fallback:
      → Call fallback model with the file
      → Prepend metadata: "[This file was sent by the user and was... original file: foo.pdf.]"
      → Combine with text/caption
      → Build ChatMessage::user_with_name(combined_text, username)

2. If no media → use existing ChatMessage::user_with_name(text, username) path
3. Continue with normal LLM pipeline (tools, tool loop, etc.)
```

### 2.7 New Module: `media.rs`

```rust
use teloxide::Bot;
use std::path::{Path, PathBuf};

/// Download a Telegram file by file_id. Returns local path.
pub async fn download_file(
    bot: &Bot,
    file_id: &str,
    dest_dir: &Path,
    original_name: Option<&str>,
) -> anyhow::Result<PathBuf>

/// Encode an image file as a base64 data-URL for ContentPart::ImageUrl.
pub fn image_to_data_url(path: &Path) -> anyhow::Result<String>

/// Call a fallback model to convert media to text.
/// Sends the file content (as a data-URL for images, or raw text for documents)
/// to the fallback model and returns the description/transcription/extraction.
pub async fn convert_via_fallback(
    client: &OpenRouterClient,
    fallback_model: &str,
    media: &IngestedMedia,
    file_path: &Path,
) -> anyhow::Result<String> {
    // For images: send the image as image_url content part + prompt "Describe this image in detail."
    // For audio: send the audio file? Or a text prompt? (Whisper models use audio endpoint, not chat)
    //   → Audio fallback may need a different API call (OpenRouter's audio transcriptions endpoint?)
    // For files: read raw bytes, send as text (or data-URL) + prompt "Extract all text content from this file."
}

/// Build metadata prefix for fallback-converted media.
pub fn media_metadata(media: &IngestedMedia) -> String {
    match media {
        IngestedMedia::Photo { .. } => 
            "[This image was sent by the user and was automatically converted to text for you.]".into(),
        IngestedMedia::Document { file_name, .. } => 
            format!("[This file was sent by the user and was automatically converted to text for you. The original file is named `{}`.]", 
                file_name.as_deref().unwrap_or("unknown")),
        IngestedMedia::Voice { duration, .. } =>
            format!("[This voice message ({}s) was sent by the user and was automatically transcribed for you.]", duration),
        IngestedMedia::Audio { duration, title, .. } =>
            format!("[This audio file{} ({}s) was sent by the user and was automatically transcribed for you.]",
                title.as_ref().map(|t| format!(" \"{}\"", t)).unwrap_or_default(),
                duration),
    }
}
```

**Important**: Audio fallback (transcription) likely needs OpenRouter's **audio transcriptions** endpoint, not the chat completions endpoint (since Whisper models don't do chat). This may require a new API method on `OpenRouterClient`:

```rust
impl OpenRouterClient {
    /// Transcribe audio via OpenRouter's audio/transcriptions endpoint.
    pub async fn transcribe_audio(
        &self,
        model: &str,       // e.g. "openai/whisper-large-v3"
        audio_path: &Path,
    ) -> anyhow::Result<String>
}
```

### 2.8 System Prompt Updates (`system_prompt.rs`)

Add to the base prompt:

```
## Media Understanding
You can receive and understand images, documents, and voice messages.
- If the current model supports images natively, images are shown to you directly.
- If not, images and files are automatically converted to text by a fallback model.
  Converted media is prefixed with metadata indicating it was converted and the original filename.
- Voice messages are automatically transcribed to text.
- You can send media files using the `send_media` tool.
```

---

## 3. Implementation Phases

### Phase 1: Fetch & Cache Model Modalities

**Files**: `openrouter.rs` (ModelInfo), `openrouter_client.rs`, `main.rs`, `bot_state.rs`

1. Add `ModelArchitecture` + `input_modalities` deserialization to `ModelInfo`
2. Add `model_metadata: HashMap<String, ModelInfo>` to `BotState`
3. Update `fetch_model_contexts` → `fetch_model_metadata` in `GlowBot` / `main.rs` to populate `model_metadata`
4. Update `context_usage()` and all `model_context_lengths` callers to use `model_metadata`
5. Tests: parse mock API response with `architecture.input_modalities`

### Phase 2: Core Types & Telegram Media Extraction

**Files**: new `media.rs`, `main.rs`, `openrouter.rs`

1. Add `IngestedMedia` enum with `TryFrom<&Message>` impl
2. Add `ProcessedMedia` struct
3. Add `ContentPart::ImageUrl` variant + `ChatMessage::user_multimodal` constructors
4. Update `handle_message` to extract media + caption
5. Update `process_message_impl` signature for `Option<&IngestedMedia>`
6. Stub out the pipeline decision (native vs fallback) — just log what would happen
7. Tests: `IngestedMedia::try_from` with mock Telegram message types, `ContentPart` serialization

### Phase 3: Telegram File Download

**Files**: `media.rs`

1. Implement `download_file()` using `Bot::get_file()` + file download
2. Save to `media/ingest/` under data directory
3. Tests with mocked teloxide `Bot`

### Phase 4: Native Image Path (Vision Models)

**Files**: `media.rs`, `bot_pipeline.rs`

1. Implement `image_to_data_url()` — read bytes, detect MIME type, base64 encode
2. In pipeline: if media is Photo AND conversation model has `image` modality:
   - Download image
   - Build `ContentPart::ImageUrl` + text part (caption)
   - Construct `ChatMessage::user_multimodal(parts)`
3. Tests

### Phase 5: Fallback Image → Text

**Files**: `media.rs`, `bot_pipeline.rs`

1. Config: add `image_fallback_model` to `OpenRouterConfig`
2. Implement `convert_image_via_fallback()`:
   - Build a chat completion request to the fallback model
   - Include the image as `image_url` content part + prompt "Describe this image in detail. Include any visible text."
   - Return the description text
3. In pipeline: if media is Photo AND model does NOT have `image` modality:
   - Download image
   - Call fallback model
   - Prepend metadata
   - Build text-only user message
4. Tests

### Phase 6: Fallback File → Text

**Files**: `media.rs`, `bot_pipeline.rs`, `openrouter_client.rs`

1. Config: add `file_fallback_model` to `OpenRouterConfig`
2. Implement `convert_file_via_fallback()`:
   - For text-based files (txt, csv, code): read raw bytes as UTF-8, send as text prompt: "Here is the content of a file. Summarize or extract the relevant information: ..."
   - For binary files (PDF, docx, etc.): read raw bytes, base64 encode, send as data-URL with prompt: "Extract all text content from this file."
   - **Alternative**: use `file` command + `pdftotext` via bash as a pre-step for better extraction, then feed extracted text to the fallback model for summarization. But the user said "use openrouter.file_fallback_model" — so send the file content somehow.
3. File types to handle:
   - Plain text / code: send directly
   - PDF: send as base64 data-URL? Most LLMs can't parse PDF bytes natively. May need bash pre-extraction for PDFs.
   - Word: same issue as PDF
4. In pipeline: if media is Document → download → call fallback → prepend metadata → text-only user message
5. Tests

**Open question for Phase 6**: How to send binary files (PDF, docx) to a text LLM? Options:
- Pre-extract text via bash (`pdftotext`, `pandoc`) then send extracted text to fallback model
- Use a fallback model that natively supports `file` input modality
- The simplest approach: bash pre-extraction as a utility, fallback model for summarization/cleanup

### Phase 7: Fallback Audio → Text

**Files**: `media.rs`, `openrouter_client.rs`

1. Config: add `audio_fallback_model` to `OpenRouterConfig`
2. Add `OpenRouterClient::transcribe_audio()` — calls OpenRouter's audio transcriptions endpoint (if available) or a Whisper model via chat completions
3. In pipeline: if media is Voice or Audio → download → transcribe → prepend metadata → text-only user message
4. Tests

### Phase 8: Config & System Prompt

**Files**: `config.rs`, `config.example.yaml`, `system_prompt.rs`

1. Add fallback model fields to `OpenRouterConfig`
2. Update system prompt
3. Tests

### Phase 9: Integration & Polish

1. End-to-end tests with wiremock for Telegram + OpenRouter APIs
2. Handle edge cases:
   - Telegram download failures (timeout, file too large, deleted)
   - Fallback model failures (API error, timeout)
   - Missing fallback model config (graceful skip: "Image received but no fallback model configured")
   - Multiple photos in one message (media groups — process only the first for now)
   - Media in replies (treat same as regular media)
3. Ensure coverage ≥ 95%

---

## 4. Open Questions

### Q1: Audio transcription API

Does OpenRouter expose an audio transcriptions endpoint? Or do we call a Whisper model via chat completions? Need to verify. If neither works, we fall back to local Whisper CLI or OpenAI's API directly.

**Update from user**: Use `openrouter.audio_fallback_model` — the exact API mechanism (transcriptions endpoint vs chat) depends on what OpenRouter supports. We'll likely need a dedicated transcriptions endpoint call.

### Q2: File content extraction for binary formats

How to feed a PDF or Word doc to a text LLM? Most fallback models won't parse binary files natively. Options:
- **Recommended**: Pre-extract text via bash (`pdftotext` for PDF, `pandoc` for docx, `cat` for text) → send extracted text to fallback model for cleanup/summarization
- Or: if the `file_fallback_model` supports `file` input modality, send the raw file as a data URL

### Q3: Media groups (albums)

Telegram sends multiple photos as an album with a `media_group_id`. Should we process all images or just the first?
- **Recommendation for MVP**: process only the first image. Multi-image handling can come later.

### Q4: File retention

Where to cache downloaded files? Cleanup strategy?
- **Recommendation**: save to `media/ingest/` under the data directory. No automatic cleanup for now (files are small, Telegram limits to 20MB default). Add cleanup in a future iteration.

### Q5: Max file size

Telegram allows up to 20MB for bots. Should we enforce a smaller limit?
- **Recommendation**: respect Telegram's limit implicitly (files >20MB won't be downloadable). Add a configurable `max_download_mb` only if needed.

### Q6: What about `text_content()` for multimodal messages?

`ChatMessage::text_content()` currently returns only text. For multimodal messages (with image parts), it would return an empty/partial string. This is used in:
- `get_recent_messages` tool output
- `embed_turn` background embedding
- Token estimation

We need to ensure these paths handle multimodal content gracefully — either by including an image placeholder in the text or skipping non-text parts.

---

## 5. Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Fallback model calls are slow (extra LLM round-trip) | Run fallback conversion in parallel with typing indicator; conversation model should be fast since it was designed for this |
| Fallback model not configured → media silently ignored | Log warning + include a text note in user message: "(Image received but no image_fallback_model configured)" |
| Large images blow context with base64 encoding | Use `detail: "low"` for large images; OpenRouter may auto-resize |
| Telegram file download fails (deleted, network) | Catch error → include fallback text: "(File `foo.pdf` could not be downloaded)" |
| `text_content()` breaks for multimodal messages | Add image placeholder: `"[image]"` in text_content() for ImageUrl parts |
| Existing tests break on signature change | Update all `process_message_impl` callers — the function is used extensively in `bot_tests.rs` |
