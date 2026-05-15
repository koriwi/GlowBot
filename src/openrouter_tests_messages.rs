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
        tools: Some(all_tool_definitions(true, None, "/media", None, None, None)),
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
    let tools = all_tool_definitions(true, None, "/media", None, None, None);
    assert_eq!(tools.len(), 27);
    assert_eq!(tools[0].function.name, "bash");
    assert_eq!(tools[1].function.name, "read_memory");
    assert_eq!(tools[2].function.name, "update_memory");
}

#[test]
fn test_all_tool_definitions_without_bash() {
    let tools = all_tool_definitions(false, None, "/media", None, None, None);
    assert_eq!(tools.len(), 26);
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

