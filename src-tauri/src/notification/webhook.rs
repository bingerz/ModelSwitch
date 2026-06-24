use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::Value;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Send an HMAC-SHA256 signed webhook notification.
///
/// The payload is JSON-serialized and POSTed to `url`. When a `secret` is
/// provided, an `X-ModelSwitch-Signature: sha256=<hex>` header is added so the
/// receiver can verify authenticity.
pub async fn send_webhook(
    http: &Client,
    url: &str,
    secret: &Option<String>,
    payload: &Value,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let body = serde_json::to_string(payload)?;
    let mut req = http
        .post(url)
        .header("Content-Type", "application/json")
        .body(body.clone());

    if let Some(ref sec) = secret {
        let mut mac = HmacSha256::new_from_slice(sec.as_bytes())?;
        mac.update(body.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        req = req.header("X-ModelSwitch-Signature", format!("sha256={signature}"));
    }

    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(format!("Webhook returned {}", resp.status()).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn sends_unsigned_webhook_on_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .and(header("Content-Type", "application/json"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let http = Client::new();
        let payload = serde_json::json!({"event": "test"});
        let result = send_webhook(&http, &format!("{}/hook", server.uri()), &None, &payload).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn sends_signed_webhook_with_signature_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .and(header_exists("X-ModelSwitch-Signature"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let http = Client::new();
        let payload = serde_json::json!({"event": "signed"});
        let secret = Some("topsecret".to_string());
        let result =
            send_webhook(&http, &format!("{}/hook", server.uri()), &secret, &payload).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn returns_error_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let http = Client::new();
        let payload = serde_json::json!({"event": "fail"});
        let result = send_webhook(&http, &format!("{}/hook", server.uri()), &None, &payload).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("500"));
    }

    /// Helper — wiremock match for header existence (by name prefix).
    fn header_exists(name: &'static str) -> impl wiremock::Match {
        struct HeaderExists(&'static str);
        impl wiremock::Match for HeaderExists {
            fn matches(&self, request: &wiremock::Request) -> bool {
                request.headers.contains_key(self.0)
            }
        }
        HeaderExists(name)
    }
}
