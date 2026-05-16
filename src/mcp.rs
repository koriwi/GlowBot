use crate::config::McpServer;
use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    service::{Peer, RoleClient, RunningService},
    transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    },
};

/// Lightweight info about a tool discovered from an MCP server.
/// Used only to build LLM tool definitions.
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub server_name: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A live connection to an MCP server that can invoke tools.
/// Must be kept alive for `Peer` handles to function.
pub struct McpConnection {
    pub server_name: String,
    running: RunningService<RoleClient, ()>,
}

impl McpConnection {
    /// Get a cloned `Peer` handle for this server.
    pub fn peer(&self) -> Peer<RoleClient> {
        self.running.peer().clone()
    }
}

/// Convert a `CallToolResult` into a plain string for the LLM.
fn call_tool_result_to_string(result: CallToolResult) -> String {
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");

    if result.is_error == Some(true) {
        if !text.is_empty() {
            return format!("error: {}", text);
        }
        return serde_json::to_string(&result).unwrap_or_else(|_| "unknown error".to_string());
    }

    if !text.is_empty() {
        return text;
    }

    // fallback: serialize entire result
    serde_json::to_string(&result).unwrap_or_else(|_| "no output".to_string())
}

/// Connect to a single MCP server, discover tools, and return the connection + tool infos.
pub async fn connect_and_discover(
    server: &McpServer,
) -> anyhow::Result<(McpConnection, Vec<McpToolInfo>)> {
    let mut config = StreamableHttpClientTransportConfig::with_uri(server.url.as_str())
        .reinit_on_expired_session(true);

    if let Some(ref key) = server.api_key {
        config = config.auth_header(key.clone());
    }

    let transport = StreamableHttpClientTransport::from_config(config);
    let running = rmcp::serve_client((), transport).await?;
    let tools = running.list_all_tools().await?;

    let infos: Vec<McpToolInfo> = tools
        .into_iter()
        .map(|t| McpToolInfo {
            server_name: server.name.clone(),
            name: t.name.to_string(),
            description: t.description.map(|d| d.to_string()).unwrap_or_default(),
            input_schema: serde_json::Value::Object(t.input_schema.as_ref().clone()),
        })
        .collect();

    let connection = McpConnection {
        server_name: server.name.clone(),
        running,
    };

    Ok((connection, infos))
}

/// Connect to all configured MCP servers and return connections + discovered tools.
pub async fn discover_all(
    servers: &[McpServer],
) -> anyhow::Result<(Vec<McpConnection>, Vec<McpToolInfo>)> {
    log::info!("MCP discover_all: {} server(s) configured", servers.len());

    let mut connections = Vec::new();
    let mut all_tools = Vec::new();

    for server in servers {
        log::info!(
            "MCP discover_all: connecting to '{}' at {} (transport={}, auth={})",
            server.name,
            server.url,
            server.transport,
            server.api_key.is_some()
        );

        match connect_and_discover(server).await {
            Ok((conn, tools)) => {
                log::info!(
                    "MCP server '{}' connected: {} tools discovered",
                    server.name,
                    tools.len()
                );
                connections.push(conn);
                all_tools.extend(tools);
            }
            Err(e) => {
                log::warn!(
                    "MCP discover_all: failed to connect to '{}' ({}): {}",
                    server.name,
                    server.url,
                    e
                );
            }
        }
    }

    log::info!(
        "MCP discover_all: done, {} total tools from {}/{} connected server(s)",
        all_tools.len(),
        connections.len(),
        servers.len()
    );

    Ok((connections, all_tools))
}

/// Invoke a tool on an MCP server via its peer handle.
pub async fn invoke_tool(
    peer: &Peer<RoleClient>,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let args =
        if arguments.is_null() || arguments.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            None
        } else {
            Some(arguments.as_object().cloned().unwrap_or_default())
        };

    let mut params = CallToolRequestParams::new(tool_name.to_string());
    params.arguments = args;

    match peer.call_tool(params).await {
        Ok(result) => call_tool_result_to_string(result),
        Err(e) => format!("MCP tool call error: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Content;

    #[test]
    fn test_call_tool_result_to_string_text() {
        let result = CallToolResult::success(vec![Content::text("hello world")]);
        assert_eq!(call_tool_result_to_string(result), "hello world");
    }

    #[test]
    fn test_call_tool_result_to_string_error() {
        let result = CallToolResult::error(vec![Content::text("something went wrong")]);
        assert_eq!(
            call_tool_result_to_string(result),
            "error: something went wrong"
        );
    }

    #[test]
    fn test_call_tool_result_to_string_multiple() {
        let result = CallToolResult::success(vec![Content::text("line1"), Content::text("line2")]);
        assert_eq!(call_tool_result_to_string(result), "line1\nline2");
    }

    #[test]
    fn test_call_tool_result_to_string_empty_fallback() {
        // Default() gives content=[], no structured content, no is_error
        let result = CallToolResult::default();
        let s = call_tool_result_to_string(result);
        assert!(!s.is_empty());
    }
}
