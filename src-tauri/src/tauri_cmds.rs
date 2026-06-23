//! Tauri commands and gateway lifecycle management.

use crate::admin::{McpServerResponse, McpToolResponse};
use crate::config;
use crate::proxy::AppState;
use crate::server::start_gateway;
use crate::GatewayHandles;

use serde::Serialize;
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, Notify};

// -- Gateway lifecycle management -------------------------------------------

struct GwInner {
    running: bool,
    shutdown: Option<Arc<Notify>>,
    stopped_rx: Option<oneshot::Receiver<()>>,
}

pub(crate) struct GatewayManager {
    pub(crate) app_state: Arc<AppState>,
    host: String,
    port: u16,
    drain_timeout_secs: u64,
    web_console_dir: Option<String>,
    inner: Mutex<GwInner>,
}

impl GatewayManager {
    pub(crate) fn new(handles: GatewayHandles) -> Self {
        Self {
            app_state: handles.state,
            host: handles.host,
            port: handles.port,
            drain_timeout_secs: handles.drain_timeout_secs,
            web_console_dir: handles.web_console_dir,
            inner: Mutex::new(GwInner {
                running: false,
                shutdown: None,
                stopped_rx: None,
            }),
        }
    }

    /// Synchronous shutdown: fires the Notify and cleans up PID file.
    /// Called from Drop or window Destroyed event -- no async context available.
    pub(crate) fn force_shutdown(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.running {
            return;
        }
        tracing::info!("Gateway force-shutdown triggered");
        inner.running = false;

        // Persist quota data before shutting down (synchronous, no async needed)
        self.app_state.billing.quota_store.persist_sync();

        if let Some(shutdown) = inner.shutdown.take() {
            shutdown.notify_waiters();
            tracing::info!("Gateway force-shutdown triggered");
        }
        inner.stopped_rx = None;

        // Clean up PID file
        let pid_path = config::app_config_dir().join("gateway.pid");
        let _ = std::fs::remove_file(&pid_path);
    }
}

impl Drop for GatewayManager {
    fn drop(&mut self) {
        self.force_shutdown();
    }
}

#[derive(Serialize)]
pub(crate) struct GatewayStatus {
    running: bool,
    host: String,
    port: u16,
}

#[tauri::command]
pub(crate) async fn gateway_status(
    manager: tauri::State<'_, GatewayManager>,
) -> Result<GatewayStatus, String> {
    let inner = manager.inner.lock().unwrap_or_else(|e| e.into_inner());
    Ok(GatewayStatus {
        running: inner.running,
        host: manager.host.clone(),
        port: manager.port,
    })
}

#[tauri::command]
pub(crate) async fn gateway_start(manager: tauri::State<'_, GatewayManager>) -> Result<(), String> {
    {
        let inner = manager.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.running {
            return Err("Gateway already running".into());
        }
    }

    let shutdown = Arc::new(Notify::new());
    let (stopped_tx, stopped_rx) = oneshot::channel();
    let (bind_ok_tx, bind_ok_rx) = oneshot::channel::<Result<(), String>>();

    let state = Arc::clone(&manager.app_state);
    let host = manager.host.clone();
    let port = manager.port;
    let drain = manager.drain_timeout_secs;
    let web_console_dir = manager.web_console_dir.clone();
    let shutdown_clone = Arc::clone(&shutdown);

    tauri::async_runtime::spawn(async move {
        start_gateway(
            state,
            &host,
            port,
            drain,
            Some(shutdown_clone),
            Some(bind_ok_tx),
            web_console_dir.as_deref(),
        )
        .await;
        let _ = stopped_tx.send(());
    });

    // Wait for the spawned task to confirm bind succeeded (or failed)
    match bind_ok_rx.await {
        Ok(Ok(())) => {
            let mut inner = manager.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.running = true;
            inner.shutdown = Some(shutdown);
            inner.stopped_rx = Some(stopped_rx);
            tracing::info!(
                "Gateway started successfully on {}:{}",
                manager.host,
                manager.port
            );
            Ok(())
        }
        Ok(Err(e)) => {
            tracing::error!("Gateway bind failed: {e}");
            Err(e)
        }
        Err(_) => {
            let msg = "Gateway task exited before bind".to_string();
            tracing::error!("{msg}");
            Err(msg)
        }
    }
}

#[tauri::command]
pub(crate) async fn gateway_stop(manager: tauri::State<'_, GatewayManager>) -> Result<(), String> {
    let (stopped_rx, shutdown) = {
        let mut inner = manager.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.running {
            return Err("Gateway not running".into());
        }
        inner.running = false;

        // Persist quota data before shutting down
        manager.app_state.billing.quota_store.persist_sync();

        let rx = inner.stopped_rx.take();
        let shutdown = inner.shutdown.take();
        (rx, shutdown)
    }; // MutexGuard dropped here

    if let Some(shutdown) = shutdown {
        shutdown.notify_waiters();
    }

    if let Some(rx) = stopped_rx {
        let timeout = std::time::Duration::from_secs(35);
        if tokio::time::timeout(timeout, rx).await.is_err() {
            tracing::warn!("Gateway stop timed out after {}s", timeout.as_secs());
        }
    }

    tracing::info!("Gateway stopped");
    Ok(())
}

#[tauri::command]
pub(crate) async fn gateway_restart(
    manager: tauri::State<'_, GatewayManager>,
) -> Result<(), String> {
    gateway_stop(manager.clone()).await?;
    // Small delay to ensure port is released
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    gateway_start(manager).await
}

#[tauri::command]
pub(crate) async fn app_quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub(crate) async fn app_hide(app: tauri::AppHandle) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
pub(crate) async fn mcp_list_servers(
    manager: tauri::State<'_, GatewayManager>,
) -> Result<Vec<McpServerResponse>, String> {
    let state = &manager.app_state;
    let statuses = state.mcp.mcp_manager.list_status().await;
    let mut responses = Vec::with_capacity(statuses.len());
    for (id, _name, status) in statuses {
        if let Some(config) = state.mcp.mcp_manager.get_config(&id).await {
            responses.push(McpServerResponse {
                id: config.id,
                name: config.name,
                command: config.command,
                args: config.args,
                env: config.env,
                cwd: config.cwd,
                enabled: config.enabled,
                expose_tools: config.expose_tools,
                status,
            });
        }
    }
    Ok(responses)
}

#[tauri::command]
pub(crate) async fn mcp_start_server(
    manager: tauri::State<'_, GatewayManager>,
    server_id: String,
) -> Result<(), String> {
    manager
        .app_state
        .mcp
        .mcp_manager
        .start_server(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn mcp_stop_server(
    manager: tauri::State<'_, GatewayManager>,
    server_id: String,
) -> Result<(), String> {
    manager
        .app_state
        .mcp
        .mcp_manager
        .stop_server(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn mcp_list_tools(
    manager: tauri::State<'_, GatewayManager>,
    server_id: String,
) -> Result<Vec<McpToolResponse>, String> {
    let tools = manager
        .app_state
        .mcp
        .mcp_manager
        .list_tools(&server_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(tools
        .iter()
        .map(|t| McpToolResponse {
            name: t.name.to_string(),
            description: t.description.as_deref().map(String::from),
        })
        .collect())
}
