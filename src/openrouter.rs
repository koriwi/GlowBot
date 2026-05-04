use serde::{Deserialize, Serialize};

#[path = "openrouter_tools.rs"]
mod openrouter_tools;
pub use openrouter_tools::all_tool_definitions;
#[allow(unused_imports)]
pub(crate) use openrouter_tools::*;

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
    /// Only populated when `conversation.include_reasoning` is enabled.
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

/// A content part (text or image etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
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
    pub fn text_content(&self) -> String {
        match &self.content {
            ChatContent::Text(t) => t.clone(),
            ChatContent::Parts(parts) => parts
                .iter()
                .map(|p| match p {
                    ContentPart::Text { text } => text.clone(),
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
}

/// Token usage from a chat completion response.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// Model metadata from OpenRouter's /api/v1/models endpoint.
#[derive(Debug, Deserialize, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub context_length: u64,
}

/// Rough token estimation for a string of text.
/// Uses ~1 token per 4 characters (common English approximation).
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64).saturating_add(3) / 4
}

/// Estimate tokens for a `ChatMessage`.
/// Counts role overhead (~4 tokens) plus content text tokens and reasoning if present.
pub fn estimate_message_tokens(msg: &ChatMessage) -> u64 {
    let text = msg.text_content();
    // Role overhead + content; tool_calls JSON adds overhead too
    let mut total = 4 + estimate_tokens(&text);
    if let Some(tcs) = &msg.tool_calls {
        let json = serde_json::to_string(tcs).unwrap_or_default();
        total += estimate_tokens(&json);
    }
    if msg.tool_call_id.is_some() {
        total += 4; // small overhead for tool result messages
    }
    if let Some(ref reasoning) = msg.reasoning {
        total += estimate_tokens(reasoning);
    }
    total
}

/// Estimate tokens for a slice of messages.
pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> u64 {
    messages.iter().map(estimate_message_tokens).sum()
}

/// Estimate tokens for tool definitions by serializing them.
pub fn estimate_tools_tokens(tools: &[ToolDefinition]) -> u64 {
    let json = serde_json::to_string(tools).unwrap_or_default();
    estimate_tokens(&json)
}

/// Safety margin multiplier for token estimates.
/// Since `estimate_tokens` is a rough approximation, we multiply our
/// estimates by this factor so we stay well under the real limit.
pub const TOKEN_ESTIMATE_MARGIN: f64 = 0.75;

/// Reserve tokens for the model's response.
pub const RESPONSE_RESERVE_TOKENS: u64 = 8192;

/// Build a message list that fits within the model's context length.
///
/// - `context_limit`: the model's max context length from OpenRouter (0 = unknown)
/// - `head`: messages that are always preserved (e.g. system prompt, task header)
/// - `history`: prior conversation messages that may be trimmed from oldest
/// - `turn`: current turn messages that are always preserved
/// - `tools`: active tool definitions
///
/// Returns `(messages, was_trimmed)` where `messages` is the list to send.
/// If the context limit is unknown, nothing is trimmed.
pub fn build_trimmed_request(
    context_limit: u64,
    head: &[ChatMessage],
    history: &[ChatMessage],
    turn: &[ChatMessage],
    tools: &[ToolDefinition],
) -> (Vec<ChatMessage>, bool) {
    if context_limit == 0 {
        let mut msgs = head.to_vec();
        msgs.extend(history.iter().cloned());
        msgs.extend(turn.iter().cloned());
        return (msgs, false);
    }

    let effective_limit = (context_limit as f64 * TOKEN_ESTIMATE_MARGIN) as u64;
    let head_tokens = estimate_messages_tokens(head);
    let tools_tokens = estimate_tools_tokens(tools);
    let turn_tokens = estimate_messages_tokens(turn);

    let fixed_cost = head_tokens
        .saturating_add(tools_tokens)
        .saturating_add(turn_tokens)
        .saturating_add(RESPONSE_RESERVE_TOKENS);

    if fixed_cost >= effective_limit {
        log::warn!(
            "Context limit too small: fixed cost {} >= effective limit {} (context limit {})",
            fixed_cost,
            effective_limit,
            context_limit
        );
        // Still try to send head + turn only, history is impossible
        let mut msgs = head.to_vec();
        msgs.extend(turn.iter().cloned());
        return (msgs, true);
    }

    let mut history_budget = effective_limit.saturating_sub(fixed_cost);
    let mut trimmed_history: Vec<ChatMessage> = Vec::new();
    let mut trimmed = false;

    // Walk history oldest → newest, keeping messages while they fit.
    for msg in history {
        let cost = estimate_message_tokens(msg);
        if cost <= history_budget {
            trimmed_history.push(msg.clone());
            history_budget = history_budget.saturating_sub(cost);
        } else {
            trimmed = true;
        }
    }

    if trimmed {
        let dropped = history.len().saturating_sub(trimmed_history.len());
        log::info!(
            "Trimmed {} old messages to fit context limit {} (effective {})",
            dropped,
            context_limit,
            effective_limit
        );
    }

    let mut msgs = head.to_vec();
    msgs.extend(trimmed_history);
    msgs.extend(turn.iter().cloned());
    (msgs, trimmed)
}

/// Trim a flat message list by dropping messages from the *middle*, preserving
/// `preserve_prefix` head messages and `preserve_suffix` tail messages.
/// Used for heartbeat tasks where `messages` is a single flat list.
pub fn trim_message_list(
    messages: &[ChatMessage],
    preserve_prefix: usize,
    preserve_suffix: usize,
) -> Vec<ChatMessage> {
    if messages.len() <= preserve_prefix + preserve_suffix {
        return messages.to_vec();
    }
    let mut result = Vec::with_capacity(preserve_prefix + preserve_suffix + 1);
    result.extend_from_slice(&messages[..preserve_prefix]);
    // Insert a placeholder summary message
    let dropped = messages.len() - preserve_prefix - preserve_suffix;
    result.push(ChatMessage::system(&format!(
        "... {} earlier messages omitted to fit context limit ...",
        dropped
    )));
    result.extend_from_slice(&messages[messages.len() - preserve_suffix..]);
    result
}

/// A response from OpenRouter's chat completions API.
#[derive(Debug, Deserialize, Default)]
pub struct ChatCompletionResponse {
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
}

/// Format token usage as a human-readable string like "37k/252k (15%)".
pub fn format_context_usage(used: u64, limit: u64) -> String {
    if limit == 0 {
        return "unknown".to_string();
    }
    let pct = ((used as f64 / limit as f64) * 100.0).round() as u64;
    let used_k = used / 1000;
    let limit_k = limit / 1000;
    format!("{}k/{}k ({}%)", used_k, limit_k, pct)
}
/// An embedding request to OpenRouter's embeddings API.
#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    input: String,
}

/// A single embedding result.
#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

/// Response from OpenRouter's embeddings API.
#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

pub struct OpenRouterClient {
    api_key: String,
    http_client: reqwest::Client,
}

impl OpenRouterClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http_client: reqwest::Client::new(),
        }
    }

    /// Fetch available models and their context lengths from OpenRouter.
    pub async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let response = self
            .http_client
            .get("https://openrouter.ai/api/v1/models")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenRouter API error ({}): {}", status, body);
        }

        #[derive(Debug, Deserialize)]
        struct ModelsApiResponse {
            data: Vec<ModelInfo>,
        }

        let resp: ModelsApiResponse = response.json().await?;
        Ok(resp.data)
    }

    /// Generate embeddings for a text string using the given model.
    pub async fn embeddings(&self, model: &str, input: &str) -> anyhow::Result<Vec<f32>> {
        let response = self
            .http_client
            .post("https://openrouter.ai/api/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&EmbeddingRequest {
                model: model.to_string(),
                input: input.to_string(),
            })
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenRouter embeddings API error ({}): {}", status, body);
        }

        let resp: EmbeddingResponse = response.json().await?;
        resp.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| anyhow::anyhow!("No embedding data in response"))
    }

    /// Send a chat completion request to OpenRouter.
    pub async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> anyhow::Result<ChatCompletionResponse> {
        let response = self
            .http_client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenRouter API error ({}): {}", status, body);
        }

        let completion: ChatCompletionResponse = response.json().await?;
        Ok(completion)
    }
}


#[cfg(test)]
#[path = "openrouter_tests.rs"]
mod tests;
