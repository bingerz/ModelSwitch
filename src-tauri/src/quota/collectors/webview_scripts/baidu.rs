/// Baidu Qianfan coding plan scraping script.
/// Fetches console.bce.baidu.com/api/qianfan/charge/codingPlan/resourceList.

pub const BAIDU_CODING_PLAN_SCRIPT: &str = r#"
(async () => {
  try {
    const resp = await fetch('/api/qianfan/charge/codingPlan/resourceList', {
      credentials: 'include',
    });
    if (!resp.ok) {
      window.__tauri__.emit('webview-quota-result', {
        provider: 'baidu',
        success: false,
        error: `HTTP ${resp.status}`,
      });
      return;
    }
    const data = await resp.json();
    window.__tauri__.emit('webview-quota-result', {
      provider: 'baidu',
      success: true,
      data: data,
    });
  } catch (e) {
    window.__tauri__.emit('webview-quota-result', {
      provider: 'baidu',
      success: false,
      error: e.message,
    });
  }
})();
"#;
