//! Signal handling and gateway run convenience.

use crate::server::start_gateway;
use crate::GatewayHandles;

pub(crate) async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received Ctrl+C, shutting down"),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down"),
    }
}

/// Start gateway from pre-built handles (convenience for CLI).
pub async fn run_gateway(handles: GatewayHandles) {
    let GatewayHandles {
        state,
        host,
        port,
        drain_timeout_secs,
        web_console_dir,
    } = handles;
    start_gateway(
        state,
        &host,
        port,
        drain_timeout_secs,
        None,
        None,
        web_console_dir.as_deref(),
    )
    .await;
}
