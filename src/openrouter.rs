use serde::{Deserialize, Serialize};

#[path = "openrouter_tools.rs"]
mod openrouter_tools;
pub use openrouter_tools::all_tool_definitions;
#[allow(unused_imports)]
pub(crate) use openrouter_tools::*;

#[path = "openrouter_context.rs"]
mod openrouter_context;
pub use openrouter_context::{
    build_trimmed_request, estimate_message_tokens, estimate_messages_tokens,
    estimate_tokens, estimate_tools_tokens, format_context_usage, trim_message_list,
    RESPONSE_RESERVE_TOKENS, TOKEN_ESTIMATE_MARGIN,
};

#[path = "openrouter_client.rs"]
mod openrouter_client;
pub use openrouter_client::OpenRouterClient;
#[cfg(test)]
pub(crate) use openrouter_client::truncate_str;

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
