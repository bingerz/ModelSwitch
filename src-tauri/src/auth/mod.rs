//! Enterprise authentication module.
//!
//! Supports LDAP/Active Directory bind authentication and OIDC SSO (scaffold).

pub mod ldap;
pub mod oidc;

/// User information returned after successful authentication.
#[derive(Debug, Clone)]
pub struct AuthUserInfo {
    /// Unique identifier for the user (e.g., DN for LDAP, sub for OIDC).
    pub id: String,
    /// Display name or username.
    pub username: String,
    /// Email address, if available.
    pub email: Option<String>,
    /// Groups the user belongs to.
    pub groups: Vec<String>,
    /// The authentication source that verified this user.
    pub source: AuthSource,
}

/// The authentication mechanism used to verify a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSource {
    /// LDAP/Active Directory simple bind.
    Ldap,
    /// OIDC / OAuth2 authorization code flow.
    Oidc,
}
