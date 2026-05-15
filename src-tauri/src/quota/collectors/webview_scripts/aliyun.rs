/// Aliyun Bailian quota scraping script.
/// Fetches billing data from the Bailian console using session cookies.

pub const ALIYUN_BAILIAN_SCRIPT: &str = r#"
(async () => {
  try {
    // Try the DashScope usage summary API first
    const resp = await fetch('https://dashscope.console.aliyun.com/api/v1/usage/summary', {
      credentials: 'include',
      headers: { 'Accept': 'application/json' },
    });
    if (resp.ok) {
      const data = await resp.json();
      window.__tauri__.emit('webview-quota-result', {
        provider: 'aliyun',
        success: true,
        data: data,
      });
      return;
    }

    // Fallback: try the Bailian console API
    const resp2 = await fetch('/api/bss/billing/usage/summary', {
      credentials: 'include',
      headers: { 'Accept': 'application/json' },
    });
    if (resp2.ok) {
      const data = await resp2.json();
      window.__tauri__.emit('webview-quota-result', {
        provider: 'aliyun',
        success: true,
        data: data,
      });
      return;
    }

    // Last resort: scrape page text for balance info
    await new Promise(r => setTimeout(r, 3000));
    const body = document.body.innerText;
    window.__tauri__.emit('webview-quota-result', {
      provider: 'aliyun',
      success: true,
      data: { page_text: body.substring(0, 5000) },
    });
  } catch (e) {
    window.__tauri__.emit('webview-quota-result', {
      provider: 'aliyun',
      success: false,
      error: e.message,
    });
  }
})();
"#;
