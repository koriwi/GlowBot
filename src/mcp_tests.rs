use super::*;
use crate::config::McpServer;
use super::mcp_client::McpClient;
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

fn test_server(url: &str) -> McpServer {
    McpServer {
        name: "test-server".into(),
        transport: "streamable".into(),
        url: url.into(),
        api_key: Some("test-key".into()),
    }
}

fn test_server_no_auth(url: &str) -> McpServer {
    McpServer {
        name: "test-server".into(),
        transport: "streamable".into(),
        url: url.into(),
        api_key: None,
    }
}

fn jsonrpc_ok(id: u64, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn jsonrpc_err(id: u64, message: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"message": message}})
}

#[tokio::test]
async fn test_mcp_client_initialize_success() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(
                    1,
                    serde_json::json!({"protocolVersion": "2024-11-05"}),
                ))
                .insert_header("mcp-session-id", "sess123"),
        )
        .mount(&mock)
        .await;

    let client = McpClient::new(test_server(&mock.uri()));
    client.initialize().await.unwrap();

    let sid = client.session_id.lock().await.clone();
    assert_eq!(sid, Some("sess123".to_string()));
}

#[tokio::test]
async fn test_mcp_client_initialize_all_versions_fail() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("fail"))
        .mount(&mock)
        .await;

    let client = McpClient::new(test_server(&mock.uri()));
    let result = client.initialize().await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("All protocol versions failed"));
}

#[tokio::test]
async fn test_mcp_client_discover_tools() {
    let mock = MockServer::start().await;

    // initialize handler (body contains protocolVersion)
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

    // notifications/initialized handler
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("notifications/initialized"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jsonrpc_ok(2, serde_json::Value::Null)),
        )
        .mount(&mock)
        .await;

    // tools/list handler
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/list"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(jsonrpc_ok(3, serde_json::json!({
                "tools": [
                    {"name": "search", "description": "Search tool", "inputSchema": {"type": "object"}}
                ]
            }))))
        .mount(&mock)
        .await;

    let tools = client.discover_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "search");
    assert_eq!(tools[0].description, "Search tool");
}

#[tokio::test]
async fn test_discover_all_with_server() {
    let mock = MockServer::start().await;
    // initialize (has protocolVersion in body)
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("protocolVersion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jsonrpc_ok(
            1,
            serde_json::json!({"protocolVersion": "2024-11-05"}),
        )))
        .mount(&mock)
        .await;

    // notifications/initialized
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("notifications/initialized"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jsonrpc_ok(2, serde_json::Value::Null)),
        )
        .mount(&mock)
        .await;

    // tools/list
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jsonrpc_ok(
            3,
            serde_json::json!({
                "tools": [
                    {"name": "fetch", "description": "Fetch URL", "inputSchema": {"type": "object"}}
                ]
            }),
        )))
        .mount(&mock)
        .await;

    let server = McpServer {
        name: "fetch-server".into(),
        transport: "streamable".into(),
        url: mock.uri(),
        api_key: None,
    };

    let tools = discover_all(&[server]).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "fetch");
    assert_eq!(tools[0].server_name, "fetch-server");
}

#[tokio::test]
async fn test_discover_all_server_fails_returns_empty() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let server = test_server(&mock.uri());
    let tools = discover_all(&[server]).await.unwrap();
    assert!(tools.is_empty());
}

#[tokio::test]
async fn test_discover_all_empty_servers() {
    let tools = discover_all(&[]).await.unwrap();
    assert!(tools.is_empty());
}

#[tokio::test]
async fn test_invoke_tool_success() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(1, serde_json::json!({"result": "found it"}))),
        )
        .mount(&mock)
        .await;

    let mut tool = McpTool {
        server_name: "s".into(),
        name: "search".into(),
        description: "d".into(),
        input_schema: serde_json::json!({}),
        server_url: mock.uri(),
        api_key: None,
        session_id: None,
        transport: "streamable".into(),
    };

    let result = invoke_tool(&mut tool, &serde_json::json!({"query": "hello"})).await;
    assert!(result.contains("found it"));
}

#[tokio::test]
async fn test_invoke_tool_error_response() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jsonrpc_err(1, "tool exploded")))
        .mount(&mock)
        .await;

    let mut tool = McpTool {
        server_name: "s".into(),
        name: "bad".into(),
        description: "d".into(),
        input_schema: serde_json::json!({}),
        server_url: mock.uri(),
        api_key: None,
        session_id: None,
        transport: "streamable".into(),
    };

    let result = invoke_tool(&mut tool, &serde_json::json!({})).await;
    assert!(result.contains("tool exploded"));
}

#[tokio::test]
async fn test_invoke_tool_parse_error() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&mock)
        .await;

    let mut tool = McpTool {
        server_name: "s".into(),
        name: "bad".into(),
        description: "d".into(),
        input_schema: serde_json::json!({}),
        server_url: mock.uri(),
        api_key: None,
        session_id: None,
        transport: "streamable".into(),
    };

    let result = invoke_tool(&mut tool, &serde_json::json!({})).await;
    assert!(result.contains("parse error"), "result: {}", result);
    assert!(
        result.contains("not json"),
        "body should be in error, got: {}",
        result
    );
}

#[tokio::test]
async fn test_invoke_tool_session_expired_and_reinitialized() {
    let mock = MockServer::start().await;

    // Call 1: tools/call with stale session → HTTP 500 "Session not found"
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/call"))
        .and(matchers::header("mcp-session-id", "oldsess"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Session not found"))
        .mount(&mock)
        .await;

    // Call 2: initialize (body contains protocolVersion)
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("protocolVersion"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(
                    1,
                    serde_json::json!({"protocolVersion": "2024-11-05"}),
                ))
                .insert_header("mcp-session-id", "newsess456"),
        )
        .mount(&mock)
        .await;

    // Call 3: notifications/initialized
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("notifications/initialized"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jsonrpc_ok(2, serde_json::Value::Null)),
        )
        .mount(&mock)
        .await;

    // Call 4: tools/call with new session → success
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/call"))
        .and(matchers::header("mcp-session-id", "newsess456"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jsonrpc_ok(
            3,
            serde_json::json!({"result": "ok after reinit"}),
        )))
        .mount(&mock)
        .await;

    let mut tool = McpTool {
        server_name: "s".into(),
        name: "search".into(),
        description: "d".into(),
        input_schema: serde_json::json!({}),
        server_url: mock.uri(),
        api_key: None,
        session_id: Some("oldsess".into()),
        transport: "streamable".into(),
    };

    let result = invoke_tool(&mut tool, &serde_json::json!({"query": "hello"})).await;
    assert!(
        result.contains("ok after reinit"),
        "Expected 'ok after reinit' in: {}",
        result
    );
    // Verify the tool's session_id was updated in place
    assert_eq!(tool.session_id.as_deref(), Some("newsess456"));
}

#[tokio::test]
async fn test_invoke_tool_session_expired_reinit_fails_returns_original_error() {
    let mock = MockServer::start().await;

    // Call 1: tools/call → HTTP 500 "Session not found"
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/call"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Session not found"))
        .mount(&mock)
        .await;

    // Re-init fails: initialize returns error
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("protocolVersion"))
        .respond_with(ResponseTemplate::new(500).set_body_string("server gone"))
        .mount(&mock)
        .await;

    let mut tool = McpTool {
        server_name: "s".into(),
        name: "search".into(),
        description: "d".into(),
        input_schema: serde_json::json!({}),
        server_url: mock.uri(),
        api_key: None,
        session_id: Some("oldsess".into()),
        transport: "streamable".into(),
    };

    let result = invoke_tool(&mut tool, &serde_json::json!({"query": "hello"})).await;
    assert!(
        result.contains("Session not found"),
        "Expected original error preserved, got: {}",
        result
    );
}

#[tokio::test]
async fn test_invoke_tool_http_500_other_error_no_retry() {
    let mock = MockServer::start().await;

    // tools/call → HTTP 500 with some other error (not session-related)
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal server error"))
        .mount(&mock)
        .await;

    let mut tool = McpTool {
        server_name: "s".into(),
        name: "search".into(),
        description: "d".into(),
        input_schema: serde_json::json!({}),
        server_url: mock.uri(),
        api_key: None,
        session_id: Some("sess1".into()),
        transport: "streamable".into(),
    };

    let result = invoke_tool(&mut tool, &serde_json::json!({"query": "hello"})).await;
    assert!(
        result.contains("Internal server error"),
        "Expected error returned without retry, got: {}",
        result
    );
}

#[tokio::test]
async fn test_invoke_tool_no_session_id_still_retries_on_session_not_found() {
    let mock = MockServer::start().await;

    // Mount retry mock FIRST (more specific), then fallback mock (less specific).
    // Wiremock matches the first mounted mock, so the retry must be
    // mounted before the catch-all fallback.

    // Retry: tools/call with fresh session → success (mounted first, highest priority)
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/call"))
        .and(matchers::header("mcp-session-id", "fresh"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(3, serde_json::json!({"ok": true}))),
        )
        .mount(&mock)
        .await;

    // Call 1: tools/call without session → HTTP 500 "Session not found" (fallback)
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/call"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Session not found"))
        .mount(&mock)
        .await;

    // Re-init: initialize → 200 with session
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("protocolVersion"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(
                    1,
                    serde_json::json!({"protocolVersion": "2024-11-05"}),
                ))
                .insert_header("mcp-session-id", "fresh"),
        )
        .mount(&mock)
        .await;

    // notifications/initialized
    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("notifications/initialized"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jsonrpc_ok(2, serde_json::Value::Null)),
        )
        .mount(&mock)
        .await;

    let mut tool = McpTool {
        server_name: "s".into(),
        name: "search".into(),
        description: "d".into(),
        input_schema: serde_json::json!({}),
        server_url: mock.uri(),
        api_key: None,
        session_id: None, // no session yet, but server still says "Session not found"
        transport: "streamable".into(),
    };

    let result = invoke_tool(&mut tool, &serde_json::json!({"query": "hello"})).await;
    assert!(
        result.contains("ok"),
        "Expected retry success, got: {}",
        result
    );
}

#[tokio::test]
async fn test_invoke_tool_stateless_transport_no_retry() {
    let mock = MockServer::start().await;

    // For "http" (stateless) transport, 500 Session not found shouldn't trigger retry
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Session not found"))
        .mount(&mock)
        .await;

    let mut tool = McpTool {
        server_name: "s".into(),
        name: "search".into(),
        description: "d".into(),
        input_schema: serde_json::json!({}),
        server_url: mock.uri(),
        api_key: None,
        session_id: Some("sess1".into()),
        transport: "http".into(),
    };

    let result = invoke_tool(&mut tool, &serde_json::json!({"query": "hello"})).await;
    // Should return the error without attempting re-init (no initialize mock mounted)
    assert!(result.contains("Session not found"));
}

#[tokio::test]
async fn test_invoke_tool_network_error() {
    // Use a URL that's guaranteed to fail
    let mut tool = McpTool {
        server_name: "s".into(),
        name: "bad".into(),
        description: "d".into(),
        input_schema: serde_json::json!({}),
        server_url: "http://127.0.0.1:1".into(),
        api_key: None,
        session_id: None,
        transport: "streamable".into(),
    };

    let result = invoke_tool(&mut tool, &serde_json::json!({})).await;
    assert!(result.contains("request failed"), "result: {}", result);
}

#[tokio::test]
async fn test_rpc_call_http_error() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&mock)
        .await;

    let client = McpClient::new(test_server(&mock.uri()));
    let result = client.rpc_call("test", None).await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    println!("Error message: {}", msg);
    assert!(msg.contains("400"), "Expected '400' in: {}", msg);
}

#[tokio::test]
async fn test_rpc_call_jsonrpc_error() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jsonrpc_err(1, "something went wrong")),
        )
        .mount(&mock)
        .await;

    let client = McpClient::new(test_server(&mock.uri()));
    let result = client.rpc_call("test", None).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("something went wrong"));
}

#[tokio::test]
async fn test_discover_tools_empty_response() {
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

    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("notifications/initialized"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jsonrpc_ok(2, serde_json::Value::Null)),
        )
        .mount(&mock)
        .await;

    Mock::given(matchers::method("POST"))
        .and(matchers::body_string_contains("tools/list"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(3, serde_json::json!({"tools": []}))),
        )
        .mount(&mock)
        .await;

    let tools = client.discover_tools().await.unwrap();
    assert!(tools.is_empty());
}

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
