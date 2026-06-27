use serde::{Deserialize, Serialize};

/// MCP server configuration for subprocess-based tool providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique identifier for this MCP server instance.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Command to execute (e.g., "npx", "node", "python").
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the subprocess.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Working directory for the subprocess.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Whether this server is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Whether tools from this server should be exposed to LLM clients.
    #[serde(default = "default_mcp_expose_tools")]
    pub expose_tools: bool,
}

// ── Default value functions ───────────────────────────

pub(crate) fn default_enabled() -> bool {
    true
}
pub(crate) fn default_mcp_expose_tools() -> bool {
    true
}
