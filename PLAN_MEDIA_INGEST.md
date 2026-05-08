# Media Ingest — Feature Plan

## Goal

Enable GlowBot to understand and respond to **images** and **voice/audio messages** sent by users — not just text.

Currently, `handle_message` in `main.rs` bails out early for non-text messages:

```rust
let text = match msg.text() {
    Some(t) => t,
    None => return,  // <-- ignores all media
};
```

The bot has **zero awareness** of photos, voice notes, or audio files users send.

**Out of scope for now**: Documents/files, video.

---

## 1. Design: Model-Aware Ingestion via OpenRouter

### 1.1 Key Insight

OpenRouter's `/api/v1/models` endpoint returns per-model input capabilities:

```
GET /api/v1/models
→ .data[].architecture.input_modalities[] ∈ ["text", "image", "video", "file", "audio"]
```

This tells us exactly what each model can consume natively. **No heuristics needed.**

### 1.2 OpenRouter Multimodal Content Parts

OpenRouter's chat completions API supports these content part types for multimodal input:

| Content Part Type | Payload | Use Case |
|-------------------|---------|----------|
| `text` | `{ "type": "text", "text": "..." }` | Plain text |
| `image_url` | `{ "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,..." } }` | Images (base64 data-URL or https URL) |
| `input_audio` | `{ "type": "input_audio", "input_audio": { "data": "<raw base64>", "format": "wav" } }` | Audio (raw base64 string, no `data:` prefix) |

### 1.3 Conversion Strategy

For each incoming media type, check the **conversation model's** native `input_modalities`:

| Media | Native Modality? | Action |
|-------|-----------------|--------|
| **Image** | `image` in modalities | Include as `image_url` content part directly in ChatMessage. Zero conversion overhead. |
| **Image** | `image` NOT in modalities | Send image to `openrouter.image_fallback_model` via `image_url` content part → get text description. Prepend metadata, inject as text. |
| **Voice / Audio** | `audio` in modalities | Include as `input_audio` content part directly in ChatMessage (raw base64 + format). Zero conversion overhead. |
| **Voice / Audio** | `audio` NOT in modalities | Send audio to `openrouter.audio_fallback_model` via `input_audio` content part → get transcription. Prepend metadata, inject as text. |

### 1.4 Fallback Model Metadata

When a fallback model converts media to text, the result is prepended with metadata so the conversation model understands the context:

> *[This image was sent by the user and was automatically converted to text for you.]*

> *[This voice message (45s) was sent by the user and was automatically transcribed for you.]*

---

## 2. Architecture Changes

### 2.1 Config Additions (`config.rs` + `config.example.yaml`)

New fields on `OpenRouterConfig`:

```rust
pub struct OpenRouterConfig {
    pub api_key: String,
    pub model: String,
    // --- NEW ---
    /// Model used to describe images when the conversation model
    /// doesn't natively support image input.
    /// This model should support the `image` input modality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_fallback_model: Option<String>,
    /// Model used to transcribe audio/voice when the conversation model
    /// doesn't natively support audio input.
    /// This model should support the `audio` input modality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_fallback_model: Option<String>,
}
```

Example `config.yaml`:

```yaml
openrouter:
  api_key: "..."
  model: "anthropic/claude-sonnet-4"            # supports text + image
  image_fallback_model: "openai/gpt-4o"          # for describing images when needed
  audio_fallback_model: "google/gemini-2.5-flash" # for transcribing audio
```

### 2.2 Model Capability Cache (`openrouter.rs` + `bot_state.rs`)

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

Replace `model_context_lengths: HashMap<String, u64>` in `BotState` with:

```rust
/// Cached model metadata from OpenRouter (context lengths + input modalities).
pub model_metadata: HashMap<String, ModelInfo>,
```

Update all callers that read `model_context_lengths` to use `model_metadata[id].context_length`.

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

/// Result of processing ingested media for the LLM.
pub struct ProcessedMedia {
    /// Text content for the user message (caption, fallback model output, metadata).
    pub text: Option<String>,
    /// Multimodal content parts to include alongside text.
    /// ImageUrl for native images, InputAudio for native audio.
    pub content_parts: Vec<crate::openrouter::ContentPart>,
}
```

### 2.4 Changes to `ContentPart` (`openrouter.rs`)

Add image and audio content parts:

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
    #[serde(rename = "input_audio")]
    InputAudio {
        input_audio: InputAudioDetail,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrlDetail {
    /// Base64 data-URL (e.g. "data:image/jpeg;base64,...") or https URL.
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,  // "low", "high", "auto"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAudioDetail {
    /// Raw base64-encoded audio data (no `data:` prefix).
    pub data: String,
    /// Audio format (e.g. "wav", "mp3", "ogg").
    pub format: String,
}
```

Add constructors to `ChatMessage`:

```rust
impl ChatMessage {
    /// Create a user message with multimodal content parts.
    pub fn user_multimodal(parts: Vec<ContentPart>) -> Self { ... }
    pub fn user_multimodal_with_name(parts: Vec<ContentPart>, name: &str) -> Self { ... }
}
```

Update `text_content()` to handle multimodal parts:

```rust
pub fn text_content(&self) -> String {
    match &self.content {
        ChatContent::Text(t) => t.clone(),
        ChatContent::Parts(parts) => parts
            .iter()
            .map(|p| match p {
                ContentPart::Text { text } => text.clone(),
                ContentPart::ImageUrl { .. } => "[image]".to_string(),
                ContentPart::InputAudio { .. } => "[audio]".to_string(),
            })
            .collect::<Vec<_>>()
            .join(""),
    }
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

    // ... typing indicator, lock, etc. ...

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

`IngestedMedia::try_from(&msg)` extracts from teloxide `Message`:
- `msg.photo()` → largest `PhotoSize` → `Photo { file_id, width, height }`
- `msg.voice()` → `Voice { file_id, duration }`
- `msg.audio()` → `Audio { file_id, duration, title }`
- `msg.document()` → **skipped** (files out of scope)
- `msg.video()` → **skipped**
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
    text: Option<&str>,               // was: text: &str
    caption: Option<&str>,            // NEW
    media: Option<&IngestedMedia>,    // NEW
    bot_username: &str,
    tg_bot: Option<&teloxide::Bot>,
) -> anyhow::Result<Option<String>>
```

New logic **before** the existing LLM tool loop:

```
1. Build the user message:
   a. If NO media:
      → existing path: ChatMessage::user_with_name(text?, username)

   b. If media IS present:
      i.   Look up conversation model's input_modalities from model_metadata cache
      ii.  Determine media kind and model capability:

      IMAGE + model supports "image" (NATIVE):
        → Download image from Telegram
        → image_to_data_url() → ContentPart::ImageUrl
        → parts = [Text(caption?), ImageUrl(data_url)]
        → ChatMessage::user_multimodal_with_name(parts, username)

      IMAGE + model does NOT support "image" (FALLBACK):
        → Download image from Telegram
        → Call image_fallback_model with image as image_url + prompt:
          "Describe this image in detail. Include any visible text."
        → text = "[image metadata] " + caption + "\n\n" + fallback_response
        → ChatMessage::user_with_name(text, username)

      AUDIO/VOICE + model supports "audio" (NATIVE):
        → Download audio from Telegram
        → Read bytes, base64 encode (raw, no data: prefix)
        → ContentPart::InputAudio { data: base64, format: "ogg" }
        → parts = [Text(caption? or voice metadata?), InputAudio(...)]
        → ChatMessage::user_multimodal_with_name(parts, username)

      AUDIO/VOICE + model does NOT support "audio" (FALLBACK):
        → Download audio from Telegram
        → Call audio_fallback_model with audio as input_audio + prompt:
          "Please transcribe this audio file."
        → text = "[audio metadata] " + fallback_transcription
        → ChatMessage::user_with_name(text, username)

2. Continue with normal LLM pipeline (system prompt, history, tools, tool loop, etc.)
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
) -> anyhow::Result<PathBuf>

/// Encode an image file as a base64 data-URL for ContentPart::ImageUrl.
/// Detects MIME type from file extension or magic bytes.
pub fn image_to_data_url(path: &Path) -> anyhow::Result<String>

/// Encode an audio file as raw base64 for ContentPart::InputAudio.
/// Returns (base64_data, format) — format is the file extension without dot.
pub fn audio_to_base64(path: &Path) -> anyhow::Result<(String, String)>

/// Call a fallback model to describe an image.
/// Sends the image as image_url content part with a description prompt.
pub async fn describe_image_via_fallback(
    client: &OpenRouterClient,
    model: &str,
    image_data_url: &str,
) -> anyhow::Result<String>

/// Call a fallback model to transcribe audio.
/// Sends the audio as input_audio content part with a transcription prompt.
pub async fn transcribe_audio_via_fallback(
    client: &OpenRouterClient,
    model: &str,
    audio_base64: &str,
    format: &str,
) -> anyhow::Result<String>

/// Build metadata prefix for fallback-converted media.
pub fn media_metadata(media: &IngestedMedia) -> String {
    match media {
        IngestedMedia::Photo { width, height, .. } =>
            format!("[This image ({}×{}) was sent by the user and was automatically converted to text for you.]", width, height),
        IngestedMedia::Voice { duration, .. } =>
            format!("[This voice message ({}s) was sent by the user and was automatically transcribed for you.]", duration),
        IngestedMedia::Audio { duration, title, .. } =>
            format!("[This audio file{} ({}s) was sent by the user and was automatically transcribed for you.]",
                title.as_ref().map(|t| format!(" \"{}\"", t)).unwrap_or_default(),
                duration),
    }
}
```

### 2.8 System Prompt Updates (`system_prompt.rs`)

Add to the base prompt:

```
## Media Understanding
You can receive and understand images and voice/audio messages.
- If the current model supports images natively, photos are shown to you directly.
  Otherwise, images are automatically described by a fallback model.
- If the current model supports audio natively, voice messages are heard directly.
  Otherwise, audio is automatically transcribed by a fallback model.
- Converted media is prefixed with metadata indicating it was converted.
- You can send media files using the `send_media` tool.
```

---

## 3. Implementation Phases

### Phase 1: Fetch & Cache Model Modalities

**Files**: `openrouter.rs` (ModelInfo), `openrouter_client.rs`, `main.rs`, `bot_state.rs`

1. Add `ModelArchitecture` + `input_modalities` deserialization to `ModelInfo`
2. Add `supports_modality()` helper
3. Replace `model_context_lengths: HashMap<String, u64>` with `model_metadata: HashMap<String, ModelInfo>`
4. Update all callers of `model_context_lengths` (context_usage, etc.)
5. Update `fetch_models` / startup path to populate `model_metadata`
6. Tests: parse mock API response with `architecture.input_modalities`

### Phase 2: Core Types & Telegram Media Extraction

**Files**: new `media.rs`, `main.rs`, `openrouter.rs`

1. Add `IngestedMedia` enum with `TryFrom<&Message>` impl
2. Add `ProcessedMedia` struct
3. Add `ContentPart::ImageUrl` and `ContentPart::InputAudio` variants with their detail types
4. Add `ChatMessage::user_multimodal` and `user_multimodal_with_name` constructors
5. Update `text_content()` to return placeholder strings for non-text parts
6. Update `handle_message` to extract text + caption + media
7. Update `process_message_impl` signature
8. **Stub**: just log what media was detected, don't process yet
9. Tests: `IngestedMedia::try_from` parsing, `ContentPart` serialization round-trips

### Phase 3: Telegram File Download

**Files**: `media.rs`

1. Implement `download_file()` using `Bot::get_file()` + `tokio::fs::write`
2. Save to `glowbot_data/media/ingest/` (create dir if needed)
3. Tests with mocked teloxide `Bot`

### Phase 4: Native Image Path

**Files**: `media.rs`, `bot_pipeline.rs`

1. Implement `image_to_data_url()` — read bytes, detect MIME type, base64 encode with `data:` prefix
2. In pipeline: if media is Photo AND model has `image` modality:
   - Download image
   - Build `ContentPart::ImageUrl` from data-URL
   - Combine with text/caption as `ContentPart::Text`
   - Construct `ChatMessage::user_multimodal(parts)`
3. Tests

### Phase 5: Native Audio Path

**Files**: `media.rs`, `bot_pipeline.rs`

1. Implement `audio_to_base64()` — read bytes, base64 encode (raw, no `data:` prefix), return format
2. In pipeline: if media is Voice/Audio AND model has `audio` modality:
   - Download audio file
   - Build `ContentPart::InputAudio { data, format }`
   - Combine with metadata text as `ContentPart::Text`
   - Construct `ChatMessage::user_multimodal(parts)`
3. Tests

### Phase 6: Fallback Image → Text

**Files**: `media.rs`, `bot_pipeline.rs`

1. Implement `describe_image_via_fallback()`:
   - Build a chat completion request to `image_fallback_model`
   - Include image as `image_url` content part + prompt: "Describe this image in detail. Include any visible text."
   - Return the response text
2. In pipeline: if media is Photo AND model does NOT have `image` modality:
   - Download image
   - Call fallback model
   - Prepend `media_metadata()` + caption
   - Build text-only `ChatMessage::user_with_name()`
3. Handle missing `image_fallback_model` config: include metadata in message, skip fallback call
4. Tests

### Phase 7: Fallback Audio → Text

**Files**: `media.rs`, `bot_pipeline.rs`

1. Implement `transcribe_audio_via_fallback()`:
   - Build a chat completion request to `audio_fallback_model`
   - Include audio as `input_audio` content part + prompt: "Please transcribe this audio file."
   - Return the transcription text
2. In pipeline: if media is Voice/Audio AND model does NOT have `audio` modality:
   - Download audio
   - Call fallback model
   - Prepend `media_metadata()`
   - Build text-only `ChatMessage::user_with_name()`
3. Handle missing `audio_fallback_model` config: include metadata, skip fallback call
4. Tests

### Phase 8: Config & System Prompt

**Files**: `config.rs`, `config.example.yaml`, `system_prompt.rs`

1. Add `image_fallback_model` + `audio_fallback_model` to `OpenRouterConfig`
2. Update system prompt
3. Update `config.example.yaml`
4. Tests

### Phase 9: Integration & Polish

1. End-to-end tests with wiremock for Telegram + OpenRouter APIs
2. Handle edge cases:
   - Telegram download failure → `(File could not be downloaded)`
   - Fallback model API error → metadata only, note the failure
   - Missing fallback config → metadata only
   - Media groups (albums) → process only the first image
   - Media in replies → treat same as regular media
3. Ensure coverage ≥ 95%
4. Update `spec.md` with media ingest section

---

## 4. Open Questions

### Q1: Telegram voice message format

Telegram voice messages are Opus-encoded in an OGG container. The actual format string for `InputAudioDetail.format` depends on what OpenRouter/underlying models accept. Gemini 2.5 Flash example uses `"wav"`. We may need to convert Opus/OGG to a widely-supported format (WAV, MP3) using `ffmpeg` if the model rejects `"ogg"`.

**Recommendation**: Try `"ogg"` first. If the model rejects it, convert to WAV via `ffmpeg -i input.ogg output.wav` before sending.

### Q2: Media groups (albums)

Telegram sends multiple photos as an album with a `media_group_id`. Each photo arrives as a separate message.

**Recommendation for MVP**: process each photo independently as it arrives. The LLM will receive multiple sequential image messages, which is fine. No special grouping logic needed.

### Q3: File retention

Where to cache downloaded files? Cleanup?

**Recommendation**: save to `media/ingest/` under the data directory. No automatic cleanup for MVP (files are small). Add cleanup in future iteration.

### Q4: `text_content()` impact on embeddings and `get_recent_messages`

Multimodal messages return `[image]` or `[audio]` in `text_content()`. This means:
- Embedding will be based on the text parts only (image/audio placeholders barely affect vectors)
- `get_recent_messages` will show `[image]` or `[audio]` placeholders for media content

This is acceptable behavior — the placeholders tell the LLM that media was present.

---

## 5. Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Fallback model calls add latency (extra LLM round-trip) | Run fallback conversion in parallel with typing indicator; fallback models are typically fast (GPT-4o, Gemini Flash) |
| Fallback model not configured → media silently ignored | Include text note in user message: "(Image received but no image_fallback_model configured)" |
| Large images blow context with base64 encoding | Base64 adds ~33% overhead. Most images are <1MB = ~1.3MB base64 = ~20K tokens. Use `detail: "low"` for larger images. |
| Audio format incompatibility (OGG vs WAV) | Try OGG first, fall back to ffmpeg WAV conversion |
| Telegram file download fails (deleted, network) | Catch error → include fallback text: "(File could not be downloaded)" |
| `text_content()` returns placeholders for multimodal messages | Acceptable — placeholders inform the LLM that media was present without bloating context |
| Existing tests break on `process_message_impl` signature change | Update all callers in `bot_tests.rs` (the function is used extensively there) |
