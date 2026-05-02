use crate::config::McpServer;
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
}

impl McpClient {
    fn new(server: McpServer) -> Self {
        Self {
            server,
            http: reqwest::Client::new(),
            request_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    fn next_id(&self) -> u64 {
        self.request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    /// Send a JSON-RPC request and return the result.
    async fn rpc_call(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: self.next_id(),
            method: method.to_string(),
            params,
        };

        let mut req = self
            .http
            .post(&self.server.url)
            .header("Content-Type", "application/json");

        if let Some(ref key) = self.server.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let response = req.json(&request).send().await?;
        let status = response.status();

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

        Ok(rpc_response.result.unwrap_or(serde_json::Value::Null))
    }

    /// Initialize the MCP connection and discover tools.
    async fn discover_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        // Send initialize
        let init_params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "GlowBot",
                "version": "0.1.0"
            }
        });
        let _init = self.rpc_call("initialize", Some(init_params)).await?;

        // Send initialized notification (no response expected)
        let _ = self.rpc_call("notifications/initialized", None).await;

        // List tools
        let tools_result = self.rpc_call("tools/list", None).await?;
        let tools_array = tools_result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

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
        .header("Content-Type", "application/json");

    if let Some(ref key) = tool.api_key {
        req = req.header("Authorization", format!("Bearer {}", key));
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
                    "Failed to connect to MCP server '{}' ({}): {}",
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

    #[test]
    fn test_mcp_tool_serialization() {
        let tool = McpTool {
            server_name: "test-server".into(),
            name: "test-tool".into(),
            description: "A test tool".into(),
            input_schema: serde_json::json!({"type": "object"}),
            server_url: "https://example.com/mcp".into(),
            api_key: None,
        };
        assert_eq!(tool.name, "test-tool");
        assert_eq!(tool.server_name, "test-server");
    }

    #[test]
    fn test_mcp_server_config() {
        let server = McpServer {
            name: "my-server".into(),
            url: "https://example.com/mcp".into(),
            api_key: Some("secret".into()),
        };
        assert_eq!(server.url, "https://example.com/mcp");
        assert_eq!(server.api_key, Some("secret".into()));
    }
}
