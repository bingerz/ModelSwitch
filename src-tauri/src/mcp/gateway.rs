//! MCP Gateway Mode — exposes ModelSwitch as an MCP server over HTTP.
//!
//! When enabled, MCP clients (Claude Desktop, Cursor, etc.) can connect
//! to `/mcp` and access all tools from managed MCP servers.

use std::future::Future;
use std::sync::Arc;

use rmcp::{
    model::*,
    service::{MaybeSendFuture, RequestContext},
    ErrorData as McpError, RoleServer, ServerHandler,
};
use tracing::warn;

use crate::mcp::{aggregator, McpManager};

/// MCP Gateway handler that aggregates tools from all managed MCP servers
/// and routes tool calls to the correct upstream server.
#[derive(Clone)]
pub struct McpGatewayHandler {
    mcp_manager: Arc<McpManager>,
}

impl McpGatewayHandler {
    pub fn new(mcp_manager: Arc<McpManager>) -> Self {
        Self { mcp_manager }
    }
}

impl ServerHandler for McpGatewayHandler {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        let mut impl_info = Implementation::default();
        impl_info.name = "ModelSwitch".into();
        impl_info.version = env!("CARGO_PKG_VERSION").into();
        info.server_info = impl_info;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "ModelSwitch MCP Gateway — proxying tools from all configured MCP servers.".into(),
        );
        info
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + MaybeSendFuture + '_ {
        let mcp_manager = &self.mcp_manager;
        async move {
            let aggregated = aggregator::aggregate_all_tools(mcp_manager).await;

            let tools: Vec<Tool> = aggregated
                .into_iter()
                .map(|at| {
                    let input_schema: JsonObject =
                        if let serde_json::Value::Object(map) = at.input_schema {
                            map
                        } else {
                            serde_json::Map::new()
                        };

                    let mut tool = Tool::default();
                    tool.name = at.namespaced_name.into();
                    tool.description = at.description.map(Into::into);
                    tool.input_schema = Arc::new(input_schema);
                    tool
                })
                .collect();

            let mut result = ListToolsResult::default();
            result.tools = tools;
            Ok(result)
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + MaybeSendFuture + '_ {
        let mcp_manager = &self.mcp_manager;
        async move {
            let tool_name = request.name.as_ref();

            // Parse the namespaced tool name: mcp__{server_id}__{original_name}
            let (server_id, original_name) = match aggregator::AggregatedTool::parse_namespaced(
                tool_name,
            ) {
                Some((sid, orig)) => (sid.to_string(), orig.to_string()),
                None => {
                    return Err(McpError::invalid_params(
                            format!(
                                "Tool name '{}' is not a valid namespaced MCP tool name (expected mcp__serverId__toolName)",
                                tool_name
                            ),
                            None,
                        ));
                }
            };

            // Route to the correct MCP server
            match mcp_manager
                .call_tool(&server_id, &original_name, request.arguments)
                .await
            {
                Ok(result) => Ok(result),
                Err(e) => {
                    warn!(
                        server_id = %server_id,
                        tool = %original_name,
                        error = %e,
                        "MCP gateway: tool call failed"
                    );
                    Err(McpError::internal_error(
                        format!("Tool call failed: {}", e),
                        None,
                    ))
                }
            }
        }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        // Validate that this is a namespaced tool name.
        let (_server_id, _original_name) = aggregator::AggregatedTool::parse_namespaced(name)?;
        // Return a minimal Tool for task-support validation.
        let mut tool = Tool::default();
        tool.name = name.to_string().into();
        Some(tool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_namespaced_extracts_server_and_tool() {
        let (server, tool) =
            aggregator::AggregatedTool::parse_namespaced("mcp__filesystem__read_file").unwrap();
        assert_eq!(server, "filesystem");
        assert_eq!(tool, "read_file");
    }

    #[test]
    fn parse_namespaced_rejects_non_namespaced() {
        assert!(aggregator::AggregatedTool::parse_namespaced("plain_tool_name").is_none());
    }

    #[test]
    fn parse_namespaced_rejects_empty_segments() {
        assert!(aggregator::AggregatedTool::parse_namespaced("mcp____read_file").is_none());
        assert!(aggregator::AggregatedTool::parse_namespaced("mcp__server__").is_none());
    }

    #[test]
    fn handler_can_be_constructed() {
        let handler = McpGatewayHandler::new(Arc::new(McpManager::new()));
        let info = handler.get_info();
        assert_eq!(info.server_info.name.as_ref() as &str, "ModelSwitch");
    }
}
