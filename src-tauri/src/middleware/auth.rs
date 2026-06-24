use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::proxy::AppState;

// ---------------------------------------------------------------------------
// Rate limiting constants for brute-force protection
// ---------------------------------------------------------------------------

/// Maximum failed attempts before the IP is blocked.
const MAX_ATTEMPTS: u32 = 3;
/// Sliding window in which failures are counted.
const WINDOW_SECS: u64 = 10;
/// Duration the IP is blocked after exceeding the threshold.
const BLOCK_SECS: u64 = 60;
/// Entries older than this are purged to prevent unbounded memory growth.
const CLEANUP_SECS: u64 = 120;

/// Tracks failed authentication attempts per client IP.
struct AuthAttemptInfo {
    fail_count: u32,
    first_fail: Instant,
    blocked_until: Option<Instant>,
}

/// Global rate-limiter state keyed by client IP address.
static ATTEMPTS: OnceLock<Mutex<HashMap<String, AuthAttemptInfo>>> = OnceLock::new();

/// Access the global attempt map (initialised on first use).
fn attempts() -> &'static Mutex<HashMap<String, AuthAttemptInfo>> {
    ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Extract the client IP from `x-forwarded-for` (first IP) or `x-real-ip`,
/// falling back to `"unknown"` when neither header is present.
fn extract_client_ip(req: &Request<Body>) -> String {
    let headers = req.headers();

    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let ip = first.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }

    if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = xri.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }

    "unknown".to_string()
}

/// Returns `true` if the IP is currently blocked due to too many failures.
fn is_rate_limited(ip: &str) -> bool {
    let Ok(map) = attempts().lock() else {
        return false; // poisoned lock — fail open
    };
    if let Some(info) = map.get(ip) {
        if let Some(until) = info.blocked_until {
            return until > Instant::now();
        }
    }
    false
}

/// Record a failed authentication attempt, blocking the IP when the threshold
/// is reached. Stale entries are cleaned up to bound memory usage.
fn record_auth_failure(ip: &str) {
    let Ok(mut map) = attempts().lock() else {
        return;
    };
    let now = Instant::now();

    // Purge entries older than CLEANUP_SECS to prevent memory leaks.
    map.retain(|_, info| now.duration_since(info.first_fail).as_secs() < CLEANUP_SECS);

    let info = map.entry(ip.to_string()).or_insert(AuthAttemptInfo {
        fail_count: 0,
        first_fail: now,
        blocked_until: None,
    });

    // Reset the sliding window if the previous failures are stale.
    if now.duration_since(info.first_fail).as_secs() >= WINDOW_SECS {
        info.fail_count = 0;
        info.first_fail = now;
        info.blocked_until = None;
    }

    info.fail_count += 1;

    if info.fail_count >= MAX_ATTEMPTS {
        info.blocked_until = Some(now + Duration::from_secs(BLOCK_SECS));
    }
}

/// Clear the rate-limit state for an IP on successful authentication.
fn record_auth_success(ip: &str) {
    let Ok(mut map) = attempts().lock() else {
        return;
    };
    map.remove(ip);
}

/// Constant-time Bearer token verification to prevent timing attacks.
fn verify_bearer_token(header_value: &str, expected: &str) -> bool {
    if !header_value.starts_with("Bearer ") {
        return false;
    }
    use subtle::ConstantTimeEq;
    header_value.as_bytes()[7..]
        .ct_eq(expected.as_bytes())
        .into()
}

/// Bearer token auth middleware for /api/* routes.
/// If admin_token is configured, validates Authorization header.
/// If not configured, passes through (backward compatible).
///
/// Includes IP-based rate limiting: after [`MAX_ATTEMPTS`] failures within
/// [`WINDOW_SECS`] the IP is blocked for [`BLOCK_SECS`].
pub async fn admin_auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, &'static str)> {
    let token = match &state.security.admin_token {
        Some(t) => t,
        None => return Ok(next.run(req).await),
    };

    let ip = extract_client_ip(&req);

    // Reject early if the IP is already blocked.
    if is_rate_limited(&ip) {
        return Err((StatusCode::TOO_MANY_REQUESTS, "Too many failed attempts"));
    }

    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(h) if verify_bearer_token(h, token) => {
            record_auth_success(&ip);
            Ok(next.run(req).await)
        }
        _ => {
            record_auth_failure(&ip);
            Err((StatusCode::UNAUTHORIZED, "Unauthorized"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_correct_token() {
        assert!(verify_bearer_token("Bearer my-secret", "my-secret"));
    }

    #[test]
    fn rejects_wrong_token() {
        assert!(!verify_bearer_token("Bearer wrong", "my-secret"));
    }

    #[test]
    fn rejects_missing_prefix() {
        assert!(!verify_bearer_token("my-secret", "my-secret"));
    }

    #[test]
    fn rejects_empty_header() {
        assert!(!verify_bearer_token("", "my-secret"));
    }

    // -----------------------------------------------------------------------
    // Rate-limiting tests — each test uses a unique IP to avoid interference
    // from parallel test execution.
    // -----------------------------------------------------------------------

    /// Three rapid failures from the same IP should trigger blocking.
    #[test]
    fn rate_limiting_blocks_after_three_failures() {
        let ip = "10.0.0.1"; // unique per test

        // Clear any leftover state (safety, though unique IP is enough).
        {
            if let Ok(mut map) = attempts().lock() {
                map.remove(ip);
            }
        }

        assert!(!is_rate_limited(ip), "IP should not be blocked initially");

        record_auth_failure(ip);
        assert!(!is_rate_limited(ip), "1st failure — not blocked");

        record_auth_failure(ip);
        assert!(!is_rate_limited(ip), "2nd failure — not blocked");

        record_auth_failure(ip);
        assert!(
            is_rate_limited(ip),
            "3rd failure within window — IP must be blocked"
        );
    }

    /// A successful authentication clears the failure counter so the user
    /// is not penalised after recovering.
    #[test]
    fn rate_limiting_success_clears_counter() {
        let ip = "10.0.0.2";

        {
            if let Ok(mut map) = attempts().lock() {
                map.remove(ip);
            }
        }

        record_auth_failure(ip);
        record_auth_failure(ip);
        assert!(!is_rate_limited(ip));

        record_auth_success(ip);

        // Two more failures after success — should only be at 2, not blocked.
        record_auth_failure(ip);
        record_auth_failure(ip);
        assert!(
            !is_rate_limited(ip),
            "Counter was reset by success; 2 failures should not block"
        );

        // Third failure after reset triggers block.
        record_auth_failure(ip);
        assert!(is_rate_limited(ip));
    }
}
