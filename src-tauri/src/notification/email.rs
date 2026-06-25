use lettre::message::header::ContentType;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

/// Errors emitted by the email notifier.
#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error("invalid from address: {0}")]
    InvalidFrom(String),
    #[error("invalid recipient address: {0}")]
    InvalidRecipient(String),
    #[error("smtp transport error: {0}")]
    Smtp(String),
    #[error("email build error: {0}")]
    Build(String),
    #[error("invalid sender address: {0}")]
    InvalidSender(String),
}

impl From<lettre::transport::smtp::Error> for NotificationError {
    fn from(e: lettre::transport::smtp::Error) -> Self {
        Self::Smtp(e.to_string())
    }
}

impl From<lettre::error::Error> for NotificationError {
    fn from(e: lettre::error::Error) -> Self {
        Self::Build(e.to_string())
    }
}

/// SMTP email notifier. Cloning is cheap — the underlying transport is
/// held inside an `Arc` and reconnects lazily per send.
#[derive(Clone)]
pub struct EmailNotifier {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
}

impl EmailNotifier {
    /// Build a new notifier from the supplied SMTP configuration.
    ///
    /// `host` is the SMTP server hostname (e.g. `"smtp.gmail.com"`).
    /// `port` is typically 587 for STARTTLS or 465 for implicit TLS.
    /// When `use_tls` is true, STARTTLS is enabled on the connection.
    pub fn new(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
        from: &str,
        use_tls: bool,
    ) -> Result<Self, NotificationError> {
        // Validate the from address up front so a misconfigured gateway
        // fails loudly at startup rather than on the first notification.
        if from.parse::<Mailbox>().is_err() {
            return Err(NotificationError::InvalidFrom(from.to_string()));
        }

        let mut builder = if use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                .map_err(|e| NotificationError::Smtp(e.to_string()))?
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::relay(host)
                .map_err(|e| NotificationError::Smtp(e.to_string()))?
        };
        builder = builder.port(port);

        if !username.is_empty() {
            builder =
                builder.credentials(Credentials::new(username.to_string(), password.to_string()));
        }

        Ok(Self {
            transport: builder.build(),
            from: from.to_string(),
        })
    }

    /// Send a plain-text email to a single recipient.
    pub async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), NotificationError> {
        let to_mailbox: Mailbox = to
            .parse()
            .map_err(|_| NotificationError::InvalidRecipient(to.to_string()))?;

        let email = Message::builder()
            .from(
                self.from
                    .parse()
                    .map_err(|_| NotificationError::InvalidSender(self.from.clone()))?,
            )
            .to(to_mailbox)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())?;

        self.transport.send(email).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_from_address() {
        let result = EmailNotifier::new("smtp.example.com", 587, "u", "p", "not-an-email", true);
        assert!(matches!(result, Err(NotificationError::InvalidFrom(_))));
    }

    #[test]
    fn accepts_valid_configuration() {
        let result = EmailNotifier::new(
            "smtp.example.com",
            587,
            "alerts@example.com",
            "secret",
            "alerts@example.com",
            true,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn builds_without_tls() {
        let result =
            EmailNotifier::new("smtp.example.com", 25, "", "", "alerts@example.com", false);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn send_rejects_invalid_recipient() {
        let notifier = EmailNotifier::new(
            "smtp.example.com",
            587,
            "alerts@example.com",
            "secret",
            "alerts@example.com",
            true,
        )
        .expect("valid config");

        let result = notifier
            .send("definitely-not-an-email", "subj", "body")
            .await;
        assert!(matches!(
            result,
            Err(NotificationError::InvalidRecipient(_))
        ));
    }
}
