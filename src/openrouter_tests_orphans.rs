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

