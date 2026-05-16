use serde::{Deserialize, Serialize};

#[path = "openrouter_tools.rs"]
mod openrouter_tools;
pub use openrouter_tools::all_tool_definitions;
#[allow(unused_imports)]
pub(crate) use openrouter_tools::*;

#[path = "openrouter_context.rs"]
mod openrouter_context;
pub use openrouter_context::{
    build_trimmed_request, estimate_message_tokens, estimate_messages_tokens, estimate_tokens,
    estimate_tools_tokens, format_context_usage, strip_orphaned_tool_results, trim_message_list,
    RESPONSE_RESERVE_TOKENS, TOKEN_ESTIMATE_MARGIN,
};

#[path = "openrouter_client.rs"]
mod openrouter_client;
#[cfg(test)]
pub(crate) use openrouter_client::truncate_str;
pub use openrouter_client::OpenRouterClient;

/// Known OpenRouter routing specifiers and their button labels.
pub const SPECIFIER_BUTTONS: &[(&str, &str)] = &[
    ("nitro", "🔼 :nitro"),
    ("floor", "💰 :floor"),
    ("free", "🆓 :free"),
];

/// Normalize an OpenRouter model ID by stripping the optional `:provider` routing suffix.
/// The `/api/v1/models` endpoint returns IDs like `deepseek/deepseek-v4-pro`, but the config
/// may specify `deepseek/deepseek-v4-pro:deepseek` to route to a specific provider.
pub(crate) fn normalize_model_id(model: &str) -> &str {
    model
        .rsplit_once(':')
        .map(|(base, _)| base)
        .unwrap_or(model)
}

/// Apply a routing specifier (e.g. `nitro`, `floor`, `free`) to a model ID.
/// Strips any existing trailing `:something` suffix and appends the new specifier.
///
/// Examples:
/// - `apply_specifier("openai/gpt-4o", "nitro")` → `"openai/gpt-4o:nitro"`
/// - `apply_specifier("openai/gpt-4o:nitro", "floor")` → `"openai/gpt-4o:floor"`
/// - `apply_specifier("deepseek/deepseek-chat:deepseek", "free")` → `"deepseek/deepseek-chat:free"`
pub fn apply_specifier(model: &str, specifier: &str) -> String {
    let base = normalize_model_id(model);
    format!("{}:{}", base, specifier)
}

/// An OpenRouter chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: ChatContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Reasoning / thinking content from models that support it (e.g. DeepSeek-R1, Claude thinking).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reasoning: Option<String>,
}

/// Content can be a simple string or an array of content parts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// A content part (text, image, or audio).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrlDetail },
    #[serde(rename = "input_audio")]
    InputAudio { input_audio: InputAudioDetail },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrlDetail {
    /// Base64 data-URL (e.g. "data:image/jpeg;base64,...") or https URL.
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAudioDetail {
    /// Raw base64-encoded audio data (no `data:` prefix).
    pub data: String,
    /// Audio format (e.g. "wav", "mp3", "ogg").
    pub format: String,
}

impl ChatMessage {
    pub fn system(content: &str) -> Self {
        Self {
            role: "system".into(),
            content: ChatContent::Text(content.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning: None,
        }
    }

    pub fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: ChatContent::Text(content.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning: None,
        }
    }

    pub fn user_with_name(content: &str, name: &str) -> Self {
        Self {
            role: "user".into(),
            content: ChatContent::Text(content.to_string()),
            name: Some(name.to_string()),
            tool_calls: None,
            tool_call_id: None,
            reasoning: None,
        }
    }

    /// Create a user message with multimodal content parts (text, images, audio).
    pub fn user_multimodal(parts: Vec<ContentPart>) -> Self {
        Self {
            role: "user".into(),
            content: ChatContent::Parts(parts),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning: None,
        }
    }

    /// Create a user message with multimodal content parts and a name.
    pub fn user_multimodal_with_name(parts: Vec<ContentPart>, name: &str) -> Self {
        Self {
            role: "user".into(),
            content: ChatContent::Parts(parts),
            name: Some(name.to_string()),
            tool_calls: None,
            tool_call_id: None,
            reasoning: None,
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".into(),
            content: ChatContent::Text(content.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning: None,
        }
    }

    /// Create an assistant message with reasoning/thinking content.
    pub fn assistant_with_reasoning(content: &str, reasoning: String) -> Self {
        Self {
            role: "assistant".into(),
            content: ChatContent::Text(content.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning: Some(reasoning),
        }
    }

    pub fn tool_result(tool_call_id: &str, content: &str) -> Self {
        Self {
            role: "tool".into(),
            content: ChatContent::Text(content.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.to_string()),
            reasoning: None,
        }
    }

    /// Create an assistant message with tool calls.
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: ChatContent::Text(String::new()),
            name: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            reasoning: None,
        }
    }

    /// Create an assistant message with tool calls and reasoning.
    pub fn assistant_tool_calls_with_reasoning(
        tool_calls: Vec<ToolCall>,
        reasoning: String,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content: ChatContent::Text(String::new()),
            name: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            reasoning: Some(reasoning),
        }
    }

    /// Extract text content regardless of format.
    /// Non-text parts (images, audio) produce placeholder markers.
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
}

/// A tool call from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

/// A function call within a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Tool definition for the bash tool.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub def_type: String,
    pub function: FunctionDef,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A request to OpenRouter's chat completions API.
#[derive(Debug, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    /// Request image/audio generation. For image generation use `["image"]` or `["image", "text"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Vec<String>>,
    /// Image generation configuration (aspect_ratio, image_size, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_config: Option<ImageConfig>,
}

/// Image generation configuration for chat completions.
#[derive(Debug, Clone, Serialize)]
pub struct ImageConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_size: Option<String>,
}

/// Token usage from a chat completion response.
/// Uses `deserialize_u64_flexible` so both integers (12345) and
/// floating-point values (10813.44) from OpenRouter providers are accepted.
#[derive(Debug, Default, Clone)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl<'de> serde::Deserialize<'de> for Usage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            #[serde(default, deserialize_with = "deserialize_u64_flexible")]
            prompt_tokens: u64,
            #[serde(default, deserialize_with = "deserialize_u64_flexible")]
            completion_tokens: u64,
            #[serde(default, deserialize_with = "deserialize_u64_flexible")]
            total_tokens: u64,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Usage {
            prompt_tokens: raw.prompt_tokens,
            completion_tokens: raw.completion_tokens,
            total_tokens: raw.total_tokens,
        })
    }
}

/// Helper: deserialize a `u64` from a JSON value that may be either
/// an integer or a floating-point number (some OpenRouter providers
/// emit token counts as floats, e.g. 10813.44).
fn deserialize_u64_flexible<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(deserializer)?;
    match v {
        serde_json::Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().map(|f| f as u64))
            .ok_or_else(|| {
                serde::de::Error::custom(format!("cannot convert number to u64: {}", n))
            }),
        serde_json::Value::Null => Ok(0),
        _ => Err(serde::de::Error::custom(format!(
            "expected number, got {}",
            v
        ))),
    }
}

/// Model architecture metadata from OpenRouter's /api/v1/models endpoint.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct ModelArchitecture {
    /// Input modalities the model natively supports: "text", "image", "audio", "file", "video".
    #[serde(default)]
    pub input_modalities: Vec<String>,
}

/// Pricing information for a model. The API returns strings that may be "0" or decimal strings like "0.0000025".
/// We parse as string then convert to f64 as needed.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct ModelPricing {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub completion: String,
    #[serde(default)]
    pub request: String,
}

impl ModelPricing {
    /// Check if this model is free (prompt and completion costs are "0").
    pub fn is_free(&self) -> bool {
        self.prompt == "0" && self.completion == "0"
    }

    /// Format the prompt/completion prices per million tokens for display.
    /// The API returns per-token prices like "0.00000015";
    /// we scale by 1e6 to show meaningful per-million rates like "0.15/0.45".
    pub fn format_per_million(&self) -> String {
        let prompt: f64 = self.prompt.parse().unwrap_or(0.0);
        let completion: f64 = self.completion.parse().unwrap_or(0.0);
        format!(
            "{}/{}",
            trim_price(prompt * 1_000_000.0),
            trim_price(completion * 1_000_000.0)
        )
    }
}

/// Format a price value: round to 4 decimal places, strip trailing zeros.
fn trim_price(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0');
    s.trim_end_matches('.').to_string()
}

/// Model metadata from OpenRouter's /api/v1/models endpoint.
#[derive(Debug, Deserialize, Clone)]
pub struct ModelInfo {
    pub id: String,
    /// Human-readable name (e.g. "OpenAI: GPT-4o").
    #[serde(default)]
    pub name: String,
    /// Unix timestamp of when the model was created / added to OpenRouter.
    #[serde(default)]
    pub created: u64,
    pub context_length: u64,
    #[serde(default)]
    pub architecture: ModelArchitecture,
    /// Pricing information (prompt, completion, request costs as strings).
    #[serde(default)]
    pub pricing: ModelPricing,
}

impl ModelInfo {
    /// Get the provider part of the model ID (e.g. "openai" from "openai/gpt-4o").
    pub fn provider(&self) -> &str {
        self.id.split('/').next().unwrap_or("unknown")
    }

    /// Check if this model natively supports a given input modality.
    pub fn supports_modality(&self, modality: &str) -> bool {
        self.architecture
            .input_modalities
            .iter()
            .any(|m| m == modality)
    }
}

/// A response from OpenRouter's chat completions API.
/// `choices` defaults to empty vec so error responses that omit the field
/// (e.g. provider-level errors wrapped by OpenRouter) don't break deserialization.
#[derive(Debug, Deserialize, Default)]
pub struct ChatCompletionResponse {
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// A choice in the chat completion response.
#[derive(Debug, Deserialize, Default)]
pub struct Choice {
    pub message: AssistantMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// The assistant message from the API.
#[derive(Debug, Deserialize, Default)]
pub struct AssistantMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Reasoning / thinking content (e.g. DeepSeek-R1 `reasoning_content`, Claude thinking).
    /// OpenRouter exposes this as `reasoning` on the message object.
    #[serde(default)]
    pub reasoning: Option<String>,
    pub role: Option<String>,
    /// Generated images from the assistant (when modalities includes "image").
    #[serde(default)]
    pub images: Option<Vec<GeneratedImage>>,
}

/// A generated image from a chat completion response.
#[derive(Debug, Clone, Deserialize)]
pub struct GeneratedImage {
    #[serde(rename = "type")]
    pub image_type: Option<String>,
    pub image_url: ImageUrlDetail,
}

/// An embedding request to OpenRouter's embeddings API.
#[derive(Debug, Serialize)]
pub(crate) struct EmbeddingRequest {
    pub(crate) model: String,
    pub(crate) input: String,
}

/// A single embedding result.
#[derive(Debug, Deserialize)]
pub(crate) struct EmbeddingData {
    pub(crate) embedding: Vec<f32>,
}

/// Response from OpenRouter's embeddings API.
#[derive(Debug, Deserialize)]
pub(crate) struct EmbeddingResponse {
    pub(crate) data: Vec<EmbeddingData>,
}

#[cfg(test)]
#[path = "openrouter_tests.rs"]
mod tests;
