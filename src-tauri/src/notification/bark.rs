use reqwest::Client;

/// Send a Bark push notification.
///
/// Bark URL format: `https://api.day.app/{key}`.
/// The title is URL-encoded and appended as a path segment (Bark's
/// standard convention). The event body is sent as JSON `{ "body": ... }`
/// so receivers get the full structured payload.
pub async fn send_bark(
    http: &Client,
    bark_url: &str,
    title: &str,
    body: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "{}/{}",
        bark_url.trim_end_matches('/'),
        urlencoding::encode(title)
    );
    let resp = http
        .post(&url)
        .header("Content-Type", "application/json; charset=utf-8")
        .body(serde_json::json!({ "body": body }).to_string())
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(format!("Bark returned {}", resp.status()).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn sends_bark_notification_on_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/test-key/Budget%20at%2080%25"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let http = Client::new();
        let base = format!("{}/test-key", server.uri());
        let result = send_bark(&http, &base, "Budget at 80%", "details").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn returns_error_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let http = Client::new();
        let base = format!("{}/missing", server.uri());
        let result = send_bark(&http, &base, "title", "body").await;
        assert!(result.is_err());
    }

    #[test]
    fn encodes_title_correctly() {
        // Sanity check that urlencoding::encode produces percent-encoded output.
        let encoded = urlencoding::encode("hello world!");
        assert_eq!(encoded, "hello%20world%21");
    }
}
