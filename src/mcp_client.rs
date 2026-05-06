use crate::config::McpServer;
use super::McpTool;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

/// The JSON-RPC request body for MCP.
#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcRequest {
    pub(crate) jsonrpc: String,
    pub(crate) id: u64,
    pub(crate) method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) params: Option<serde_json::Value>,
}

/// The JSON-RPC response from MCP.
#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcResponse {
    #[serde(default)]
    pub(crate) result: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcError {
    pub(crate) message: String,
}

/// MCP client for a single server.
pub(crate) struct McpClient {
    server: McpServer,
    http: reqwest::Client,
    request_id: std::sync::atomic::AtomicU64,
    pub(crate) session_id: std::sync::Mutex<Option<String>>,
}

impl McpClient {
    pub(crate) fn new(server: McpServer) -> Self {
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
    pub(crate) async fn rpc_call_with_headers(
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
    pub(crate) async fn rpc_call(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        self.rpc_call_with_headers(method, params)
            .await
            .map(|(r, _)| r)
    }

    /// Initialize the MCP connection, trying protocol versions in order.
    pub(crate) async fn initialize(&self) -> anyhow::Result<()> {
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
    /// Parse tool entries from a `tools/list` response result into McpTool structs.
pub(super) fn parse_tools_from_result(&self, result: &serde_json::Value) -> Vec<McpTool> {
        let tools_array = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let session_id = self.session_id.lock().unwrap().clone();
        tools_array
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                // description is optional per MCP spec
                let description = t
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                // inputSchema is optional per MCP spec
                let input_schema = t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                Some(McpTool {
                    server_name: self.server.name.clone(),
                    name,
                    description,
                    input_schema,
                    server_url: self.server.url.clone(),
                    api_key: self.server.api_key.clone(),
                    session_id: session_id.clone(),
                    transport: self.server.transport.clone(),
                })
            })
            .collect()
    }

    /// Discover tools from the server, following pagination cursors.
    pub(crate) async fn discover_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        // Send initialized notification
        let _ = self.rpc_call("notifications/initialized", None).await;

        let mut all_tools = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = cursor.as_ref().map(|c| serde_json::json!({"cursor": c}));

            let result = self.rpc_call("tools/list", params).await?;

            let page_tools = self.parse_tools_from_result(&result);
            all_tools.extend(page_tools);

            // Check for next cursor (pagination support)
            cursor = result
                .get("nextCursor")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if cursor.is_none() {
                break;
            }
        }

        Ok(all_tools)
    }
}
