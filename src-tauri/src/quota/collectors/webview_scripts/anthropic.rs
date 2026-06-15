/// Anthropic OAuth usage scraping script.
/// Fetches console.anthropic.com/api/oauth/usage with session cookies.
/// JavaScript to inject into the Anthropic console WebView.
/// Navigates to the OAuth usage API and extracts 5h/7d utilization data.
pub const ANTHROPIC_USAGE_SCRIPT: &str = r#"
(async () => {
  try {
    const resp = await fetch('https://console.anthropic.com/api/oauth/usage', {
      credentials: 'include',
    });
    if (!resp.ok) {
      window.__tauri__.emit('webview-quota-result', {
        provider: 'anthropic',
        success: false,
        error: `HTTP ${resp.status}`,
      });
      return;
    }
    const data = await resp.json();
    window.__tauri__.emit('webview-quota-result', {
      provider: 'anthropic',
      success: true,
      data: data,
    });
  } catch (e) {
    window.__tauri__.emit('webview-quota-result', {
      provider: 'anthropic',
      success: false,
      error: e.message,
    });
  }
})();
"#;

/// Parsed Anthropic usage data.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AnthropicUsage {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day_sonnet: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day_opus: Option<UsageWindow>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UsageWindow {
    pub utilization: f64,
    pub resets_at: Option<String>,
}

/// Convert AnthropicUsage into QuotaInfo-compatible groups.
impl AnthropicUsage {
    pub fn to_groups(&self) -> Vec<crate::quota::QuotaGroup> {
        let mut groups = Vec::new();
        if let Some(ref w) = self.five_hour {
            groups.push(crate::quota::QuotaGroup {
                window: "5h".into(),
                utilization_pct: Some(w.utilization),
                resets_at: w.resets_at.clone(),
            });
        }
        if let Some(ref w) = self.seven_day {
            groups.push(crate::quota::QuotaGroup {
                window: "7d".into(),
                utilization_pct: Some(w.utilization),
                resets_at: w.resets_at.clone(),
            });
        }
        if let Some(ref w) = self.seven_day_sonnet {
            groups.push(crate::quota::QuotaGroup {
                window: "7d_sonnet".into(),
                utilization_pct: Some(w.utilization),
                resets_at: w.resets_at.clone(),
            });
        }
        if let Some(ref w) = self.seven_day_opus {
            groups.push(crate::quota::QuotaGroup {
                window: "7d_opus".into(),
                utilization_pct: Some(w.utilization),
                resets_at: w.resets_at.clone(),
            });
        }
        groups
    }
}
