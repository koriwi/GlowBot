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
fn test_chat_completion_response_missing_choices() {
    // OpenRouter sometimes returns error responses without a `choices` field.
    let json = serde_json::json!({
        "error": {"message": "Provider returned error", "code": 400}
    });
    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    assert!(resp.choices.is_empty());
    assert!(resp.usage.is_none());
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
fn test_deserialize_usage_floats() {
    // Some OpenRouter providers emit token counts as floats (e.g. 10813.44).
    let json = serde_json::json!({
        "prompt_tokens": 10813.44,
        "completion_tokens": 289.0,
        "total_tokens": 11102.44
    });
    let u: Usage = serde_json::from_value(json).unwrap();
    assert_eq!(u.prompt_tokens, 10813);
    assert_eq!(u.completion_tokens, 289);
    assert_eq!(u.total_tokens, 11102);
}

#[test]
fn test_deserialize_usage_null_fields() {
    let json = serde_json::json!({
        "prompt_tokens": null,
        "completion_tokens": 42,
        "total_tokens": 42
    });
    let u: Usage = serde_json::from_value(json).unwrap();
    assert_eq!(u.prompt_tokens, 0);
    assert_eq!(u.completion_tokens, 42);
}

#[test]
fn test_deserialize_usage_missing_fields() {
    let json = serde_json::json!({});
    let u: Usage = serde_json::from_value(json).unwrap();
    assert_eq!(u.prompt_tokens, 0);
    assert_eq!(u.completion_tokens, 0);
    assert_eq!(u.total_tokens, 0);
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

