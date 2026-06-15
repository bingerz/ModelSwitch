/// DouBao / Volcengine Ark quota scraping script.
/// Fetches usage/quota data from the Volcengine Ark console using session cookies.
pub const DOUBAO_USAGE_SCRIPT: &str = r#"
(async () => {
  try {
    // Try the Ark console internal API for usage/quota data
    const resp = await fetch('https://console.volcengine.com/ark/api/v1/usage/quota', {
      credentials: 'include',
      headers: { 'Accept': 'application/json' },
    });
    if (resp.ok) {
      const data = await resp.json();
      window.__tauri__.emit('webview-quota-result', {
        provider: 'doubao',
        success: true,
        data: data,
      });
      return;
    }

    // Fallback: try the billing summary endpoint
    const resp2 = await fetch('https://console.volcengine.com/ark/api/v1/billing/summary', {
      credentials: 'include',
      headers: { 'Accept': 'application/json' },
    });
    if (resp2.ok) {
      const data = await resp2.json();
      window.__tauri__.emit('webview-quota-result', {
        provider: 'doubao',
        success: true,
        data: data,
      });
      return;
    }

    // Last resort: scrape visible page text for balance info
    await new Promise(r => setTimeout(r, 3000));
    const body = document.body.innerText;
    window.__tauri__.emit('webview-quota-result', {
      provider: 'doubao',
      success: true,
      data: { page_text: body.substring(0, 5000) },
    });
  } catch (e) {
    window.__tauri__.emit('webview-quota-result', {
      provider: 'doubao',
      success: false,
      error: e.message,
    });
  }
})();
"#;
