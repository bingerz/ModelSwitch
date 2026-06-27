//! Tool aggregation across multiple MCP servers.
//!
//! Namespaces tool names to prevent collisions across servers.
//! The namespaced format is: `mcp__{server_id}__{original_name}`.
//! This prefix is stable and parseable back into `(server_id, original_name)`.

use crate::mcp::{McpManager, McpServerStatus};

/// A tool with its origin server identified.
///
/// Carries the full JSON Schema for the tool's parameters alongside
/// provenance metadata so callers can route tool calls back to the
/// correct MCP server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AggregatedTool {
    /// Namespaced name: `mcp__{server_id}__{original_name}`.
    pub namespaced_name: String,
    /// Original tool name from the MCP server.
    pub original_name: String,
    /// Server ID that provides this tool.
    pub server_id: String,
    /// Tool description, if the server provided one.
    pub description: Option<String>,
    /// JSON Schema describing the tool's parameters.
    pub input_schema: serde_json::Value,
}

impl AggregatedTool {
    /// Create the namespace prefix for a server: `mcp__{server_id}__`.
    pub fn namespace_prefix(server_id: &str) -> String {
        format!("mcp__{}__", server_id)
    }

    /// Build the namespaced name for a tool on a given server.
    pub fn namespaced_name(server_id: &str, original_name: &str) -> String {
        format!("{}{}", Self::namespace_prefix(server_id), original_name)
    }

    /// Parse a namespaced tool name back into `(server_id, original_name)`.
    ///
    /// Returns `None` if the name does not start with `mcp__`, if the
    /// server_id segment is empty, or if the original tool name is empty.
    ///
    /// Note: server_ids containing `__` are not supported by this scheme
    /// and should be avoided when configuring servers.
    pub fn parse_namespaced(name: &str) -> Option<(&str, &str)> {
        let rest = name.strip_prefix("mcp__")?;
        let server_id = rest.split("__").next()?;
        if server_id.is_empty() {
            return None;
        }
        let original = &rest[server_id.len() + 2..];
        if original.is_empty() {
            return None;
        }
        Some((server_id, original))
    }
}

/// Aggregate tools from all running MCP servers.
///
/// Calls `list_tools` once per running server (not once per tool) to
/// efficiently fetch full `Tool` objects including input schemas.
/// Servers that fail to enumerate are logged and skipped.
///
/// This function does NOT filter by `expose_tools`. Callers that expose
/// tools to LLM clients (e.g. the `/v1/tools` proxy endpoint) should
/// filter the result using `McpManager::get_config` to respect that flag.
pub async fn aggregate_all_tools(mcp_manager: &McpManager) -> Vec<AggregatedTool> {
    let statuses = mcp_manager.list_status().await;
    let mut all = Vec::new();

    for (server_id, _name, status) in statuses {
        if !matches!(status, McpServerStatus::Running { .. }) {
            continue;
        }

        let server_tools = match mcp_manager.list_tools(&server_id).await {
            Ok(tools) => tools,
            Err(e) => {
                tracing::warn!(
                    server_id = %server_id,
                    error = %e,
                    "Failed to list tools during aggregation"
                );
                continue;
            }
        };

        for tool in server_tools {
            let original_name = tool.name.to_string();
            let namespaced_name = AggregatedTool::namespaced_name(&server_id, &original_name);
            let input_schema = serde_json::Value::Object(tool.input_schema.as_ref().clone());

            all.push(AggregatedTool {
                namespaced_name,
                original_name,
                server_id: server_id.clone(),
                description: tool.description.as_deref().map(String::from),
                input_schema,
            });
        }
    }

    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_prefix_format() {
        assert_eq!(AggregatedTool::namespace_prefix("fs"), "mcp__fs__");
        assert_eq!(AggregatedTool::namespace_prefix("github"), "mcp__github__");
    }

    #[test]
    fn namespaced_name_combines_correctly() {
        assert_eq!(
            AggregatedTool::namespaced_name("fs", "read_file"),
            "mcp__fs__read_file"
        );
    }

    #[test]
    fn parse_namespaced_valid() {
        let (server, tool) = AggregatedTool::parse_namespaced("mcp__fs__read_file").unwrap();
        assert_eq!(server, "fs");
        assert_eq!(tool, "read_file");
    }

    #[test]
    fn parse_namespaced_preserves_double_underscore_in_tool_name() {
        let (server, tool) =
            AggregatedTool::parse_namespaced("mcp__fs__some__nested__tool").unwrap();
        assert_eq!(server, "fs");
        assert_eq!(tool, "some__nested__tool");
    }

    #[test]
    fn parse_namespaced_no_prefix() {
        assert!(AggregatedTool::parse_namespaced("read_file").is_none());
        assert!(AggregatedTool::parse_namespaced("mcp_read_file").is_none());
    }

    #[test]
    fn parse_namespaced_empty_tool() {
        assert!(AggregatedTool::parse_namespaced("mcp__fs__").is_none());
    }

    #[test]
    fn parse_namespaced_empty_server_id() {
        assert!(AggregatedTool::parse_namespaced("mcp____tool").is_none());
    }
}
