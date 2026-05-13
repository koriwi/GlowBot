#[test]
fn test_mcp_tool_serialization() {
    let tool = McpTool {
        server_name: "test-server".into(),
        name: "test-tool".into(),
        description: "A test tool".into(),
        input_schema: serde_json::json!({"type": "object"}),
        server_url: "https://example.com/mcp".into(),
        api_key: None,
        session_id: None,
        transport: "streamable".into(),
    };
    assert_eq!(tool.name, "test-tool");
    assert_eq!(tool.server_name, "test-server");
}

#[test]
fn test_mcp_server_config() {
    let server = McpServer {
        name: "my-server".into(),
        transport: "streamable".into(),
        url: "https://example.com/mcp".into(),
        api_key: Some("secret".into()),
    };
    assert_eq!(server.url, "https://example.com/mcp");
    assert_eq!(server.api_key, Some("secret".into()));
    assert_eq!(server.transport, "streamable");
}

#[test]
fn test_mcp_server_default_transport() {
    let yaml = "name: test\nurl: https://example.com\n";
    let server: McpServer = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(server.transport, "streamable");
}

#[test]
fn test_parse_tools_missing_description() {
    let client = McpClient::new(test_server_no_auth("http://unused"));
    let result = serde_json::json!({
        "tools": [
            {"name": "no-desc", "inputSchema": {"type": "object"}}
        ]
    });
    let tools = client.parse_tools_from_result(&result, None);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "no-desc");
    assert_eq!(tools[0].description, "");
}

#[test]
fn test_parse_tools_missing_input_schema() {
    let client = McpClient::new(test_server_no_auth("http://unused"));
    let result = serde_json::json!({
        "tools": [
            {"name": "no-schema", "description": "just a tool"}
        ]
    });
    let tools = client.parse_tools_from_result(&result, None);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "no-schema");
    assert_eq!(tools[0].input_schema, serde_json::json!({}));
}

#[test]
fn test_parse_tools_missing_both_optional_fields() {
    let client = McpClient::new(test_server_no_auth("http://unused"));
    let result = serde_json::json!({
        "tools": [
            {"name": "bare-minimum"}
        ]
    });
    let tools = client.parse_tools_from_result(&result, None);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "bare-minimum");
    assert_eq!(tools[0].description, "");
    assert_eq!(tools[0].input_schema, serde_json::json!({}));
}

#[tokio::test]
async fn test_discover_tools_pagination() {
    let mock = MockServer::start().await;
    // initialize
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("protocolVersion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jsonrpc_ok(
            1,
            serde_json::json!({"protocolVersion": "2024-11-05"}),
        )))
        .mount(&mock)
        .await;

    let client = McpClient::new(test_server_no_auth(&mock.uri()));
    client.initialize().await.unwrap();

    // notifications/initialized
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("notifications/initialized"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jsonrpc_ok(2, serde_json::Value::Null)),
        )
        .mount(&mock)
        .await;

    // Page 2 (cursor-specific): mount LAST so it takes priority for requests with cursor
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/list"))
        .and(matchers::body_string_contains("page2cursor"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jsonrpc_ok(
            4,
            serde_json::json!({
                "tools": [
                    {"name": "page2_a", "description": "second page tool A"},
                    {"name": "page2_b", "description": "second page tool B"}
                ]
            }),
        )))
        .mount(&mock)
        .await;

    // Page 1 (generic tools/list): returns 1 tool + nextCursor
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jsonrpc_ok(
            3,
            serde_json::json!({
                "tools": [
                    {"name": "page1", "description": "first page tool"}
                ],
                "nextCursor": "page2cursor"
            }),
        )))
        .mount(&mock)
        .await;

    let tools = client.discover_tools().await.unwrap();
    assert_eq!(tools.len(), 3);
    assert_eq!(tools[0].name, "page1");
    assert_eq!(tools[1].name, "page2_a");
    assert_eq!(tools[2].name, "page2_b");
}
