use serde::{Deserialize, Serialize};

/// Enterprise authentication configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    /// LDAP/Active Directory settings. None = LDAP disabled.
    #[serde(default)]
    pub ldap: Option<LdapConfig>,
    /// OIDC SSO settings. None = OIDC disabled.
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
}

/// LDAP/Active Directory authentication configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapConfig {
    /// LDAP server URL (e.g., `ldap://dc01.corp.local:389` or `ldaps://dc01.corp.local:636`).
    pub url: String,
    /// Bind DN template with `{username}` placeholder.
    /// Example: `cn={username},ou=users,dc=corp,dc=local`
    /// For Active Directory userPrincipalName style, use `{username}@corp.local`.
    pub bind_dn_template: String,
    /// Whether to upgrade the connection with StartTLS before binding (recommended).
    #[serde(default = "default_ldap_starttls")]
    pub starttls: bool,
    /// Default virtual key group to assign to LDAP-provisioned users.
    #[serde(default = "default_ldap_group")]
    pub default_group: String,
    /// Connection timeout in seconds (default 10).
    #[serde(default = "default_ldap_timeout_secs")]
    pub timeout_secs: u64,
}

impl LdapConfig {
    /// Build the bind DN by substituting the username into the template.
    ///
    /// The username is escaped according to RFC 4514 rules to prevent LDAP DN
    /// injection. The following characters are escaped with a backslash:
    /// - Comma (`,`) — DN separator
    /// - Plus (`+`) — multi-valued RDN separator
    /// - Double-quote (`"`) — quoting character
    /// - Backslash (`\`) — escape character
    /// - Less-than (`<`) — comparison operator
    /// - Greater-than (`>`) — comparison operator
    /// - Semicolon (`;`) — hierarchy separator
    /// - Null (`\0`) — string terminator
    ///
    /// Leading/trailing whitespace and the `#` character at the start of a
    /// value component are also escaped.
    pub fn build_bind_dn(&self, username: &str) -> String {
        let escaped = escape_ldap_dn(username);
        self.bind_dn_template.replace("{username}", &escaped)
    }
}

/// OIDC / OAuth2 SSO configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    /// Issuer URL (e.g., `https://login.microsoftonline.com/{tenant}/v2.0`).
    pub issuer: String,
    /// OAuth2 client ID registered with the IdP.
    pub client_id: String,
    /// OAuth2 client secret (for confidential clients).
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Redirect URI registered with the IdP (must match exactly).
    pub redirect_uri: String,
    /// Requested scopes (default: `["openid", "email", "profile"]`).
    #[serde(default = "default_oidc_scopes")]
    pub scopes: Vec<String>,
}

/// Config entry for a role-based admin token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminTokenConfig {
    pub token: String,
    pub role: String,
}

// ── Default value functions ───────────────────────────

pub(crate) fn default_ldap_starttls() -> bool {
    false
}

pub(crate) fn default_ldap_group() -> String {
    "ldap".to_string()
}

pub(crate) fn default_ldap_timeout_secs() -> u64 {
    10
}

pub(crate) fn default_oidc_scopes() -> Vec<String> {
    vec![
        "openid".to_string(),
        "email".to_string(),
        "profile".to_string(),
    ]
}

/// Escape a string for safe inclusion in an LDAP distinguished name (RFC 4514).
pub(crate) fn escape_ldap_dn(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    let chars: Vec<char> = input.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        let is_first = i == 0;
        let is_last = i == chars.len() - 1;
        match ch {
            ',' | '+' | '"' | '\\' | '<' | '>' | ';' => {
                out.push('\\');
                out.push(*ch);
            }
            '#' if is_first => {
                out.push_str("\\#");
            }
            ' ' if is_first || is_last => {
                out.push_str("\\ ");
            }
            '\0' => {
                out.push_str("\\00");
            }
            _ => {
                out.push(*ch);
            }
        }
    }
    out
}
