pub mod aggregator;
pub mod gateway;
pub mod manager;
pub mod translator;

pub use aggregator::AggregatedTool;
pub use gateway::McpGatewayHandler;
pub use manager::{McpManager, McpServerStatus};

// `McpToolInfo` remains accessible via `crate::mcp::manager::McpToolInfo` for
// callers that need the lightweight tool-listing shape returned by
// `McpManager::list_all_tools`. It is intentionally not re-exported here to
// steer external consumers toward `AggregatedTool`, which carries the full
// JSON Schema needed for tool dispatch.
