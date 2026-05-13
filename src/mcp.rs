#[path = "mcp_client.rs"]
pub(crate) mod mcp_client;
#[path = "mcp_invoke.rs"]
mod mcp_invoke;

pub use self::mcp_invoke::{discover_all, invoke_tool, invoke_tool_impl};

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

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
