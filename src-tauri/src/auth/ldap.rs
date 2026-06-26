//! LDAP/Active Directory authentication.
//!
//! Uses simple bind (DN + password) to verify credentials against an LDAP
//! directory server. The bind DN is constructed from a template with the
//! username escaped per RFC 4514 to prevent DN injection.

use std::time::Duration;

use ldap3::{Ldap, LdapConnAsync, LdapConnSettings, LdapResult};

use crate::config::LdapConfig;

use super::{AuthSource, AuthUserInfo};

/// LDAP authentication errors.
#[derive(Debug, thiserror::Error)]
pub enum LdapAuthError {
    /// The server returned a non-zero result code (invalid credentials, account locked, etc.).
    #[error("LDAP bind failed: rc={rc}, text={text}")]
    BindFailed { rc: u32, text: String },
    /// Network or connection error.
    #[error("LDAP connection error: {0}")]
    Connection(#[from] ldap3::LdapError),
    /// The connection driver task terminated unexpectedly.
    #[error("LDAP connection driver error: {0}")]
    Driver(String),
}

/// Authenticator for LDAP/AD simple bind.
pub struct LdapAuthenticator {
    config: LdapConfig,
}

impl LdapAuthenticator {
    /// Create a new authenticator with the given LDAP configuration.
    pub fn new(config: LdapConfig) -> Self {
        Self { config }
    }

    /// Authenticate a user by attempting a simple bind with their credentials.
    ///
    /// On success, returns [`AuthUserInfo`] with the username and default group
    /// from the configuration. The password is never stored.
    pub async fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AuthUserInfo, LdapAuthError> {
        let settings = self.build_settings();
        let bind_dn = self.config.build_bind_dn(username);

        let (conn, mut ldap) = LdapConnAsync::with_settings(settings, &self.config.url).await?;

        // Spawn the connection driver — required by ldap3 for async operation.
        let driver_handle = tokio::spawn(async move {
            if let Err(e) = conn.drive().await {
                tracing::error!("LDAP connection driver error: {e}");
            }
        });

        let result = self.do_bind(&mut ldap, &bind_dn, password).await;

        // Always attempt to unbind, regardless of bind outcome.
        let _ = ldap.unbind().await;

        // Wait for the driver task to finish.
        let _ = driver_handle.await;

        match result {
            Ok(ldap_result) if ldap_result.rc == 0 => Ok(AuthUserInfo {
                id: bind_dn,
                username: username.to_string(),
                email: None,
                groups: vec![self.config.default_group.clone()],
                source: AuthSource::Ldap,
            }),
            Ok(ldap_result) => Err(LdapAuthError::BindFailed {
                rc: ldap_result.rc,
                text: ldap_result.text,
            }),
            Err(e) => Err(e),
        }
    }

    /// Build the LDAP connection settings from configuration.
    fn build_settings(&self) -> LdapConnSettings {
        let mut settings = LdapConnSettings::new();
        if self.config.starttls {
            settings = settings.set_starttls(true);
        }
        settings = settings.set_conn_timeout(Duration::from_secs(self.config.timeout_secs));
        settings
    }

    /// Perform the simple bind operation.
    async fn do_bind(
        &self,
        ldap: &mut Ldap,
        bind_dn: &str,
        password: &str,
    ) -> Result<LdapResult, LdapAuthError> {
        let result = ldap.simple_bind(bind_dn, password).await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> LdapConfig {
        LdapConfig {
            url: "ldap://localhost:389".into(),
            bind_dn_template: "cn={username},ou=users,dc=test,dc=local".into(),
            starttls: false,
            default_group: "test-users".into(),
            timeout_secs: 2,
        }
    }

    #[test]
    fn authenticator_builds_bind_dn_from_template() {
        let cfg = test_config();
        let auth = LdapAuthenticator::new(cfg);
        // The config field is accessible and the authenticator stores it.
        assert_eq!(
            auth.config.build_bind_dn("alice"),
            "cn=alice,ou=users,dc=test,dc=local"
        );
    }

    #[test]
    fn authenticator_escapes_username_in_bind_dn() {
        let cfg = test_config();
        let auth = LdapAuthenticator::new(cfg);
        assert_eq!(
            auth.config.build_bind_dn("user,evil"),
            r"cn=user\,evil,ou=users,dc=test,dc=local"
        );
    }

    #[test]
    fn authenticator_stores_config_correctly() {
        let cfg = test_config();
        let auth = LdapAuthenticator::new(cfg);
        assert_eq!(auth.config.url, "ldap://localhost:389");
        assert!(!auth.config.starttls);
        assert_eq!(auth.config.default_group, "test-users");
        assert_eq!(auth.config.timeout_secs, 2);
    }

    #[test]
    fn build_settings_includes_starttls_when_enabled() {
        let cfg = LdapConfig {
            url: "ldaps://dc01.corp.local:636".into(),
            bind_dn_template: "{username}@corp.local".into(),
            starttls: true,
            default_group: "ad".into(),
            timeout_secs: 5,
        };
        let auth = LdapAuthenticator::new(cfg);
        let settings = auth.build_settings();
        assert!(settings.starttls());
    }

    #[test]
    fn build_settings_excludes_starttls_when_disabled() {
        let cfg = test_config();
        let auth = LdapAuthenticator::new(cfg);
        let settings = auth.build_settings();
        assert!(!settings.starttls());
    }

    #[tokio::test]
    async fn authenticate_fails_on_unreachable_server() {
        // Point at a port that's almost certainly not running LDAP.
        let cfg = LdapConfig {
            url: "ldap://127.0.0.1:1".into(),
            bind_dn_template: "cn={username},dc=test".into(),
            starttls: false,
            default_group: "test".into(),
            timeout_secs: 1,
        };
        let auth = LdapAuthenticator::new(cfg);
        let result = auth.authenticate("user", "pass").await;
        assert!(result.is_err());
        // The error should be a connection error, not a bind error.
        match result.unwrap_err() {
            LdapAuthError::Connection(_) => { /* expected */ }
            other => panic!("expected Connection error, got: {other}"),
        }
    }
}
