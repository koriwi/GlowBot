use crate::config::McpServer;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

/// A tool discovered from an MCP server.
#[derive(Debug, Clone)]
pub struct McpTool {
    /// The server this tool belongs to.
    pub server_name: String,
    /// The tool name from the MCP server.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: serde_json::Value,
    /// Server URL for invoking the tool.
    pub server_url: String,
    /// Optional auth token.
    pub api_key: Option<String>,
    /// Session ID for streamable transport (if any).
    pub session_id: Option<String>,
    /// Transport type.
    pub transport: String,
}

/// The JSON-RPC request body for MCP.
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

/// The JSON-RPC response from MCP.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    message: String,
}

/// MCP client for a single server.
struct McpClient {
    server: McpServer,
    http: reqwest::Client,
    request_id: std::sync::atomic::AtomicU64,
    session_id: std::sync::Mutex<Option<String>>,
}

impl McpClient {
    fn new(server: McpServer) -> Self {
        Self {
            server,
            http: reqwest::Client::new(),
            request_id: std::sync::atomic::AtomicU64::new(1),
            session_id: std::sync::Mutex::new(None),
        }
    }

    fn next_id(&self) -> u64 {
        self.request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    /// Send a JSON-RPC request and return the result + response headers.
    async fn rpc_call_with_headers(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<(serde_json::Value, HeaderMap)> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: self.next_id(),
            method: method.to_string(),
            params,
        };

        let mut req = self
            .http
            .post(&self.server.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");

        if let Some(ref key) = self.server.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        // Include session ID for streamable transport
        if self.server.transport == "streamable" {
            if let Some(ref sid) = *self.session_id.lock().unwrap() {
                req = req.header("Mcp-Session-Id", sid);
            }
        }

        let response = req.json(&request).send().await?;
        let status = response.status();
        let headers = response.headers().clone();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "MCP server {} error ({}): {}",
                self.server.url,
                status,
                body
            );
        }

        let rpc_response: JsonRpcResponse = response.json().await?;

        if let Some(err) = rpc_response.error {
            anyhow::bail!("MCP RPC error from {}: {}", self.server.url, err.message);
        }

        Ok((
            rpc_response.result.unwrap_or(serde_json::Value::Null),
            headers,
        ))
    }

    /// Send a JSON-RPC request (convenience, discards headers).
    async fn rpc_call(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        self.rpc_call_with_headers(method, params)
            .await
            .map(|(r, _)| r)
    }

    /// Initialize the MCP connection, trying protocol versions in order.
    async fn initialize(&self) -> anyhow::Result<()> {
        let versions = ["2025-11-25", "2025-06-18", "2024-11-05"];

        for version in versions {
            let init_params = serde_json::json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": {
                    "name": "GlowBot",
                    "version": "0.1.0"
                }
            });

            match self
                .rpc_call_with_headers("initialize", Some(init_params))
                .await
            {
                Ok((_result, headers)) => {
                    // Capture session ID for streamable transport
                    if self.server.transport == "streamable" {
                        if let Some(sid) =
                            headers.get("mcp-session-id").and_then(|v| v.to_str().ok())
                        {
                            *self.session_id.lock().unwrap() = Some(sid.to_string());
                            log::info!(
                                "MCP '{}': session established (protocol {})",
                                self.server.name,
                                version
                            );
                        }
                    }
                    return Ok(());
                }
                Err(e) => {
                    log::debug!(
                        "MCP '{}': protocol {} failed: {}",
                        self.server.name,
                        version,
                        e
                    );
                }
            }
        }

        anyhow::bail!("All protocol versions failed for {}", self.server.url)
    }

    /// Discover tools from the server.
    async fn discover_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        // Send initialized notification
        let _ = self.rpc_call("notifications/initialized", None).await;

        // List tools
        let tools_result = self.rpc_call("tools/list", None).await?;
        let tools_array = tools_result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let session_id = self.session_id.lock().unwrap().clone();
        let tools: Vec<McpTool> = tools_array
            .into_iter()
            .filter_map(|t| {
                Some(McpTool {
                    server_name: self.server.name.clone(),
                    name: t.get("name")?.as_str()?.to_string(),
                    description: t.get("description")?.as_str()?.to_string(),
                    input_schema: t.get("inputSchema")?.clone(),
                    server_url: self.server.url.clone(),
                    api_key: self.server.api_key.clone(),
                    session_id: session_id.clone(),
                    transport: self.server.transport.clone(),
                })
            })
            .collect();

        Ok(tools)
    }
}

/// Invoke an MCP tool and return the result.
pub async fn invoke_tool(tool: &McpTool, arguments: &serde_json::Value) -> String {
    let client = reqwest::Client::new();
    let request = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: 1,
        method: "tools/call".to_string(),
        params: Some(serde_json::json!({
            "name": tool.name,
            "arguments": arguments,
        })),
    };

    let mut req = client
        .post(&tool.server_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json");

    if let Some(ref key) = tool.api_key {
        req = req.header("Authorization", format!("Bearer {}", key));
    }

    if tool.transport == "streamable" {
        if let Some(ref sid) = tool.session_id {
            req = req.header("Mcp-Session-Id", sid);
        }
    }

    match req.json(&request).send().await {
        Ok(response) => match response.json::<JsonRpcResponse>().await {
            Ok(rpc) => {
                if let Some(err) = rpc.error {
                    format!("MCP tool error: {}", err.message)
                } else {
                    rpc.result
                        .map(|r| r.to_string())
                        .unwrap_or_else(|| "null".into())
                }
            }
            Err(e) => format!("Failed to parse MCP response: {}", e),
        },
        Err(e) => format!("MCP request failed: {}", e),
    }
}

/// Connect to all configured MCP servers and return discovered tools.
pub async fn discover_all(servers: &[McpServer]) -> anyhow::Result<Vec<McpTool>> {
    let mut all_tools = Vec::new();

    for server in servers {
        let client = McpClient::new(server.clone());

        // Initialize (with protocol negotiation + session capture)
        if let Err(e) = client.initialize().await {
            log::warn!(
                "Failed to initialize MCP server '{}' ({}): {}",
                server.name,
                server.url,
                e
            );
            continue;
        }

        // Discover tools
        match client.discover_tools().await {
            Ok(tools) => {
                log::info!(
                    "MCP server '{}' connected: {} tools discovered",
                    server.name,
                    tools.len()
                );
                all_tools.extend(tools);
            }
            Err(e) => {
                log::warn!(
                    "Failed to discover tools from MCP server '{}' ({}): {}",
                    server.name,
                    server.url,
                    e
                );
            }
        }
    }

    Ok(all_tools)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(1, serde_json::json!({"protocolVersion": "2024-11-05"})))
                .insert_header("mcp-session-id", "sess123"))
            .mount(&mock)
            .await;

        let client = McpClient::new(test_server(&mock.uri()));
        client.initialize().await.unwrap();

        let sid = client.session_id.lock().unwrap().clone();
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
        assert!(result.unwrap_err().to_string().contains("All protocol versions failed"));
    }

    #[tokio::test]
    async fn test_mcp_client_discover_tools() {
        let mock = MockServer::start().await;

        // initialize handler (body contains protocolVersion)
        Mock::given(matchers::method("POST"))
            .and(matchers::body_string_contains("protocolVersion"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(1, serde_json::json!({"protocolVersion": "2024-11-05"}))))
            .mount(&mock)
            .await;

        let client = McpClient::new(test_server_no_auth(&mock.uri()));
        client.initialize().await.unwrap();

        // notifications/initialized handler
        Mock::given(matchers::method("POST"))
            .and(matchers::body_string_contains("notifications/initialized"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(2, serde_json::Value::Null)))
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
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(1, serde_json::json!({"protocolVersion": "2024-11-05"}))))
            .mount(&mock)
            .await;

        // notifications/initialized
        Mock::given(matchers::method("POST"))
            .and(matchers::body_string_contains("notifications/initialized"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(2, serde_json::Value::Null)))
            .mount(&mock)
            .await;

        // tools/list
        Mock::given(matchers::method("POST"))
            .and(matchers::body_string_contains("tools/list"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(3, serde_json::json!({
                    "tools": [
                        {"name": "fetch", "description": "Fetch URL", "inputSchema": {"type": "object"}}
                    ]
                }))))
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
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(1, serde_json::json!({"result": "found it"}))))
            .mount(&mock)
            .await;

        let tool = McpTool {
            server_name: "s".into(),
            name: "search".into(),
            description: "d".into(),
            input_schema: serde_json::json!({}),
            server_url: mock.uri(),
            api_key: None,
            session_id: None,
            transport: "streamable".into(),
        };

        let result = invoke_tool(&tool, &serde_json::json!({"query": "hello"})).await;
        assert!(result.contains("found it"));
    }

    #[tokio::test]
    async fn test_invoke_tool_error_response() {
        let mock = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_err(1, "tool exploded")))
            .mount(&mock)
            .await;

        let tool = McpTool {
            server_name: "s".into(),
            name: "bad".into(),
            description: "d".into(),
            input_schema: serde_json::json!({}),
            server_url: mock.uri(),
            api_key: None,
            session_id: None,
            transport: "streamable".into(),
        };

        let result = invoke_tool(&tool, &serde_json::json!({})).await;
        assert!(result.contains("tool exploded"));
    }

    #[tokio::test]
    async fn test_invoke_tool_parse_error() {
        let mock = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&mock)
            .await;

        let tool = McpTool {
            server_name: "s".into(),
            name: "bad".into(),
            description: "d".into(),
            input_schema: serde_json::json!({}),
            server_url: mock.uri(),
            api_key: None,
            session_id: None,
            transport: "streamable".into(),
        };

        let result = invoke_tool(&tool, &serde_json::json!({})).await;
        assert!(result.contains("Failed to parse MCP response"));
    }

    #[tokio::test]
    async fn test_invoke_tool_network_error() {
        // Use a URL that's guaranteed to fail
        let tool = McpTool {
            server_name: "s".into(),
            name: "bad".into(),
            description: "d".into(),
            input_schema: serde_json::json!({}),
            server_url: "http://127.0.0.1:1".into(),
            api_key: None,
            session_id: None,
            transport: "streamable".into(),
        };

        let result = invoke_tool(&tool, &serde_json::json!({})).await;
        assert!(result.contains("MCP request failed"));
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
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_err(1, "something went wrong")))
            .mount(&mock)
            .await;

        let client = McpClient::new(test_server(&mock.uri()));
        let result = client.rpc_call("test", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("something went wrong"));
    }

    #[tokio::test]
    async fn test_discover_tools_empty_response() {
        let mock = MockServer::start().await;
        // initialize
        Mock::given(matchers::method("POST"))
            .and(matchers::body_string_contains("protocolVersion"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(1, serde_json::json!({"protocolVersion": "2024-11-05"}))))
            .mount(&mock)
            .await;

        let client = McpClient::new(test_server_no_auth(&mock.uri()));
        client.initialize().await.unwrap();

        Mock::given(matchers::method("POST"))
            .and(matchers::body_string_contains("notifications/initialized"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(2, serde_json::Value::Null)))
            .mount(&mock)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_string_contains("tools/list"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(jsonrpc_ok(3, serde_json::json!({"tools": []}))))
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
}
