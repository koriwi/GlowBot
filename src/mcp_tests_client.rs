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

