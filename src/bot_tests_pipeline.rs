use super::bot_pipeline::prepare_messages_for_storage;
use crate::config::DatabaseConfig;

// --- prepare_messages_for_storage tests ---

#[test]
fn test_all_messages_preserved_by_default() {
    let msgs = vec![
        ChatMessage::user("hello"),
        ChatMessage::assistant_tool_calls(vec![ToolCall {
            id: "1".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "bash".into(),
                arguments: "echo hi".into(),
            },
        }]),
        ChatMessage::tool_result("1", "output"),
        ChatMessage::assistant("response"),
    ];
    let config = DatabaseConfig::default();
    let result = prepare_messages_for_storage(&msgs, &config);
    // All messages preserved — no filtering
    assert_eq!(result.len(), 4);
    assert_eq!(result[0].role, "user");
    assert_eq!(result[1].role, "assistant");
    assert!(result[1].tool_calls.is_some());
    assert_eq!(result[2].role, "tool");
    assert_eq!(result[3].role, "assistant");
}

#[test]
fn test_tool_max_content_len_truncates_tool_result() {
    let msgs = vec![ChatMessage::tool_result("1", &"a".repeat(100))];
    let config = DatabaseConfig {
        tool_max_content_len: Some(10),
        ..DatabaseConfig::default()
    };
    let result = prepare_messages_for_storage(&msgs, &config);
    assert_eq!(result.len(), 1);
    let content = match &result[0].content {
        crate::openrouter::ChatContent::Text(s) => s,
        _ => panic!("expected text content"),
    };
    assert!(content.len() <= 13); // 10 chars + "..."
    assert!(content.ends_with("..."));
}

#[test]
fn test_tool_max_content_len_truncates_tool_call_arguments() {
    let msgs = vec![ChatMessage::assistant_tool_calls(vec![ToolCall {
        id: "1".into(),
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: "a".repeat(200),
        },
    }])];
    let config = DatabaseConfig {
        tool_max_content_len: Some(10),
        ..DatabaseConfig::default()
    };
    let result = prepare_messages_for_storage(&msgs, &config);
    assert_eq!(result.len(), 1);
    let tcs = result[0].tool_calls.as_ref().unwrap();
    let args = &tcs[0].function.arguments;
    assert!(args.len() <= 13); // 10 chars + "..."
    assert!(args.ends_with("..."));
}

#[test]
fn test_tool_max_content_len_none_no_truncation() {
    let long = "a".repeat(200);
    let msgs = vec![
        ChatMessage::tool_result("1", &long),
        ChatMessage::assistant_tool_calls(vec![ToolCall {
            id: "1".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "bash".into(),
                arguments: long.clone(),
            },
        }]),
    ];
    let config = DatabaseConfig::default(); // tool_max_content_len = None
    let result = prepare_messages_for_storage(&msgs, &config);
    assert_eq!(result.len(), 2);
    // Content should not be truncated
    match &result[0].content {
        crate::openrouter::ChatContent::Text(s) => assert_eq!(s.len(), 200),
        _ => panic!("expected text content"),
    }
    assert_eq!(
        result[1].tool_calls.as_ref().unwrap()[0].function.arguments.len(),
        200
    );
}

#[test]
fn test_reasoning_max_content_len_truncates() {
    let msgs = vec![ChatMessage::assistant_with_reasoning(
        "response",
        "r".repeat(200),
    )];
    let config = DatabaseConfig {
        reasoning_max_content_len: Some(10),
        ..DatabaseConfig::default()
    };
    let result = prepare_messages_for_storage(&msgs, &config);
    assert_eq!(result.len(), 1);
    let reasoning = result[0].reasoning.as_ref().unwrap();
    assert!(reasoning.len() <= 13); // 10 chars + "..."
    assert!(reasoning.ends_with("..."));
}

#[test]
fn test_reasoning_preserved_by_default() {
    let msgs = vec![ChatMessage::assistant_with_reasoning(
        "response",
        "thinking".into(),
    )];
    let config = DatabaseConfig::default();
    let result = prepare_messages_for_storage(&msgs, &config);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].reasoning.as_deref(), Some("thinking"));
}

#[test]
fn test_tool_calls_always_preserved() {
    let msgs = vec![
        ChatMessage::user("hello"),
        ChatMessage::assistant_tool_calls(vec![ToolCall {
            id: "1".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "bash".into(),
                arguments: "echo hi".into(),
            },
        }]),
        ChatMessage::tool_result("1", "output"),
        ChatMessage::assistant("response"),
    ];
    let config = DatabaseConfig::default();
    let result = prepare_messages_for_storage(&msgs, &config);
    assert_eq!(result.len(), 4);
}
