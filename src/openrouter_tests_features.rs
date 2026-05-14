// ─── embedding tool tests ──────────────────────────────────────────

#[test]
fn test_all_tool_definitions_with_embedding_model() {
    // With bash + embedding: 21 base + bash + search_conversations + 3 config + 2 model tools = 28
    let tools = all_tool_definitions(true, Some("openai/text-embedding-3-small"), "/media", None, None);
    assert_eq!(tools.len(), 28);
    assert_eq!(tools[0].function.name, "bash");
    assert!(tools
        .iter()
        .any(|t| t.function.name == "search_conversations"));

    // Without bash, with embedding: 21 base + search_conversations + 3 config + 2 model tools = 27
    let tools = all_tool_definitions(false, Some("openai/text-embedding-3-small"), "/media", None, None);
    assert_eq!(tools.len(), 27);
    assert!(tools
        .iter()
        .any(|t| t.function.name == "search_conversations"));
    assert!(!tools.iter().any(|t| t.function.name == "bash"));

    // Without embedding model, without bash: 21 base + 3 config + 2 model tools = 26
    let tools = all_tool_definitions(false, None, "/media", None, None);
    assert_eq!(tools.len(), 26);
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
fn test_truncate_str_trims_whitespace() {
    // Prevents empty log lines when the body is mostly whitespace
    // (e.g. binary image data interpreted as string).
    assert_eq!(truncate_str("  \n  \n  ", 5), "");
    assert_eq!(truncate_str("\n\n  hello  \n", 5), "hello");
    assert_eq!(truncate_str("  \n  abcdefgh  \n", 5), "abcde...");
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

#[test]
fn test_apply_specifier() {
    // No existing specifier — appends
    assert_eq!(apply_specifier("openai/gpt-4o", "nitro"), "openai/gpt-4o:nitro");
    assert_eq!(apply_specifier("deepseek/deepseek-chat", "floor"), "deepseek/deepseek-chat:floor");
    assert_eq!(apply_specifier("google/gemini-2.5-pro", "free"), "google/gemini-2.5-pro:free");
    // Replaces existing specifier
    assert_eq!(apply_specifier("openai/gpt-4o:nitro", "floor"), "openai/gpt-4o:floor");
    assert_eq!(apply_specifier("deepseek/deepseek-chat:floor", "nitro"), "deepseek/deepseek-chat:nitro");
    assert_eq!(apply_specifier("openai/gpt-4o:free", "nitro"), "openai/gpt-4o:nitro");
    // Replaces provider routing suffix
    assert_eq!(apply_specifier("deepseek/deepseek-chat:deepseek", "free"), "deepseek/deepseek-chat:free");
    assert_eq!(apply_specifier("openai/gpt-4o:openai", "nitro"), "openai/gpt-4o:nitro");
}

