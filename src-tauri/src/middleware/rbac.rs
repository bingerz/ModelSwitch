use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    SuperAdmin,
    KeyManager,
    Auditor,
}

impl Role {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "super_admin" | "superadmin" | "admin" => Some(Self::SuperAdmin),
            "key_manager" | "keymanager" => Some(Self::KeyManager),
            "auditor" | "audit" => Some(Self::Auditor),
            _ => None,
        }
    }
}

/// Check if the path targets a virtual-keys endpoint.
/// Anchored to known API prefixes to prevent any substring matching.
fn is_virtual_keys_path(path: &str) -> bool {
    const VK_PREFIXES: [&str; 2] = ["/api/virtual-keys", "/v1/api/virtual-keys"];
    VK_PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{}/", prefix)))
}

/// Check if a role is permitted to perform an operation.
/// `path` is the full request path (e.g., "/api/virtual-keys").
pub fn is_permitted(method: &Method, path: &str, role: Role) -> bool {
    match role {
        Role::SuperAdmin => true,
        Role::KeyManager => {
            if method.is_safe() {
                return true;
            }
            // KeyManager can write to virtual-keys endpoints
            is_virtual_keys_path(path)
        }
        Role::Auditor => method.is_safe(),
    }
}

/// RBAC middleware — checks role-based permissions after auth middleware.
///
/// Reads the injected [`Role`] from request extensions. If no role is present
/// (open-proxy mode / no auth configured), the request passes through without
/// any permission check, preserving backward compatibility.
pub async fn rbac_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, &'static str)> {
    // If no role extension was injected, auth is not enforcing tokens — pass through.
    let role = match req.extensions().get::<Role>() {
        Some(r) => *r,
        None => return Ok(next.run(req).await),
    };

    let path = req.uri().path();
    if is_permitted(req.method(), path, role) {
        Ok(next.run(req).await)
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "Insufficient permissions for this role",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn super_admin_permitted_for_everything() {
        assert!(is_permitted(
            &Method::GET,
            "/api/virtual-keys",
            Role::SuperAdmin
        ));
        assert!(is_permitted(
            &Method::POST,
            "/api/channels",
            Role::SuperAdmin
        ));
        assert!(is_permitted(
            &Method::DELETE,
            "/api/virtual-keys/123",
            Role::SuperAdmin
        ));
    }

    #[test]
    fn auditor_can_read_but_not_write() {
        assert!(is_permitted(
            &Method::GET,
            "/api/virtual-keys",
            Role::Auditor
        ));
        assert!(is_permitted(&Method::GET, "/api/channels", Role::Auditor));
        assert!(!is_permitted(
            &Method::POST,
            "/api/virtual-keys",
            Role::Auditor
        ));
        assert!(!is_permitted(
            &Method::DELETE,
            "/api/channels/1",
            Role::Auditor
        ));
    }

    #[test]
    fn key_manager_can_read_all_and_write_vk_only() {
        assert!(is_permitted(
            &Method::GET,
            "/api/virtual-keys",
            Role::KeyManager
        ));
        assert!(is_permitted(
            &Method::GET,
            "/api/channels",
            Role::KeyManager
        ));
        assert!(is_permitted(
            &Method::POST,
            "/api/virtual-keys",
            Role::KeyManager
        ));
        assert!(is_permitted(
            &Method::PUT,
            "/api/virtual-keys/123",
            Role::KeyManager
        ));
        assert!(is_permitted(
            &Method::DELETE,
            "/api/virtual-keys/123",
            Role::KeyManager
        ));
        assert!(is_permitted(
            &Method::POST,
            "/api/virtual-keys/batch",
            Role::KeyManager
        ));
        assert!(!is_permitted(
            &Method::POST,
            "/api/channels",
            Role::KeyManager
        ));
        assert!(!is_permitted(
            &Method::DELETE,
            "/api/channels/1",
            Role::KeyManager
        ));
        assert!(!is_permitted(
            &Method::PUT,
            "/api/guardrails",
            Role::KeyManager
        ));
    }

    #[test]
    fn key_manager_substring_bypass_prevented() {
        // These should NOT match — substring bypass attempt
        assert!(!is_permitted(
            &Method::POST,
            "/api/channels/virtual-keys-foo",
            Role::KeyManager
        ));
        assert!(!is_permitted(
            &Method::DELETE,
            "/api/channels/1/virtual-keys-backdoor",
            Role::KeyManager
        ));
        assert!(!is_permitted(
            &Method::PUT,
            "/v1/api/not-virtual-keys/at-all",
            Role::KeyManager
        ));
        // These SHOULD match — legitimate VK paths
        assert!(is_permitted(
            &Method::POST,
            "/api/virtual-keys",
            Role::KeyManager
        ));
        assert!(is_permitted(
            &Method::PUT,
            "/api/virtual-keys/123",
            Role::KeyManager
        ));
        assert!(is_permitted(
            &Method::POST,
            "/v1/api/virtual-keys/batch",
            Role::KeyManager
        ));
    }

    #[test]
    fn role_from_str_parses_variants() {
        assert_eq!(Role::from_str("super_admin"), Some(Role::SuperAdmin));
        assert_eq!(Role::from_str("SuperAdmin"), Some(Role::SuperAdmin));
        assert_eq!(Role::from_str("admin"), Some(Role::SuperAdmin));
        assert_eq!(Role::from_str("key_manager"), Some(Role::KeyManager));
        assert_eq!(Role::from_str("keymanager"), Some(Role::KeyManager));
        assert_eq!(Role::from_str("auditor"), Some(Role::Auditor));
        assert_eq!(Role::from_str("invalid"), None);
    }
}
