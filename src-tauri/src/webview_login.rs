use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

struct ProviderConfig {
    login_url: &'static str,
    logged_in: fn(&str) -> bool,
}

fn claude_logged_in(url: &str) -> bool {
    url.contains("claude.ai") && !url.contains("/login") && !url.contains("/auth")
}

fn chatgpt_logged_in(url: &str) -> bool {
    url.contains("chatgpt.com") && !url.contains("/auth")
}

fn deepseek_logged_in(url: &str) -> bool {
    url.contains("chat.deepseek.com") && !url.contains("login")
}

fn get_provider_config(provider: &str) -> Option<ProviderConfig> {
    match provider {
        "anthropic" | "claude" => Some(ProviderConfig {
            login_url: "https://claude.ai/login",
            logged_in: claude_logged_in,
        }),
        "openai" | "chatgpt" => Some(ProviderConfig {
            login_url: "https://chatgpt.com/auth/login",
            logged_in: chatgpt_logged_in,
        }),
        "deepseek" => Some(ProviderConfig {
            login_url: "https://chat.deepseek.com/",
            logged_in: deepseek_logged_in,
        }),
        _ => None,
    }
}

#[tauri::command]
pub async fn open_login_webview(
    provider: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let config = get_provider_config(&provider).ok_or_else(|| {
        format!(
            "WebView login not supported for provider '{}'. Supported: claude, chatgpt, deepseek",
            provider
        )
    })?;

    let label = "login-webview";
    let login_url = config.login_url;
    let provider_clone = provider.clone();
    let logged_in_fn = config.logged_in;

    // Close any existing login window
    if let Some(existing) = app.get_webview_window(label) {
        let _ = existing.close();
    }

    let app_for_builder = app.clone();
    let _webview_window = WebviewWindowBuilder::new(
        &app_for_builder,
        label,
        WebviewUrl::External(login_url.parse().unwrap()),
    )
    .title(format!("Login to {}", provider))
    .inner_size(480.0, 700.0)
    .center()
    .focused(true)
    .on_navigation(move |url| {
        let url_str = url.to_string();

        if logged_in_fn(&url_str) {
            let app = app.clone();
            let provider_clone = provider_clone.clone();

            tauri::async_runtime::spawn(async move {
                // Give the page time to set cookies after login redirect
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;

                if let Some(webview) = app.get_webview_window(label) {
                    // Inject JS that sends cookies to our local gateway
                    let script = format!(
                        "fetch('http://127.0.0.1:8080/api/auth/cookies',{{\
                         method:'POST',\
                         headers:{{'Content-Type':'application/json'}},\
                         body:JSON.stringify({{provider:'{}',cookies:document.cookie}})\
                         }}).then(()=>{{\
                         document.title='Login Complete';\
                         }})",
                        provider_clone
                    );
                    let _ = webview.eval(&script);

                    // Wait for the fetch to complete, then close
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let _ = webview.close();
                }
            });
        }

        true // Allow navigation
    })
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}
