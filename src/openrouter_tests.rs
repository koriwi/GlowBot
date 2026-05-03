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
