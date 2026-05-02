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
        update_skill_tool_definition(),
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

/// A response from OpenRouter's chat completions API.
#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<Choice>,
}

/// A choice in the chat completion response.
#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: AssistantMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// The assistant message from the API.
#[derive(Debug, Deserialize)]
pub struct AssistantMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    pub role: Option<String>,
}

/// OpenRouter API client.
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
        assert_eq!(tools.len(), 7);
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
