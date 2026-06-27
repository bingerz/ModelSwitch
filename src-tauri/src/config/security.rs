use serde::{Deserialize, Serialize};

/// Privacy guardrail configuration for the sanitizer middleware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanitizerConfig {
    /// Master switch. When false, the middleware is a no-op.
    #[serde(default = "default_sanitizer_enabled")]
    pub enabled: bool,
    /// Whether to actually redact matched secrets. When false, patterns are
    /// scanned for telemetry only without mutating the body.
    #[serde(default = "default_sanitizer_redact")]
    pub redact_secrets: bool,
    /// Scan SSE response streams for echoed secrets (default false for performance).
    #[serde(default)]
    pub scan_response: bool,
    /// User-supplied patterns in addition to the built-in catalog.
    #[serde(default)]
    pub custom_patterns: Vec<CustomPattern>,
}

impl Default for SanitizerConfig {
    fn default() -> Self {
        Self {
            enabled: default_sanitizer_enabled(),
            redact_secrets: default_sanitizer_redact(),
            scan_response: false,
            custom_patterns: vec![],
        }
    }
}

/// A user-defined sanitizer pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPattern {
    /// Human-readable label used in logs.
    pub name: String,
    /// Regular expression source. Invalid regexes are silently skipped at compile time.
    pub pattern: String,
    /// Replacement text written in place of each match.
    #[serde(default = "default_custom_replacement")]
    pub replacement: String,
}

// ── Default value functions ───────────────────────────

pub(crate) fn default_sanitizer_enabled() -> bool {
    true
}
pub(crate) fn default_sanitizer_redact() -> bool {
    true
}
pub(crate) fn default_custom_replacement() -> String {
    "[REDACTED]".to_string()
}
