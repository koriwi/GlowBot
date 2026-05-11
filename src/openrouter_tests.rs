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
        tools: Some(all_tool_definitions(true, None, "/media", None)),
        tool_choice: None,
        modalities: None,
        image_config: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("test/model"));
    assert!(json.contains("sys"));
    assert!(json.contains("bash"));
    assert!(json.contains("read_memory"));
    assert!(json.contains("update_memory"));
}

#[test]
fn test_all_tool_definitions_with_bash() {
    let tools = all_tool_definitions(true, None, "/media", None);
    assert_eq!(tools.len(), 21);
    assert_eq!(tools[0].function.name, "bash");
    assert_eq!(tools[1].function.name, "read_memory");
    assert_eq!(tools[2].function.name, "update_memory");
}

#[test]
fn test_all_tool_definitions_without_bash() {
    let tools = all_tool_definitions(false, None, "/media", None);
    assert_eq!(tools.len(), 20);
    assert_eq!(tools[0].function.name, "read_memory");
    assert!(!tools.iter().any(|t| t.function.name == "bash"));
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
fn test_send_media_tool_definition() {
    let def = send_media_tool_definition("/media");
    assert_eq!(def.function.name, "send_media");
    assert!(!def.function.description.is_empty());
    assert!(def.function.description.contains("/media"));
    let params = &def.function.parameters;
    assert_eq!(params["required"][0], "file_path");
    assert!(params["properties"]["file_path"].is_object());
    assert!(params["properties"]["caption"].is_object());
    assert!(params["properties"]["original_quality"].is_object());
    assert_eq!(params["properties"]["original_quality"]["type"], "boolean");
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
        reasoning: None,
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
        modalities: None,
        image_config: None,
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
    // limit unknown, but usage available
    assert_eq!(
        format_context_usage(5000, 0),
        "5k used (context limit unknown)"
    );
    assert_eq!(
        format_context_usage(500, 0),
        "0k used (context limit unknown)"
    );
    // limit unknown, no usage data
    assert_eq!(format_context_usage(0, 0), "no token data yet");
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
    // Architecture defaults to empty when not present in JSON
    assert!(m.architecture.input_modalities.is_empty());
}

#[test]
fn test_deserialize_model_info_with_architecture() {
    let json = serde_json::json!({
        "id": "google/gemini-2.5-flash",
        "context_length": 1048576,
        "architecture": {
            "input_modalities": ["text", "image", "audio", "video"]
        }
    });
    let m: ModelInfo = serde_json::from_value(json).unwrap();
    assert_eq!(m.id, "google/gemini-2.5-flash");
    assert_eq!(m.context_length, 1048576);
    assert_eq!(m.architecture.input_modalities.len(), 4);
    assert!(m.supports_modality("text"));
    assert!(m.supports_modality("image"));
    assert!(m.supports_modality("audio"));
    assert!(m.supports_modality("video"));
    assert!(!m.supports_modality("file"));
}

#[test]
fn test_supports_modality_empty() {
    let m = ModelInfo {
        id: "test/model".into(),
        name: String::new(),
        created: 0,
        context_length: 4096,
        architecture: Default::default(),
        pricing: Default::default(),
    };
    assert!(!m.supports_modality("text"));
    assert!(!m.supports_modality("image"));
    assert!(!m.supports_modality("audio"));
}

#[test]
fn test_model_pricing_is_free() {
    use crate::openrouter::ModelPricing;
    let free = ModelPricing {
        prompt: "0".into(),
        completion: "0".into(),
        request: String::new(),
    };
    assert!(free.is_free());

    let paid = ModelPricing {
        prompt: "0.000001".into(),
        completion: "0.000002".into(),
        request: String::new(),
    };
    assert!(!paid.is_free());
}

#[test]
fn test_model_provider() {
    let m = ModelInfo {
        id: "openai/gpt-4o".into(),
        name: "OpenAI: GPT-4o".into(),
        created: 1715367049,
        context_length: 128000,
        architecture: Default::default(),
        pricing: Default::default(),
    };
    assert_eq!(m.provider(), "openai");
}

#[test]
fn test_model_provider_no_slash() {
    let m = ModelInfo {
        id: "custom-model".into(),
        name: String::new(),
        created: 0,
        context_length: 4096,
        architecture: Default::default(),
        pricing: Default::default(),
    };
    assert_eq!(m.provider(), "custom-model");
}

#[test]
fn test_deserialize_model_with_pricing() {
    let json = serde_json::json!({
        "id": "google/gemini-2.5-flash",
        "context_length": 1048576,
        "pricing": {
            "prompt": "0.0000375",
            "completion": "0.00015"
        }
    });
    let m: ModelInfo = serde_json::from_value(json).unwrap();
    assert_eq!(m.pricing.prompt, "0.0000375");
    assert_eq!(m.pricing.completion, "0.00015");
    assert!(!m.pricing.is_free());
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

// ─── embedding tool tests ──────────────────────────────────────────

#[test]
fn test_all_tool_definitions_with_embedding_model() {
    // With bash + embedding: 17 base + bash + search_conversations + 3 config = 22
    let tools = all_tool_definitions(true, Some("openai/text-embedding-3-small"), "/media", None);
    assert_eq!(tools.len(), 22);
    assert_eq!(tools[0].function.name, "bash");
    assert!(tools
        .iter()
        .any(|t| t.function.name == "search_conversations"));

    // Without bash, with embedding: 17 base + search_conversations + 3 config = 21
    let tools = all_tool_definitions(false, Some("openai/text-embedding-3-small"), "/media", None);
    assert_eq!(tools.len(), 21);
    assert!(tools
        .iter()
        .any(|t| t.function.name == "search_conversations"));
    assert!(!tools.iter().any(|t| t.function.name == "bash"));

    // Without embedding model, without bash: 17 base + 3 config = 20
    let tools = all_tool_definitions(false, None, "/media", None);
    assert_eq!(tools.len(), 20);
    assert!(!tools
        .iter()
        .any(|t| t.function.name == "search_conversations"));
}

#[test]
fn test_search_conversations_tool_definition() {
    let def = search_conversations_tool_definition();
    assert_eq!(def.def_type, "function");
    assert_eq!(def.function.name, "search_conversations");
    assert!(!def.function.description.is_empty());

    let params = &def.function.parameters;
    assert_eq!(params["type"], "object");
    assert!(params["required"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("query")));
    assert!(params["properties"]["query"]["type"] == "string");
    assert!(params["properties"]["count"]["type"] == "integer");
}

#[test]
fn test_embedding_response_deserialization() {
    let json = serde_json::json!({
        "data": [{
            "embedding": [0.1, 0.2, 0.3]
        }]
    });
    let resp: EmbeddingResponse = serde_json::from_value(json).unwrap();
    assert_eq!(resp.data.len(), 1);
    assert_eq!(resp.data[0].embedding, vec![0.1, 0.2, 0.3]);
}

// ─── reasoning / thinking tests ───────────────────────────────────

#[test]
fn test_assistant_message_with_reasoning() {
    let json = serde_json::json!({
        "content": "The answer is 42.",
        "role": "assistant",
        "reasoning": "Let me think step by step..."
    });
    let msg: AssistantMessage = serde_json::from_value(json).unwrap();
    assert_eq!(msg.content.as_deref(), Some("The answer is 42."));
    assert_eq!(
        msg.reasoning.as_deref(),
        Some("Let me think step by step...")
    );
    assert!(msg.tool_calls.is_none());
}

#[test]
fn test_assistant_message_without_reasoning() {
    let json = serde_json::json!({
        "content": "Hello",
        "role": "assistant"
    });
    let msg: AssistantMessage = serde_json::from_value(json).unwrap();
    assert!(msg.reasoning.is_none());
}

#[test]
fn test_assistant_message_with_tool_calls_and_reasoning() {
    let json = serde_json::json!({
        "content": null,
        "role": "assistant",
        "reasoning": "I need to use a tool...",
        "tool_calls": [{
            "id": "call_x",
            "type": "function",
            "function": {
                "name": "bash",
                "arguments": "{\"command\":\"ls\"}"
            }
        }]
    });
    let msg: AssistantMessage = serde_json::from_value(json).unwrap();
    assert_eq!(msg.reasoning.as_deref(), Some("I need to use a tool..."));
    assert!(msg.tool_calls.is_some());
}

#[test]
fn test_chat_message_assistant_with_reasoning() {
    let msg = ChatMessage::assistant_with_reasoning("Hello", "thinking...".into());
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.text_content(), "Hello");
    assert_eq!(msg.reasoning.as_deref(), Some("thinking..."));
}

#[test]
fn test_chat_message_assistant_tool_calls_with_reasoning() {
    let tc = ToolCall {
        id: "t1".into(),
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: "{}".into(),
        },
    };
    let msg = ChatMessage::assistant_tool_calls_with_reasoning(vec![tc], "considering...".into());
    assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    assert_eq!(msg.reasoning.as_deref(), Some("considering..."));
    assert!(msg.text_content().is_empty());
}

#[test]
fn test_estimate_tokens_with_reasoning() {
    let msg = ChatMessage::assistant_with_reasoning("ok", "a".repeat(400));
    let tokens = estimate_message_tokens(&msg);
    // 4 (role) + 1 ("ok" ≈ 1 token) + 100 (400 chars / 4) = ~105
    assert!(
        tokens >= 100 && tokens <= 110,
        "unexpected tokens: {}",
        tokens
    );
}

#[test]
fn test_estimate_tokens_without_reasoning() {
    let msg = ChatMessage::assistant("ok");
    let tokens = estimate_message_tokens(&msg);
    assert_eq!(tokens, 5); // 4 (role) + 1 ("ok" ≈ 1 token)
}

#[test]
fn test_chat_message_reasoning_serialization() {
    let msg = ChatMessage::assistant_with_reasoning("result", "step by step...".into());
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("step by step..."));
    assert!(json.contains("reasoning"));
    assert!(json.contains("result"));
}

#[test]
fn test_chat_message_no_reasoning_serialization() {
    let msg = ChatMessage::assistant("hello");
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("reasoning"));
}

#[test]
fn test_embedding_response_empty_errors() {
    let json = serde_json::json!({
        "data": []
    });
    let resp: EmbeddingResponse = serde_json::from_value(json).unwrap();
    assert!(resp.data.is_empty());
}

#[test]
fn test_truncate_str() {
    assert_eq!(truncate_str("hi", 5), "hi");
    assert_eq!(truncate_str("hello", 5), "hello");
    assert_eq!(truncate_str("hello world", 5), "hello...");
    assert_eq!(truncate_str("", 5), "");
    // Multi-byte boundary: takes first 3 chars, not bytes
    assert_eq!(truncate_str("héllo world", 3), "hél...");
}

#[test]
fn test_normalize_model_id() {
    // Without provider suffix — unchanged
    assert_eq!(normalize_model_id("deepseek/deepseek-v4-pro"), "deepseek/deepseek-v4-pro");
    assert_eq!(normalize_model_id("openai/gpt-4o"), "openai/gpt-4o");
    // With provider suffix — stripped
    assert_eq!(normalize_model_id("deepseek/deepseek-v4-pro:deepseek"), "deepseek/deepseek-v4-pro");
    assert_eq!(normalize_model_id("openai/gpt-4o:nitro"), "openai/gpt-4o");
    // Multiple colons — only last suffix stripped
    assert_eq!(normalize_model_id("openai/o3-mini-high:nitro"), "openai/o3-mini-high");
}

// ─── strip_orphaned_tool_results tests ─────────────────────────────

fn tool_call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: "{}".into(),
        },
    }
}

#[test]
fn test_strip_orphaned_empty() {
    let msgs: Vec<ChatMessage> = vec![];
    let result = strip_orphaned_tool_results(&msgs);
    assert!(result.is_empty());
}

#[test]
fn test_strip_orphaned_no_orphans() {
    // Normal sequence: user, assistant_tool_calls, tool_result, assistant
    let msgs = vec![
        ChatMessage::user("hi"),
        ChatMessage::assistant_tool_calls(vec![tool_call("t1")]),
        ChatMessage::tool_result("t1", "result"),
        ChatMessage::assistant("done"),
    ];
    let result = strip_orphaned_tool_results(&msgs);
    assert_eq!(result.len(), 4);
    assert_eq!(result[0].role, "user");
}

#[test]
fn test_strip_orphaned_leading_tool_result() {
    // Orphan: tool_result without preceding tool_calls at start
    let msgs = vec![
        ChatMessage::tool_result("t1", "result"),
        ChatMessage::tool_result("t2", "result2"),
        ChatMessage::user("hey"),
    ];
    let result = strip_orphaned_tool_results(&msgs);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].role, "user");
}

#[test]
fn test_strip_orphaned_all_orphans() {
    // Only orphaned tool_results
    let msgs = vec![
        ChatMessage::tool_result("t1", "r1"),
        ChatMessage::tool_result("t2", "r2"),
    ];
    let result = strip_orphaned_tool_results(&msgs);
    assert!(result.is_empty());
}

#[test]
fn test_strip_orphaned_user_first() {
    // User message first, then orphaned tool_result — orphan is stripped
    let msgs = vec![
        ChatMessage::user("hi"),
        ChatMessage::tool_result("t1", "r1"), // orphaned — no preceding tool_calls
    ];
    let result = strip_orphaned_tool_results(&msgs);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].role, "user");
}

#[test]
fn test_strip_orphaned_assistant_no_tool_calls_first() {
    // Assistant without tool_calls first, then orphaned tool_result
    let msgs = vec![
        ChatMessage::assistant("hello"),
        ChatMessage::tool_result("t1", "r1"), // orphaned — no tool_calls opened
    ];
    let result = strip_orphaned_tool_results(&msgs);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].role, "assistant");
}

#[test]
fn test_strip_orphaned_tool_calls_first() {
    // Assistant with tool_calls first, then matching result, then orphaned result
    let msgs = vec![
        ChatMessage::assistant_tool_calls(vec![tool_call("t1")]),
        ChatMessage::tool_result("t1", "r1"), // matches t1 — kept
        ChatMessage::tool_result("t2", "r2"), // orphaned (t2 not opened) — stripped
    ];
    let result = strip_orphaned_tool_results(&msgs);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].role, "assistant");
    assert!(result[0].tool_calls.is_some());
    assert_eq!(result[1].role, "tool");
    assert_eq!(result[1].tool_call_id.as_deref(), Some("t1"));
}

#[test]
fn test_strip_orphaned_multiple_leading_orphans_then_valid() {
    // Multiple orphan tool_results, then a system message
    let msgs = vec![
        ChatMessage::tool_result("t1", "r1"),
        ChatMessage::tool_result("t2", "r2"),
        ChatMessage::tool_result("t3", "r3"),
        ChatMessage::system("system prompt excerpt"),
    ];
    let result = strip_orphaned_tool_results(&msgs);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].role, "system");
}

#[test]
fn test_build_trimmed_request_orphan_stripping() {
    // Simulate a tight budget where assistant_tool_calls is too expensive
    // but the following tool_result fits -> would create orphaned tool_result.
    // After our fix, the orphaned tool_result is stripped.
    let head = vec![ChatMessage::system("sys")];
    let history = vec![
        ChatMessage::user("hello"),
        ChatMessage::assistant_tool_calls(vec![tool_call("t1")]),
        ChatMessage::tool_result("t1", "ok"),
        ChatMessage::user("world"),
    ];
    let turn = vec![ChatMessage::user("current")];
    let tools: Vec<ToolDefinition> = vec![];

    // Use a context limit where the budget fits user("hello") + tool_result("ok")
    // + user("world") but NOT assistant_tool_calls (which has JSON overhead).
    //
    // Estimates:
    //   head: system("sys")  = 4 + ⌈3/4⌉ = 5
    //   turn: user("current") = 4 + ⌈7/4⌉ = 6
    //   tools: 0
    //   fixed = 5 + 0 + 6 + 8192 = 8203
    //   user("hello")  = 4 + ⌈5/4⌉ = 6
    //   assistant_tool_calls = 4 + 0 + JSON(~80 chars / 4) ≈ 24
    //   tool_result("ok") = 4 + ⌈2/4⌉ + 4 = 9
    //   user("world")  = 4 + ⌈5/4⌉ = 6
    //   total history = 6 + 24 + 9 + 6 = 45
    //
    // We need budget ≥ 21 (user+tool_res+user) but < 30 (so assistant_tool_calls
    // at position 2 gets skipped while tool_result at position 3 fits).
    // So: effective_limit = 8203 + 21..30 = 8224..8233
    //     context_limit = effective_limit / 0.75 ≈ 10965..10977
    let context_limit: u64 = 10970;

    let (result, trimmed) =
        build_trimmed_request(context_limit, &head, &history, &turn, &tools);

    // The assistant_tool_calls should be skipped (too expensive), and the
    // orphaned tool_result("ok") should be stripped. Expected:
    // [system, user("hello"), user("world"), user("current")]
    assert!(trimmed, "expected trimming to occur");
    assert_eq!(result.len(), 4);
    assert_eq!(result[0].role, "system");
    assert_eq!(result[1].role, "user");
    assert_eq!(result[1].text_content(), "hello");
    assert_eq!(result[2].role, "user");
    assert_eq!(result[2].text_content(), "world");
    assert_eq!(result[3].role, "user");
    assert_eq!(result[3].text_content(), "current");
    // Verify no tool messages made it through
    assert!(result.iter().all(|m| m.role != "tool"));
}

#[test]
fn test_trim_message_list_orphan_stripping() {
    // Simulate heartbeat trimming where the middle is dropped and the
    // preserved suffix starts with orphaned tool_results.
    let msgs = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("task"),
        ChatMessage::assistant_tool_calls(vec![tool_call("t1")]),
        ChatMessage::tool_result("t1", "step 1"),
        ChatMessage::tool_result("t2", "step 2"),  // orphaned t2 after middle drop
    ];
    // preserve_prefix=2 (sys + task), preserve_suffix=2 (last 2 messages)
    // Middle dropped: assistant_tool_calls + tool_result("t1")
    // Suffix: [tool_result("t2", "step 2")] — orphaned!
    // After fix: orphaned tool_result stripped, suffix is empty.
    let result = trim_message_list(&msgs, 2, 1);
    // Expected: [sys, task, placeholder] — orphaned tool_result stripped
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].role, "system");
    assert_eq!(result[1].role, "user");
    assert_eq!(result[1].text_content(), "task");
    assert_eq!(result[2].role, "system"); // placeholder
    assert!(result[2].text_content().contains("omitted"));
    // No tool messages
    assert!(result.iter().all(|m| m.role != "tool"));
}

// --- Multimodal message tests ---

#[test]
fn test_user_multimodal_with_image() {
    let msg = ChatMessage::user_multimodal(vec![
        ContentPart::Text {
            text: "Look at this".into(),
        },
        ContentPart::ImageUrl {
            image_url: ImageUrlDetail {
                url: "data:image/jpeg;base64,abc".into(),
                detail: None,
            },
        },
    ]);
    assert_eq!(msg.role, "user");
    assert!(msg.name.is_none());
    // Verify serialization
    let json = serde_json::to_value(&msg.content).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["type"], "text");
    assert_eq!(arr[1]["type"], "image_url");
    assert_eq!(arr[1]["image_url"]["url"], "data:image/jpeg;base64,abc");
}

#[test]
fn test_user_multimodal_with_audio() {
    let msg = ChatMessage::user_multimodal(vec![
        ContentPart::InputAudio {
            input_audio: InputAudioDetail {
                data: "base64data".into(),
                format: "ogg".into(),
            },
        },
        ContentPart::Text {
            text: "Transcribe this".into(),
        },
    ]);
    assert_eq!(msg.role, "user");
    let json = serde_json::to_value(&msg.content).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr[0]["type"], "input_audio");
    assert_eq!(arr[0]["input_audio"]["data"], "base64data");
    assert_eq!(arr[1]["type"], "text");
}

#[test]
fn test_user_multimodal_with_name() {
    let msg = ChatMessage::user_multimodal_with_name(
        vec![ContentPart::Text {
            text: "hi".into(),
        }],
        "alice",
    );
    assert_eq!(msg.name.unwrap(), "alice");
    assert_eq!(msg.role, "user");
}

#[test]
fn test_text_content_with_image_placeholder() {
    let msg = ChatMessage::user_multimodal(vec![
        ContentPart::Text {
            text: "before".into(),
        },
        ContentPart::ImageUrl {
            image_url: ImageUrlDetail {
                url: "data:image/jpeg;base64,abc".into(),
                detail: None,
            },
        },
        ContentPart::Text {
            text: "after".into(),
        },
    ]);
    assert_eq!(msg.text_content(), "before[image]after");
}

// ─── image generation tests ────────────────────────────────────────

#[test]
fn test_generate_image_tool_definition() {
    let def = generate_image_tool_definition("/media");
    assert_eq!(def.def_type, "function");
    assert_eq!(def.function.name, "generate_image");
    assert!(def.function.description.contains("Generate"));
    assert!(def.function.description.contains("/media"));

    let params = &def.function.parameters;
    assert_eq!(params["type"], "object");
    let required = params["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0], "prompt");
    assert_eq!(params["properties"]["prompt"]["type"], "string");
    assert_eq!(params["properties"]["size"]["type"], "string");
    assert!(params["properties"]["reference_images"]["type"] == "array");
}

#[test]
fn test_all_tool_definitions_with_image_gen_model() {
    // With image_gen_model, without bash, without embedding: 17 base + generate_image + 3 config = 21
    let tools = all_tool_definitions(false, None, "/media", Some("black-forest-labs/flux-1.1-pro"));
    assert_eq!(tools.len(), 21);
    assert!(tools.iter().any(|t| t.function.name == "generate_image"));

    // With image_gen_model, with bash, without embedding: 17 base + generate_image + bash + 3 config = 22
    let tools = all_tool_definitions(true, None, "/media", Some("black-forest-labs/flux-1.1-pro"));
    assert_eq!(tools.len(), 22);
    assert_eq!(tools[0].function.name, "bash");
    assert!(tools.iter().any(|t| t.function.name == "generate_image"));

    // Without image_gen_model: generate_image is excluded
    let tools = all_tool_definitions(true, None, "/media", None);
    assert_eq!(tools.len(), 21);
    assert!(!tools.iter().any(|t| t.function.name == "generate_image"));
}

#[test]
fn test_chat_completion_request_with_modalities() {
    let req = ChatCompletionRequest {
        model: "google/gemini-2.5-flash-image".into(),
        messages: vec![ChatMessage::user("a cat")],
        tools: None,
        tool_choice: None,
        modalities: Some(vec!["image".into()]),
        image_config: Some(ImageConfig {
            aspect_ratio: Some("16:9".into()),
            image_size: Some("2K".into()),
        }),
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("modalities"));
    assert!(json.contains("image"));
    assert!(json.contains("image_config"));
    assert!(json.contains("aspect_ratio"));
    assert!(json.contains("16:9"));
    assert!(json.contains("image_size"));
    assert!(json.contains("2K"));
}

#[test]
fn test_chat_completion_request_without_modalities() {
    let req = ChatCompletionRequest {
        model: "test/model".into(),
        messages: vec![ChatMessage::user("hi")],
        tools: None,
        tool_choice: None,
        modalities: None,
        image_config: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("modalities"));
    assert!(!json.contains("image_config"));
}

#[test]
fn test_assistant_message_with_images() {
    let json = serde_json::json!({
        "content": "Here's your image",
        "role": "assistant",
        "images": [
            {
                "type": "image_url",
                "image_url": {
                    "url": "data:image/png;base64,iVBORw0KGgo="
                }
            }
        ]
    });
    let msg: AssistantMessage = serde_json::from_value(json).unwrap();
    assert_eq!(msg.content.as_deref(), Some("Here's your image"));
    let imgs = msg.images.unwrap();
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].image_url.url, "data:image/png;base64,iVBORw0KGgo=");
    assert_eq!(imgs[0].image_type.as_deref(), Some("image_url"));
}

#[test]
fn test_assistant_message_without_images() {
    let json = serde_json::json!({
        "content": "Hello",
        "role": "assistant"
    });
    let msg: AssistantMessage = serde_json::from_value(json).unwrap();
    assert!(msg.images.is_none());
}

#[test]
fn test_image_config_aspect_ratio() {
    let config = ImageConfig {
        aspect_ratio: Some("16:9".into()),
        image_size: None,
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("aspect_ratio"));
    assert!(json.contains("16:9"));
    assert!(!json.contains("image_size"));
}

#[test]
fn test_text_content_with_audio_placeholder() {
    let msg = ChatMessage::user_multimodal(vec![
        ContentPart::InputAudio {
            input_audio: InputAudioDetail {
                data: "x".into(),
                format: "ogg".into(),
            },
        },
        ContentPart::Text {
            text: "text".into(),
        },
    ]);
    assert_eq!(msg.text_content(), "[audio]text");
}
