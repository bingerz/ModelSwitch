mod admin;
mod channel;
pub mod config;
pub mod credential;
pub mod guardrails;
mod health;
pub mod http_pool;
mod log;
mod mcp;
pub mod metrics;
mod middleware;
pub mod model_registry;
pub mod notification;
pub mod persisted_store;
mod provider_budget;
mod proxy;
mod quota;
mod router;
pub mod virtual_key;
#[cfg(feature = "tauri")]
mod webview_login;

mod server;
mod shutdown;
pub mod telemetry;
#[cfg(feature = "tauri")]
mod tauri_cmds;
#[cfg(test)]
pub(crate) mod test_helpers;

use crate::proxy::AppState;
use std::sync::Arc;

// Re-export for CLI binary and external consumers
pub use server::start_gateway_services;
pub use shutdown::run_gateway;

/// Spawn a background task, working both in CLI (tokio runtime) and Tauri (main thread without
/// runtime).
pub(crate) fn spawn_bg<F>(future: F)
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(future);
        }
        Err(_) => {
            #[cfg(feature = "tauri")]
            {
                tauri::async_runtime::spawn(future);
            }
            #[cfg(not(feature = "tauri"))]
            {
                static RT: std::sync::OnceLock<tokio::runtime::Runtime> =
                    std::sync::OnceLock::new();
                let rt = RT.get_or_init(|| {
                    tokio::runtime::Runtime::new().expect("Failed to create background runtime")
                });
                rt.spawn(future);
            }
        }
    }
}

/// Shared gateway state returned by [`server::start_gateway_services`].
pub struct GatewayHandles {
    pub state: Arc<AppState>,
    pub host: String,
    pub port: u16,
    pub drain_timeout_secs: u64,
    pub web_console_dir: Option<String>,
    pub tls: crate::config::TlsConfig,
}

// -- Tauri entry point -------------------------------------------------------

#[cfg(feature = "tauri")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use channel::ChannelStatus;
    use tauri::Manager;
    use tauri_cmds::{
        app_hide, app_quit, gateway_restart, gateway_start, gateway_status, gateway_stop,
        mcp_list_servers, mcp_list_tools, mcp_start_server, mcp_stop_server, GatewayManager,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            webview_login::open_login_webview,
            quota::webview_scrape::scrape_webview_quota,
            gateway_status,
            gateway_start,
            gateway_stop,
            gateway_restart,
            app_quit,
            app_hide,
            mcp_list_servers,
            mcp_start_server,
            mcp_stop_server,
            mcp_list_tools,
        ])
        .setup(|app| {
            let handles = server::start_gateway_services(None);
            let manager = GatewayManager::new(handles);
            let state = Arc::clone(&manager.app_state);

            app.manage(manager);

            // Setup system tray
            let tray = tauri::tray::TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("ModelSwitch - Starting...")
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Background task: update tray tooltip with channel health
            let mgr = Arc::clone(&state.channel_mgr);
            spawn_bg(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    let channels = mgr.list().await;
                    let total = channels.len();
                    let healthy = channels
                        .iter()
                        .filter(|c| c.status == ChannelStatus::Healthy && c.enabled)
                        .count();
                    let tooltip = format!("ModelSwitch -- {}/{} channels healthy", healthy, total);
                    let _ = tray.set_tooltip(Some(&tooltip));
                }
            });

            // Intercept window close: prevent default and let frontend decide
            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    use tauri::Emitter;
                    match event {
                        tauri::WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            let _ = app_handle.emit("close-requested", ());
                        }
                        tauri::WindowEvent::Destroyed => {
                            tracing::warn!("Window Destroyed event -- calling force_shutdown");
                            // Safety net: ensure gateway stops when window is destroyed
                            // (e.g. OS shutdown, task manager close)
                            let manager = app_handle.state::<GatewayManager>();
                            manager.force_shutdown();
                        }
                        _ => {}
                    }
                });

                // Open DevTools in debug builds to help diagnose rendering issues
                #[cfg(debug_assertions)]
                {
                    window.open_devtools();
                }
            }

            // Gateway auto-start is handled by the frontend (App.tsx useEffect)
            // to avoid a race between Rust-side auto-start and frontend invoke.

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
