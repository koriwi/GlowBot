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
    /// Parse tool entries from a `tools/list` response result into McpTool structs.
    fn parse_tools_from_result(
        &self,
        result: &serde_json::Value,
    ) -> Vec<McpTool> {
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
    async fn discover_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        // Send initialized notification
        let _ = self.rpc_call("notifications/initialized", None).await;

        let mut all_tools = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = cursor.as_ref().map(|c| {
                serde_json::json!({"cursor": c})
            });

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

/// Invoke an MCP tool over HTTP once (no session recovery).
async fn invoke_tool_once(tool: &McpTool, arguments: &serde_json::Value) -> String {
    let tool_label = format!("mcp_{}_{}", tool.server_name, tool.name);
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
        Ok(response) => {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            if !status.is_success() {
                let preview: String = body_text.chars().take(500).collect();
                log::warn!(
                    "{} HTTP {} from {}: {}",
                    tool_label,
                    status.as_u16(),
                    tool.server_url,
                    preview
                );
                return format!("{} HTTP {}: {}", tool_label, status.as_u16(), preview);
            }
            match serde_json::from_str::<JsonRpcResponse>(&body_text) {
                Ok(rpc) => {
                    if let Some(err) = rpc.error {
                        format!("{} RPC error: {}", tool_label, err.message)
                    } else {
                        rpc.result
                            .map(|r| r.to_string())
                            .unwrap_or_else(|| "null".into())
                    }
                }
                Err(e) => {
                    let preview: String = body_text.chars().take(500).collect();
                    log::warn!(
                        "{} failed to parse response ({} bytes) from {}: {} | body: {}",
                        tool_label,
                        body_text.len(),
                        tool.server_url,
                        e,
                        preview
                    );
                    format!(
                        "{} parse error: {} | body (first 500 chars): {}",
                        tool_label, e, preview
                    )
                }
            }
        }
        Err(e) => {
            log::warn!(
                "{} request failed to {}: {}",
                tool_label,
                tool.server_url,
                e
            );
            format!("{} request failed: {}", tool_label, e)
        }
    }
}

/// Re-initialize the MCP session: send initialize → notifications/initialized.
/// Returns the new session ID on success.
async fn reinitialize_mcp_session(tool: &McpTool) -> Option<String> {
    // Only session-based transports can be re-initialized
    if tool.transport != "streamable" {
        return None;
    }

    let server = McpServer {
        name: tool.server_name.clone(),
        transport: tool.transport.clone(),
        url: tool.server_url.clone(),
        api_key: tool.api_key.clone(),
    };

    let client = McpClient::new(server);

    // Step 1: initialize (protocol negotiation + session capture)
    if let Err(e) = client.initialize().await {
        log::warn!(
            "MCP session re-init initialize failed for {}: {}",
            tool.server_name,
            e
        );
        return None;
    }

    // Step 2: notifications/initialized
    if let Err(e) = client.rpc_call("notifications/initialized", None).await {
        log::warn!(
            "MCP session re-init notifications/initialized failed for {}: {}",
            tool.server_name,
            e
        );
        return None;
    }

    let sid = client.session_id.lock().unwrap().clone();
    sid
}

/// Invoke an MCP tool and return the result.
/// On HTTP 500 "Session not found", automatically re-initializes the session,
/// updates `tool.session_id` in place, and retries once.
pub async fn invoke_tool(tool: &mut McpTool, arguments: &serde_json::Value) -> String {
    let result = invoke_tool_once(tool, arguments).await;

    // Detect stale session: HTTP 500 with "Session not found" (session expired server-side)
    let is_session_lost = result.contains("HTTP 500")
        && result.to_lowercase().contains("session not found");

    if is_session_lost {
        log::info!(
            "MCP session expired for {}/{}, attempting re-initialization",
            tool.server_name,
            tool.name
        );

        // Reborrow as immutable for the reinit call
        if let Some(new_session_id) = reinitialize_mcp_session(&*tool).await {
            tool.session_id = Some(new_session_id);
            log::info!(
                "MCP session re-initialized for {}, retrying tool call",
                tool.server_name
            );
            return invoke_tool_once(tool, arguments).await;
        }

        log::warn!(
            "MCP session re-initialization failed for {}",
            tool.server_name
        );
    }

    result
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
#[path = "mcp_tests.rs"]
mod tests;
