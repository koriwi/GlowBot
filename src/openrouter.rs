use serde::{Deserialize, Serialize};

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
        }
    }

    pub fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: ChatContent::Text(content.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user_with_name(content: &str, name: &str) -> Self {
        Self {
            role: "user".into(),
            content: ChatContent::Text(content.to_string()),
            name: Some(name.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".into(),
            content: ChatContent::Text(content.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: &str, content: &str) -> Self {
        Self {
            role: "tool".into(),
            content: ChatContent::Text(content.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.to_string()),
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

/// The bash tool definition.
pub fn bash_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "bash".into(),
            description: "Execute a bash command in the container. Use for file operations, API calls, invoking skills, and reading raw files. Commands are stateless and oneshot.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to execute."
                    }
                },
                "required": ["command"]
            }),
        },
    }
}

/// The read_memory tool definition.
pub fn read_memory_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "read_memory".into(),
            description: "Read a user's memory file. Returns the full memory as JSON with frontmatter fields (user_id, username, call_name, description) and body (log entries).".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "user_id": {
                        "type": "string",
                        "description": "The Telegram user ID whose memory to read."
                    }
                },
                "required": ["user_id"]
            }),
        },
    }
}

/// The update_memory tool definition.
pub fn update_memory_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "update_memory".into(),
            description: "Update a user's memory file. All fields are optional — only provided fields are overwritten. Use log_entry to append a timestamped line to the body. Use call_name to set what you should call them. Use description to update your summary of the user.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "user_id": {
                        "type": "string",
                        "description": "The Telegram user ID whose memory to update."
                    },
                    "username": {
                        "type": "string",
                        "description": "Optional: update the Telegram @username."
                    },
                    "call_name": {
                        "type": "string",
                        "description": "Optional: update what you call this user."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional: update the summary/description of this user."
                    },
                    "log_entry": {
                        "type": "string",
                        "description": "Optional: a fact or event to append as a timestamped log entry."
                    }
                },
                "required": ["user_id"]
            }),
        },
    }
}

/// All tool definitions.
pub fn all_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        bash_tool_definition(),
        read_memory_tool_definition(),
        update_memory_tool_definition(),
        read_chat_memory_tool_definition(),
        update_chat_memory_tool_definition(),
        create_skill_tool_definition(),
        read_skill_tool_definition(),
        update_skill_tool_definition(),
        add_task_tool_definition(),
        list_tasks_tool_definition(),
        remove_task_tool_definition(),
        get_recent_messages_tool_definition(),
        send_message_tool_definition(),
    ]
}

/// The read_chat_memory tool definition.
pub fn read_chat_memory_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "read_chat_memory".into(),
            description: "Read the chat-level memory for this conversation. Returns JSON with call_name, description, and body (log entries). Use this to recall context about the chat itself — topics, participants, group purpose, etc.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// The update_chat_memory tool definition.
pub fn update_chat_memory_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "update_chat_memory".into(),
            description: "Update the chat-level memory. All fields optional — only provided fields are overwritten. Use call_name to name the chat, description to summarize it, log_entry to append a timestamped fact.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "call_name": {
                        "type": "string",
                        "description": "Optional: a name for this chat (e.g. 'Rust Study Group')."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional: summary of what this chat is about."
                    },
                    "log_entry": {
                        "type": "string",
                        "description": "Optional: a fact to append as a timestamped log entry."
                    }
                },
                "required": []
            }),
        },
    }
}

/// The create_skill tool definition.
pub fn create_skill_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "create_skill".into(),
            description: "Create a new skill. Skills extend my capabilities with shell commands, API calls, or custom workflows. Each skill gets a directory under skills/<name>/ with a skill.md file.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique skill name (lowercase, hyphens for spaces, e.g. 'search-web')."
                    },
                    "description": {
                        "type": "string",
                        "description": "Short description of what the skill does."
                    },
                    "body": {
                        "type": "string",
                        "description": "Instructions for using the skill — bash commands, API endpoints, workflow steps. Gets injected into my system prompt."
                    }
                },
                "required": ["name", "description", "body"]
            }),
        },
    }
}

/// The read_skill tool definition.
pub fn read_skill_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "read_skill".into(),
            description: "Read an existing skill's full content. Returns JSON with name, description, and body. Use before updating a skill so you know what's already there.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to read."
                    }
                },
                "required": ["name"]
            }),
        },
    }
}

/// The update_skill tool definition.
pub fn update_skill_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "update_skill".into(),
            description: "Update an existing skill. Only provided fields are overwritten. Skills are reloaded automatically after updating.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the existing skill to update."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional: new description."
                    },
                    "body": {
                        "type": "string",
                        "description": "Optional: new body/instructions."
                    }
                },
                "required": ["name"]
            }),
        },
    }
}

/// The add_task tool definition.
pub fn add_task_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "add_task".into(),
            description: "Add a task to this chat's task list. The bot will work on tasks autonomously on a heartbeat timer.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "What needs to be done. Be specific and actionable."
                    }
                },
                "required": ["description"]
            }),
        },
    }
}

/// The list_tasks tool definition.
pub fn list_tasks_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "list_tasks".into(),
            description: "List all pending tasks for this chat.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// The remove_task tool definition.
pub fn remove_task_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "remove_task".into(),
            description: "Remove a completed or obsolete task from this chat's task list.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The ID of the task to remove (from list_tasks)."
                    }
                },
                "required": ["id"]
            }),
        },
    }
}

/// The get_recent_messages tool definition.
pub fn get_recent_messages_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "get_recent_messages".into(),
            description: "Get the last N messages from this conversation. Call this when you need to recall something from earlier in the chat. Returns a JSON array of recent messages with role, content, and sender name.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "count": {
                        "type": "integer",
                        "description": "Number of recent messages to retrieve (default: 10, max: 50)"
                    }
                },
                "required": []
            }),
        },
    }
}

/// The send_message tool definition.
pub fn send_message_tool_definition() -> ToolDefinition {
    ToolDefinition {
        def_type: "function".into(),
        function: FunctionDef {
            name: "send_message".into(),
            description: "Send a plain text message to the current chat. In normal conversations, use this ONLY for headsup/intermediate messages (e.g. 'ok, give me a second, taking a look now...') — never for your final answer, which is sent automatically. In background tasks, use this once to report completion or deliver results. Use sparingly.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The plain text message to send."
                    }
                },
                "required": ["text"]
            }),
        },
    }
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
/// Counts role overhead (~4 tokens) plus content text tokens.
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
    total
}

/// Estimate tokens for a slice of messages.
pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> u64 {
    messages.iter().map(|m| estimate_message_tokens(m)).sum()
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
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_system() {
        let msg = ChatMessage::system("You are a bot.");
        assert_eq!(msg.role, "system");
        assert_eq!(msg.text_content(), "You are a bot.");
        assert!(msg.tool_calls.is_none());
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn test_chat_message_user() {
        let msg = ChatMessage::user("Hello!");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.text_content(), "Hello!");
    }

    #[test]
    fn test_chat_message_assistant_tool_calls() {
        let tc = ToolCall {
            id: "call_1".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "bash".into(),
                arguments: r#"{"command":"echo hi"}"#.into(),
            },
        };
        let msg = ChatMessage::assistant_tool_calls(vec![tc]);
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_chat_message_tool_result() {
        let msg = ChatMessage::tool_result("call_1", "result");
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(msg.text_content(), "result");
    }

    #[test]
    fn test_chat_message_user_with_name() {
        let msg = ChatMessage::user_with_name("Hi", "John");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.name.as_deref(), Some("John"));
    }

    #[test]
    fn test_bash_tool_definition() {
        let def = bash_tool_definition();
        assert_eq!(def.def_type, "function");
        assert_eq!(def.function.name, "bash");
        assert!(!def.function.description.is_empty());
    }

    #[test]
    fn test_tool_call_serialization() {
        let tc = ToolCall {
            id: "call_abc".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "bash".into(),
                arguments: r#"{"command":"ls"}"#.into(),
            },
        };
        let json = serde_json::to_string(&tc).unwrap();
        let parsed: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "call_abc");
        assert_eq!(parsed.function.name, "bash");
    }

    #[test]
    fn test_chat_completion_request_seialization() {
        let req = ChatCompletionRequest {
            model: "test/model".into(),
            messages: vec![ChatMessage::system("sys"), ChatMessage::user("hi")],
            tools: Some(all_tool_definitions()),
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("test/model"));
        assert!(json.contains("sys"));
        assert!(json.contains("bash"));
        assert!(json.contains("read_memory"));
        assert!(json.contains("update_memory"));
    }

    #[test]
    fn test_all_tool_definitions() {
        let tools = all_tool_definitions();
        assert_eq!(tools.len(), 13);
        assert_eq!(tools[0].function.name, "bash");
        assert_eq!(tools[1].function.name, "read_memory");
        assert_eq!(tools[2].function.name, "update_memory");
    }

    #[test]
    fn test_read_memory_tool_definition() {
        let def = read_memory_tool_definition();
        assert_eq!(def.function.name, "read_memory");
        assert!(!def.function.description.is_empty());
    }

    #[test]
    fn test_update_memory_tool_definition() {
        let def = update_memory_tool_definition();
        assert_eq!(def.function.name, "update_memory");
        assert!(!def.function.description.is_empty());
    }

    #[test]
    fn test_chat_message_assistant_empty_text() {
        let msg = ChatMessage::assistant("");
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.text_content(), "");
    }

    #[test]
    fn test_chat_message_text_content_with_parts() {
        let msg = ChatMessage {
            role: "assistant".into(),
            content: ChatContent::Parts(vec![
                ContentPart::Text {
                    text: "Part1 ".into(),
                },
                ContentPart::Text {
                    text: "Part2".into(),
                },
            ]),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        assert_eq!(msg.text_content(), "Part1 Part2");
    }

    #[test]
    fn test_chat_completion_response_deserialization() {
        let json = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "Test response",
                    "role": "assistant"
                },
                "finish_reason": "stop"
            }]
        });
        let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(
            resp.choices[0].message.content.as_deref(),
            Some("Test response")
        );
        assert_eq!(resp.choices[0].finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn test_chat_completion_response_with_tool_calls() {
        let json = serde_json::json!({
            "choices": [{
                "message": {
                    "content": null,
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": "{\"command\":\"ls\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.choices.len(), 1);
        let tc = resp.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0].id, "call_1");
        assert_eq!(tc[0].function.name, "bash");
    }

    #[test]
    fn test_tool_call_deserialization() {
        let json = serde_json::json!({
            "id": "abc123",
            "type": "function",
            "function": {
                "name": "test_fn",
                "arguments": "{\"key\":\"value\"}"
            }
        });
        let tc: ToolCall = serde_json::from_value(json).unwrap();
        assert_eq!(tc.id, "abc123");
        assert_eq!(tc.call_type, "function");
        assert_eq!(tc.function.name, "test_fn");
        assert_eq!(tc.function.arguments, "{\"key\":\"value\"}");
    }

    #[test]
    fn test_chat_content_serialization() {
        let text = ChatContent::Text("hello".into());
        let json = serde_json::to_string(&text).unwrap();
        assert_eq!(json, "\"hello\"");

        let parts = ChatContent::Parts(vec![ContentPart::Text {
            text: "world".into(),
        }]);
        let json = serde_json::to_string(&parts).unwrap();
        assert!(json.contains("type"));
        assert!(json.contains("text"));
        assert!(json.contains("world"));
    }

    #[test]
    fn test_chat_completion_request_with_tool_choice() {
        let req = ChatCompletionRequest {
            model: "m".into(),
            messages: vec![],
            tools: None,
            tool_choice: Some("auto".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("tool_choice"));
        assert!(json.contains("auto"));
    }

    #[test]
    fn test_chat_message_assistant() {
        let msg = ChatMessage::assistant("Hello");
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.text_content(), "Hello");
    }

    #[test]
    fn test_chat_message_assistant_tool_calls_with_no_content() {
        let tc = ToolCall {
            id: "t1".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "f".into(),
                arguments: "{}".into(),
            },
        };
        let msg = ChatMessage::assistant_tool_calls(vec![tc]);
        assert!(msg.text_content().is_empty());
        assert_eq!(msg.tool_calls.unwrap().len(), 1);
    }

    #[test]
    fn test_deserialize_response_no_role() {
        let json = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "no role"
                }
            }]
        });
        let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.choices[0].message.role, None);
    }

    #[test]
    fn test_format_context_usage() {
        assert_eq!(format_context_usage(37000, 252000), "37k/252k (15%)");
        assert_eq!(format_context_usage(0, 100000), "0k/100k (0%)");
        assert_eq!(format_context_usage(1000, 10000), "1k/10k (10%)");
        assert_eq!(format_context_usage(999, 1000), "0k/1k (100%)");
        assert_eq!(format_context_usage(500, 0), "unknown");
    }

    #[test]
    fn test_deserialize_usage() {
        let json = serde_json::json!({
            "prompt_tokens": 1234,
            "completion_tokens": 56,
            "total_tokens": 1290
        });
        let u: Usage = serde_json::from_value(json).unwrap();
        assert_eq!(u.prompt_tokens, 1234);
        assert_eq!(u.completion_tokens, 56);
        assert_eq!(u.total_tokens, 1290);
    }

    #[test]
    fn test_deserialize_model_info() {
        let json = serde_json::json!({
            "id": "anthropic/claude-sonnet-4",
            "context_length": 200000
        });
        let m: ModelInfo = serde_json::from_value(json).unwrap();
        assert_eq!(m.id, "anthropic/claude-sonnet-4");
        assert_eq!(m.context_length, 200000);
    }

    #[test]
    fn test_deserialize_response_with_usage() {
        let json = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "hi",
                    "role": "assistant"
                }
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 10,
                "total_tokens": 110
            }
        });
        let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
        let usage = resp.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 10);
    }

    #[test]
    fn test_deserialize_response_without_usage() {
        let json = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "hi",
                    "role": "assistant"
                }
            }]
        });
        let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
        assert!(resp.usage.is_none());
    }

    #[test]
    fn test_deserialize_tool_call_invalid_args() {
        // Test that we handle non-JSON arguments gracefully
        let json = serde_json::json!({
            "id": "x",
            "type": "function",
            "function": {
                "name": "bash",
                "arguments": "not-json"
            }
        });
        let tc: ToolCall = serde_json::from_value(json).unwrap();
        assert_eq!(tc.function.arguments, "not-json");
    }
}
