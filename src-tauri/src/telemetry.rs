//! OpenTelemetry distributed tracing infrastructure.
//!
//! This module provides scaffolding for OTLP trace export. Full
//! integration requires adding `opentelemetry`, `opentelemetry-otlp`,
//! and `tracing-opentelemetry` to Cargo.toml, then wiring the OTLP
//! layer into the global tracing subscriber in `build_infra`.
//!
//! When `OTEL_EXPORTER_OTLP_ENDPOINT` is not set, tracing works normally
//! via the `tracing_subscriber::fmt` layer only.

/// Check whether OTLP export is configured.
///
/// Returns `true` when the `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable
/// is set, indicating the operator wants spans exported to an OTLP collector.
pub fn is_otlp_enabled() -> bool {
    std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok()
}

/// Initialize OpenTelemetry tracing if configured.
///
/// When `OTEL_EXPORTER_OTLP_ENDPOINT` is set, logs an informational message.
/// Full OTLP export requires adding the `opentelemetry-otlp` and
/// `tracing-opentelemetry` crates to Cargo.toml.
///
/// # TODO
/// Wire OTLP layer into `tracing_subscriber`:
/// ```ignore
/// let tracer = opentelemetry_otlp::new_pipeline()
///     .tracing()
///     .with_exporter(opentelemetry_otlp::new_exporter().tonic())
///     .install_batch(opentelemetry_sdk::runtime::Tokio)?;
/// let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
/// ```
pub fn init_tracing() {
    if is_otlp_enabled() {
        tracing::info!(
            "OTLP endpoint detected — OpenTelemetry export will be enabled \
             once the opentelemetry-otlp crate is added to Cargo.toml"
        );
    } else {
        tracing::info!(
            "OpenTelemetry not configured — set OTEL_EXPORTER_OTLP_ENDPOINT to enable"
        );
    }
}

/// Extract W3C TraceContext information from incoming request headers.
///
/// Logs the `traceparent` header at debug level when present. Full
/// context propagation requires an `opentelemetry::propagation::TextMapPropagator`
/// wired into middleware.
pub fn extract_otel_context(headers: &axum::http::HeaderMap) -> tracing::Span {
    if let Some(traceparent) = headers
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
    {
        tracing::debug!(traceparent = %traceparent, "Incoming W3C traceparent header");
    }
    tracing::Span::current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_otlp_enabled_returns_false_without_env() {
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        assert!(!is_otlp_enabled());
    }

    #[test]
    fn is_otlp_enabled_returns_true_with_env() {
        std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4317");
        assert!(is_otlp_enabled());
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    }

    #[test]
    fn extract_otel_context_handles_no_header() {
        let headers = axum::http::HeaderMap::new();
        let span = extract_otel_context(&headers);
        // Should not panic; span is always current
        let _ = span;
    }

    #[test]
    fn extract_otel_context_logs_traceparent() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "traceparent",
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
                .parse()
                .unwrap(),
        );
        let span = extract_otel_context(&headers);
        let _ = span;
    }
}
