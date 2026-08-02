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
    // With image_gen_model, without bash, without embedding: 21 base + generate_image + 3 config + 2 model tools = 27
    let tools = all_tool_definitions(false, None, "/media", Some("black-forest-labs/flux-1.1-pro"), None, None);
    assert_eq!(tools.len(), 27);
    assert!(tools.iter().any(|t| t.function.name == "generate_image"));

    // With image_gen_model, with bash, without embedding: 21 base + generate_image + bash + 3 config + 2 model tools = 28
    let tools = all_tool_definitions(true, None, "/media", Some("black-forest-labs/flux-1.1-pro"), None, None);
    assert_eq!(tools.len(), 28);
    assert_eq!(tools[0].function.name, "bash");
    assert!(tools.iter().any(|t| t.function.name == "generate_image"));

    // Without image_gen_model: generate_image is excluded
    let tools = all_tool_definitions(true, None, "/media", None, None, None);
    assert_eq!(tools.len(), 27);
    assert!(!tools.iter().any(|t| t.function.name == "generate_image"));
}

#[test]
fn test_describe_image_tool_definition() {
    let def = describe_image_tool_definition();
    assert_eq!(def.def_type, "function");
    assert_eq!(def.function.name, "describe_image");
    assert!(def.function.description.contains("vision-capable"));
    assert!(def.function.description.contains("portion sizes"));
    assert!(def.function.description.contains("NEVER send that heads-up"));
    assert!(def.function.description.contains("terminal-only message policy"));

    let params = &def.function.parameters;
    assert_eq!(params["type"], "object");
    let required = params["required"].as_array().unwrap();
    assert_eq!(required.len(), 2);
    assert!(required.iter().any(|v| v.as_str() == Some("file_path")));
    assert!(required.iter().any(|v| v.as_str() == Some("prompt")));
    assert_eq!(params["properties"]["file_path"]["type"], "string");
    assert_eq!(params["properties"]["prompt"]["type"], "string");
}

#[test]
fn test_all_tool_definitions_with_image_fallback_model() {
    // With image_fallback_model, without bash, without embedding: 21 base + describe_image + 3 config + 2 model tools = 27
    let tools = all_tool_definitions(false, None, "/media", None, Some("openai/gpt-4o"), None);
    assert_eq!(tools.len(), 27);
    assert!(tools.iter().any(|t| t.function.name == "describe_image"));

    // With image_fallback_model, with bash, without embedding: 21 base + describe_image + bash + 3 config + 2 model tools = 28
    let tools = all_tool_definitions(true, None, "/media", None, Some("openai/gpt-4o"), None);
    assert_eq!(tools.len(), 28);
    assert_eq!(tools[0].function.name, "bash");
    assert!(tools.iter().any(|t| t.function.name == "describe_image"));

    // Without image_fallback_model: describe_image is excluded
    let tools = all_tool_definitions(true, None, "/media", None, None, None);
    assert_eq!(tools.len(), 27);
    assert!(!tools.iter().any(|t| t.function.name == "describe_image"));
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

// --- ask_advisor tool definition tests ---

#[test]
fn test_ask_advisor_tool_definition() {
    let def = ask_advisor_tool_definition("openai/gpt-4o");
    assert_eq!(def.function.name, "ask_advisor");
    assert!(def.function.description.contains("openai/gpt-4o"));
    let params = &def.function.parameters;
    assert!(params["required"].as_array().unwrap().iter().any(|v| v.as_str() == Some("query")));
    assert!(params["properties"]["query"]["type"].as_str() == Some("string"));
}

#[test]
fn test_all_tool_definitions_with_advice_model() {
    // Without bash, without embedding, with advice_model: base 26 + advice = 27
    let tools = all_tool_definitions(false, None, "/media", None, None, Some("openai/gpt-4o"));
    assert_eq!(tools.len(), 27);
    assert!(tools.iter().any(|t| t.function.name == "ask_advisor"));

    // With bash, without embedding, with advice_model: base 27 + advice = 28
    let tools = all_tool_definitions(true, None, "/media", None, None, Some("openai/gpt-4o"));
    assert_eq!(tools.len(), 28);
    assert!(tools.iter().any(|t| t.function.name == "ask_advisor"));
    assert!(tools.iter().any(|t| t.function.name == "bash"));

    // Without advice_model: tool is excluded
    let tools = all_tool_definitions(true, None, "/media", None, None, None);
    assert_eq!(tools.len(), 27);
    assert!(!tools.iter().any(|t| t.function.name == "ask_advisor"));
}
