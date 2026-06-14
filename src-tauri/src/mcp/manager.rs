use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::Context;
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{Peer, RoleClient, ServiceExt};
use tokio::process::Command;
use tokio::sync::RwLock;

use crate::config::McpServerConfig;

/// Status of a managed MCP server.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerStatus {
    Stopped,
    Running { tool_count: usize },
    Error { message: String },
}

/// Tool metadata for listing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpToolInfo {
    pub server_id: String,
    pub name: String,
    pub description: Option<String>,
}

/// Entry for a managed MCP server.
struct McpServerEntry {
    config: McpServerConfig,
    client: Option<RunningService<RoleClient, ()>>,
    last_error: Option<String>,
    /// Cached tool count, updated on start and on list_tools.
    tool_count: Option<usize>,
}

/// Manages MCP server subprocesses. Thread-safe, async.
/// Mirrors the ChannelManager pattern.
pub struct McpManager {
    entries: Arc<RwLock<HashMap<String, McpServerEntry>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load server configs from AppConfig. Does NOT auto-start them.
    /// This is intended for initial startup — it clears all existing entries.
    pub async fn load_configs(&self, configs: &[McpServerConfig]) {
        let mut entries = self.entries.write().await;
        entries.clear();
        for config in configs {
            entries.insert(
                config.id.clone(),
                McpServerEntry {
                    config: config.clone(),
                    client: None,
                    last_error: None,
                    tool_count: None,
                },
            );
        }
    }

    /// Reload configs from a config change, preserving running servers that
    /// still exist in the new config.
    ///
    /// - Servers in the new config that are already running: config updated, keep running.
    /// - Servers removed from config: stopped and removed.
    /// - New servers in config: added but not auto-started.
    pub async fn reload_configs(&self, configs: &[McpServerConfig]) {
        let new_ids: HashSet<&str> = configs.iter().map(|c| c.id.as_str()).collect();

        // Collect IDs to stop and remove
        let to_remove: Vec<String> = {
            let entries = self.entries.read().await;
            entries
                .keys()
                .filter(|id| !new_ids.contains(id.as_str()))
                .cloned()
                .collect()
        };

        // Stop removed servers
        for id in &to_remove {
            let _ = self.stop_server(id).await;
        }

        // Apply changes
        let mut entries = self.entries.write().await;

        // Remove stopped entries
        for id in &to_remove {
            entries.remove(id);
            tracing::info!(server_id = %id, "Removed MCP server deleted from config");
        }

        // Add or update entries
        for config in configs {
            if let Some(entry) = entries.get_mut(&config.id) {
                // Update config for existing entry (preserve running client)
                entry.config = config.clone();
            } else {
                // New entry
                entries.insert(
                    config.id.clone(),
                    McpServerEntry {
                        config: config.clone(),
                        client: None,
                        last_error: None,
                        tool_count: None,
                    },
                );
                tracing::info!(server_id = %config.id, "Added MCP server from config");
            }
        }
    }

    /// Start a specific MCP server by ID.
    pub async fn start_server(&self, id: &str) -> anyhow::Result<()> {
        // Get config and check if already running
        let config = {
            let entries = self.entries.read().await;
            let entry = entries
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("Unknown MCP server: {id}"))?;
            if entry.client.is_some() {
                anyhow::bail!("MCP server {id} is already running");
            }
            entry.config.clone()
        };

        // Build the command
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(ref cwd) = config.cwd {
            cmd.current_dir(cwd);
        }

        // Spawn and connect (outside the lock to avoid blocking other operations)
        let spawn_result: anyhow::Result<RunningService<RoleClient, ()>> = async {
            let transport = TokioChildProcess::new(cmd)
                .map_err(|e| anyhow::anyhow!("Failed to spawn MCP process '{id}': {e}"))?;
            let client = ()
                .serve(transport)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to initialize MCP client '{id}': {e}"))?;
            Ok(client)
        }
        .await;

        match spawn_result {
            Ok(client) => {
                // Get initial tool count
                let tool_count = client
                    .peer()
                    .list_tools(Default::default())
                    .await
                    .map(|r| r.tools.len())
                    .unwrap_or(0);

                tracing::info!(server_id = %id, tool_count, "MCP server started");

                let mut entries = self.entries.write().await;
                if let Some(entry) = entries.get_mut(id) {
                    entry.client = Some(client);
                    entry.last_error = None;
                    entry.tool_count = Some(tool_count);
                }
                Ok(())
            }
            Err(e) => {
                let mut entries = self.entries.write().await;
                if let Some(entry) = entries.get_mut(id) {
                    entry.last_error = Some(e.to_string());
                }
                tracing::error!(server_id = %id, error = %e, "Failed to start MCP server");
                Err(e)
            }
        }
    }

    /// Stop a specific MCP server by ID.
    /// Idempotent — returns Ok if the server is not running.
    pub async fn stop_server(&self, id: &str) -> anyhow::Result<()> {
        let client = {
            let mut entries = self.entries.write().await;
            let entry = entries
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("Unknown MCP server: {id}"))?;
            entry.client.take()
        };

        if let Some(client) = client {
            if let Err(e) = client.cancel().await {
                tracing::warn!(server_id = %id, error = %e, "Error during MCP server shutdown");
            } else {
                tracing::info!(server_id = %id, "MCP server stopped");
            }

            let mut entries = self.entries.write().await;
            if let Some(entry) = entries.get_mut(id) {
                entry.tool_count = None;
            }
        }
        Ok(())
    }

    /// Stop all running servers (for shutdown).
    pub async fn stop_all(&self) {
        let clients: Vec<(String, RunningService<RoleClient, ()>)> = {
            let mut entries = self.entries.write().await;
            entries
                .iter_mut()
                .filter_map(|(id, entry)| {
                    entry.client.take().map(|c| (id.clone(), c))
                })
                .collect()
        };

        for (id, client) in clients {
            if let Err(e) = client.cancel().await {
                tracing::warn!(server_id = %id, error = %e, "Error during MCP server shutdown");
            } else {
                tracing::info!(server_id = %id, "MCP server stopped");
            }
        }
    }

    /// Get status of all servers.
    pub async fn list_status(&self) -> Vec<(String, String, McpServerStatus)> {
        let entries = self.entries.read().await;
        entries
            .iter()
            .map(|(id, entry)| {
                let status = if entry.client.is_some() {
                    McpServerStatus::Running {
                        tool_count: entry.tool_count.unwrap_or(0),
                    }
                } else if let Some(ref err) = entry.last_error {
                    McpServerStatus::Error {
                        message: err.clone(),
                    }
                } else {
                    McpServerStatus::Stopped
                };
                (id.clone(), entry.config.name.clone(), status)
            })
            .collect()
    }

    /// List all tools from a specific running server.
    pub async fn list_tools(&self, id: &str) -> anyhow::Result<Vec<Tool>> {
        let peer: Peer<RoleClient> = {
            let entries = self.entries.read().await;
            let entry = entries
                .get(id)
                .with_context(|| format!("Unknown MCP server: {id}"))?;
            let client = entry
                .client
                .as_ref()
                .with_context(|| format!("MCP server {id} is not running"))?;
            client.peer().clone()
        };

        let result = peer
            .list_tools(Default::default())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list tools from '{id}': {e}"))?;

        // Update cached tool count
        let count = result.tools.len();
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.get_mut(id) {
            entry.tool_count = Some(count);
        }

        Ok(result.tools)
    }

    /// List all tools across all running servers (aggregated).
    pub async fn list_all_tools(&self) -> Vec<McpToolInfo> {
        // Collect IDs of running servers
        let running_ids: Vec<String> = {
            let entries = self.entries.read().await;
            entries
                .iter()
                .filter(|(_, e)| e.client.is_some())
                .map(|(id, _)| id.clone())
                .collect()
        };

        let mut all_tools = Vec::new();
        for id in &running_ids {
            match self.list_tools(id).await {
                Ok(tools) => {
                    for tool in tools {
                        all_tools.push(McpToolInfo {
                            server_id: id.clone(),
                            name: tool.name.to_string(),
                            description: tool.description.as_deref().map(|s| s.to_string()),
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!(server_id = %id, error = %e, "Failed to list tools");
                }
            }
        }
        all_tools
    }

    /// Call a tool on a specific server.
    pub async fn call_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> anyhow::Result<CallToolResult> {
        let peer: Peer<RoleClient> = {
            let entries = self.entries.read().await;
            let entry = entries
                .get(server_id)
                .with_context(|| format!("Unknown MCP server: {server_id}"))?;
            let client = entry
                .client
                .as_ref()
                .with_context(|| format!("MCP server {server_id} is not running"))?;
            client.peer().clone()
        };

        // CallToolRequestParams is #[non_exhaustive] — build via Default then mutate.
        let mut params = CallToolRequestParams::default();
        params.name = tool_name.to_string().into();
        params.arguments = arguments;

        let result = peer
            .call_tool(params)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to call tool '{tool_name}' on '{server_id}': {e}"))?;

        Ok(result)
    }

    /// Get a config for a specific server.
    pub async fn get_config(&self, id: &str) -> Option<McpServerConfig> {
        let entries = self.entries.read().await;
        entries.get(id).map(|e| e.config.clone())
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(id: &str, name: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.to_string(),
            name: name.to_string(),
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            enabled: true,
            expose_tools: true,
        }
    }

    #[tokio::test]
    async fn load_configs_populates_entries() {
        let mgr = McpManager::new();
        let configs = vec![
            make_config("srv1", "Server 1"),
            make_config("srv2", "Server 2"),
        ];
        mgr.load_configs(&configs).await;

        let status = mgr.list_status().await;
        assert_eq!(status.len(), 2);
        assert!(status.iter().all(|(_, _, s)| matches!(s, McpServerStatus::Stopped)));
    }

    #[tokio::test]
    async fn load_configs_clears_existing() {
        let mgr = McpManager::new();
        mgr.load_configs(&[make_config("srv1", "Server 1")])
            .await;
        assert_eq!(mgr.list_status().await.len(), 1);

        // Reload with different configs
        mgr.load_configs(&[make_config("srv2", "Server 2")])
            .await;
        let status = mgr.list_status().await;
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].0, "srv2");
    }

    #[tokio::test]
    async fn get_config_returns_loaded_config() {
        let mgr = McpManager::new();
        mgr.load_configs(&[make_config("srv1", "My Server")])
            .await;

        let config = mgr.get_config("srv1").await;
        assert!(config.is_some());
        assert_eq!(config.unwrap().name, "My Server");

        assert!(mgr.get_config("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn list_status_reports_stopped_for_unstarted() {
        let mgr = McpManager::new();
        mgr.load_configs(&[make_config("srv1", "Server 1")])
            .await;

        let status = mgr.list_status().await;
        assert_eq!(status.len(), 1);
        assert!(matches!(status[0].2, McpServerStatus::Stopped));
    }

    #[tokio::test]
    async fn reload_configs_preserves_running_metadata() {
        let mgr = McpManager::new();
        mgr.load_configs(&[make_config("srv1", "Original")])
            .await;

        // Reload with updated config for same ID plus a new one
        mgr.reload_configs(&[
            McpServerConfig {
                id: "srv1".to_string(),
                name: "Updated".to_string(),
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
                cwd: None,
                enabled: false,
                expose_tools: false,
            },
            make_config("srv2", "New Server"),
        ])
        .await;

        let config = mgr.get_config("srv1").await.unwrap();
        assert_eq!(config.name, "Updated");
        assert!(!config.enabled);

        let status = mgr.list_status().await;
        assert_eq!(status.len(), 2);
    }

    #[tokio::test]
    async fn reload_configs_removes_deleted_servers() {
        let mgr = McpManager::new();
        mgr.load_configs(&[make_config("srv1", "A"), make_config("srv2", "B")])
            .await;

        mgr.reload_configs(&[make_config("srv1", "A")])
            .await;

        let status = mgr.list_status().await;
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].0, "srv1");
    }

    #[tokio::test]
    async fn start_unknown_server_returns_error() {
        let mgr = McpManager::new();
        let result = mgr.start_server("nonexistent").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown MCP server"));
    }

    #[tokio::test]
    async fn stop_unknown_server_returns_error() {
        let mgr = McpManager::new();
        let result = mgr.stop_server("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn stop_all_on_empty_manager_is_noop() {
        let mgr = McpManager::new();
        mgr.stop_all().await;
        assert!(mgr.list_status().await.is_empty());
    }
}
