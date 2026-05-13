use crate::config::McpServer;
use super::McpTool;
use super::mcp_client::{JsonRpcRequest, JsonRpcResponse, McpClient};

/// Invoke an MCP tool over HTTP once (no session recovery).
pub(crate) async fn invoke_tool_once(tool: &McpTool, arguments: &serde_json::Value) -> String {
    let tool_label = format!("mcp_{}_{}", tool.server_name, tool.name);
    log::debug!(
        "{}: calling {} at {} (transport={}, session={}, auth={})",
        tool_label,
        tool.name,
        tool.server_url,
        tool.transport,
        tool.session_id.is_some(),
        tool.api_key.is_some()
    );

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
pub(crate) async fn reinitialize_mcp_session(tool: &McpTool) -> Option<String> {
    // Only session-based transports can be re-initialized
    if tool.transport != "streamable" {
        log::debug!(
            "MCP reinit: skipping non-streamable transport '{}' for {}",
            tool.transport,
            tool.server_name
        );
        return None;
    }

    log::info!(
        "MCP reinit: re-initializing session for {} at {}",
        tool.server_name,
        tool.server_url
    );

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

    let sid = client.session_id.lock().await.clone();
    sid
}

/// Invoke an MCP tool and return the result.
/// On HTTP 500 "Session not found", automatically re-initializes the session,
/// updates `tool.session_id` in place, and retries once.
pub async fn invoke_tool(tool: &mut McpTool, arguments: &serde_json::Value) -> String {
    invoke_tool_impl(tool, arguments, None).await
}

/// Internal implementation with optional per-server lock for session re-init serialization.
/// When `server_lock` is provided, it is only acquired during session re-initialization
/// (not during normal tool calls), preventing thundering-herd re-inits on session expiry.
pub async fn invoke_tool_impl(
    tool: &mut McpTool,
    arguments: &serde_json::Value,
    server_lock: Option<&tokio::sync::Mutex<()>>,
) -> String {
    let result = invoke_tool_once(tool, arguments).await;

    // Detect stale session: HTTP 500 with "Session not found" (session expired server-side)
    let is_session_lost =
        result.contains("HTTP 500") && result.to_lowercase().contains("session not found");

    if is_session_lost {
        // Serialize re-initialization per server to avoid double re-inits.
        // Normal tool calls never touch this lock — only the rare re-init path does.
        let _guard = if let Some(lock) = server_lock {
            Some(lock.lock().await)
        } else {
            None
        };

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
    log::info!("MCP discover_all: {} server(s) configured", servers.len());
    let mut all_tools = Vec::new();

    for server in servers {
        log::info!(
            "MCP discover_all: connecting to '{}' at {} (transport={}, auth={})",
            server.name,
            server.url,
            server.transport,
            server.api_key.is_some()
        );

        let client = McpClient::new(server.clone());

        // Initialize (with protocol negotiation + session capture)
        let start = std::time::Instant::now();
        if let Err(e) = client.initialize().await {
            log::warn!(
                "MCP discover_all: failed to initialize '{}' ({}) after {:?}: {}",
                server.name,
                server.url,
                start.elapsed(),
                e
            );
            continue;
        }
        log::debug!(
            "MCP discover_all: '{}' initialized in {:?}",
            server.name,
            start.elapsed()
        );

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
                    "MCP discover_all: failed to discover tools from '{}' ({}): {}",
                    server.name,
                    server.url,
                    e
                );
            }
        }
    }

    log::info!(
        "MCP discover_all: done, {} total tools from {} server(s)",
        all_tools.len(),
        servers.len()
    );
    Ok(all_tools)
}
