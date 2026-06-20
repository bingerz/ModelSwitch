import { useState } from "react";
import { type QuotaInfo, invokeTauri } from "../../lib/api";
import {
  balanceColor,
  formatBalance,
  formatTokens,
  sourceColor,
  sourceLabel,
  strategyHint,
} from "./types";

/** Providers that support WebView quota scraping */
const WEBVIEW_SCRAPE_PROVIDERS = new Set(["anthropic", "baidu", "aliyun", "doubao"]);

async function scrapeWebView(channelId: string): Promise<QuotaInfo | null> {
  try {
    const result = await invokeTauri<QuotaInfo>("scrape_webview_quota", {
      channelId,
    });
    return result;
  } catch {
    // Not running in Tauri or scrape failed
    return null;
  }
}

function SectionDivider({ label }: { label: string }) {
  return (
    <div className="quota-section-divider">
      <span className="quota-section-label">{label}</span>
    </div>
  );
}

export function QuotaCard({ q, onScrape }: { q: QuotaInfo; onScrape?: (info: QuotaInfo) => void }) {
  const [scraping, setScraping] = useState(false);
  const canScrape = WEBVIEW_SCRAPE_PROVIDERS.has(q.provider);
  const hasError = q.error !== null;
  const hasBalance = q.balance != null;
  const hasRateLimit =
    q.rate_limit_remaining_req != null || q.rate_limit_remaining_tok != null;
  const hasUsage = q.usage != null && q.usage > 0;
  const hasTokenUsage = q.total_input_tokens != null || q.total_output_tokens != null;
  const hasGroups = q.groups.length > 0;
  const hint = strategyHint(q.source);

  const usagePct =
    hasBalance && q.limit != null && q.limit > 0
      ? Math.min(100, ((q.limit - q.balance!) / q.limit) * 100)
      : null;

  return (
    <div className={`quota-card${hasError ? " quota-card-error" : ""}`}>
      {/* Header */}
      <div className="quota-card-header">
        <div className="quota-card-title">
          <strong>{q.channel_name}</strong>
          <span
            className="quota-source-badge"
            style={{ color: sourceColor(q.source) }}
          >
            {sourceLabel(q.source)}
          </span>
        </div>
        <span className="quota-provider">{q.provider}</span>
        {canScrape && (
          <button
            className="btn btn-sm"
            disabled={scraping}
            onClick={async () => {
              setScraping(true);
              const result = await scrapeWebView(q.channel_id);
              if (result && onScrape) onScrape(result);
              setScraping(false);
            }}
            title="Scrape balance via WebView"
          >
            {scraping ? "Scraping..." : "WebView Scrape"}
          </button>
        )}
      </div>

      {/* Error state — show prominently */}
      {hasError && (
        <div className="quota-error">{q.error}</div>
      )}

      {/* Strategy hint when no data — guide user on what to do */}
      {!hasError && !hasBalance && !hasTokenUsage && hint && (
        <div className="quota-hint">{hint}</div>
      )}

      {/* ── Proxy Statistics (software-calculated) ── */}
      {hasTokenUsage && (
        <>
          <SectionDivider label="Proxy Statistics" />
          <div className="quota-proxy-stats">
            <div className="quota-tokens-row">
              <span className="quota-tokens-label">Total Tokens</span>
              <span className="quota-tokens-value">
                {formatTokens((q.total_input_tokens ?? 0) + (q.total_output_tokens ?? 0))}
              </span>
            </div>
            {q.total_requests_counted != null && (
              <div className="quota-tokens-row">
                <span className="quota-tokens-label">Requests</span>
                <span className="quota-tokens-value">
                  {q.total_requests_counted.toLocaleString()}
                </span>
              </div>
            )}
            {q.total_estimated_cost != null && q.total_estimated_cost > 0 && (
              <div className="quota-tokens-row">
                <span className="quota-tokens-label">Est. Cost</span>
                <span className="quota-tokens-value mono">
                  {q.provider === "deepseek" ? "¥" : "$"}{q.total_estimated_cost.toFixed(2)}
                </span>
              </div>
            )}
            {(q.total_cache_hit_tokens != null || q.total_cache_miss_tokens != null) && (
              <div className="quota-tokens-row quota-tokens-row-sub">
                <span className="quota-tokens-label">Cache Hit</span>
                <span className="quota-tokens-value">
                  {formatTokens(q.total_cache_hit_tokens)}
                  {q.total_cache_hit_tokens != null && (q.total_cache_hit_tokens + (q.total_cache_miss_tokens ?? 0)) > 0 && (
                    <span className="quota-cache-pct">
                      {" "}({((q.total_cache_hit_tokens / (q.total_cache_hit_tokens + (q.total_cache_miss_tokens ?? 0))) * 100).toFixed(0)}%)
                    </span>
                  )}
                </span>
              </div>
            )}
          </div>
        </>
      )}

      {/* ── Provider Data (from provider billing API) ── */}
      {(hasBalance || hasUsage || hasGroups || hasRateLimit || q.items.length > 0) && (
        <>
          <SectionDivider label="Provider Data" />

          {/* Balance section */}
          {hasBalance && (
            <div className="quota-balance-section">
              <div className="quota-balance-value">
                <span
                  className="mono"
                  style={{
                    fontSize: "1.5rem",
                    fontWeight: 700,
                    color: balanceColor(q.balance!, q.limit),
                  }}
                >
                  {formatBalance(q.balance)}
                </span>
                {q.limit != null && (
                  <span className="quota-limit mono">
                    {" "}
                    / {formatBalance(q.limit)}
                  </span>
                )}
              </div>

              {/* Progress bar */}
              {usagePct != null && (
                <div className="quota-progress-track">
                  <div
                    className="quota-progress-fill"
                    style={{
                      width: `${usagePct}%`,
                      background:
                        usagePct > 80
                          ? "var(--color-danger)"
                          : usagePct > 50
                            ? "var(--color-warning)"
                            : "var(--color-success)",
                    }}
                  />
                </div>
              )}

              {hasUsage && (
                <div className="quota-usage-text mono">
                  {formatBalance(q.usage)} used
                  {usagePct != null && ` (${usagePct.toFixed(0)}%)`}
                </div>
              )}
            </div>
          )}

          {/* Items (daily/weekly/monthly/voucher/cash) */}
          {q.items.length > 0 && (
            <div className="quota-items">
              {q.items.map((item) => (
                <span key={item.label} className="quota-item">
                  <span className="quota-item-label">{item.label}</span>
                  <span className="quota-item-value mono">{item.value}</span>
                </span>
              ))}
            </div>
          )}

          {/* Groups (5h/7d utilization from WebView) */}
          {hasGroups && (
            <div className="quota-groups">
              {q.groups.map((g) => (
                <div key={g.window} className="quota-group">
                  <span className="quota-group-window">{g.window}</span>
                  {g.utilization_pct != null && (
                    <div className="quota-group-bar-track">
                      <div
                        className="quota-group-bar-fill"
                        style={{
                          width: `${Math.min(100, g.utilization_pct)}%`,
                          background:
                            g.utilization_pct > 80
                              ? "var(--color-danger)"
                              : g.utilization_pct > 50
                                ? "var(--color-warning)"
                                : "var(--color-success)",
                        }}
                      />
                    </div>
                  )}
                  <span className="quota-group-value mono">
                    {g.utilization_pct != null
                      ? `${g.utilization_pct.toFixed(0)}%`
                      : "—"}
                  </span>
                  {g.resets_at && (
                    <span className="quota-group-resets">
                      resets {new Date(g.resets_at).toLocaleTimeString()}
                    </span>
                  )}
                </div>
              ))}
            </div>
          )}

          {/* Rate limit info */}
          {hasRateLimit && (
            <div className="quota-ratelimit">
              {q.rate_limit_remaining_req != null &&
                q.rate_limit_limit_req != null && (
                  <span className="quota-rl-item">
                    <span className="quota-rl-label">RPM</span>
                    <span className="mono">
                      {q.rate_limit_remaining_req}/{q.rate_limit_limit_req}
                    </span>
                  </span>
                )}
              {q.rate_limit_remaining_tok != null &&
                q.rate_limit_limit_tok != null && (
                  <span className="quota-rl-item">
                    <span className="quota-rl-label">TPM</span>
                    <span className="mono">
                      {(q.rate_limit_remaining_tok / 1000).toFixed(0)}k/
                      {(q.rate_limit_limit_tok / 1000).toFixed(0)}k
                    </span>
                  </span>
                )}
              {q.rate_limit_updated_at && (
                <span className="quota-rl-time">
                  updated{" "}
                  {new Date(q.rate_limit_updated_at).toLocaleTimeString()}
                </span>
              )}
            </div>
          )}
        </>
      )}

      {/* No balance, no rate limit → usage-only or empty */}
      {!hasBalance && !hasRateLimit && !hasUsage && !hasGroups && !hasTokenUsage && !hasError && (
        <div className="quota-no-data">No balance data available yet</div>
      )}

      {/* Expires */}
      {q.expires_at && (
        <div className="quota-expires">
          Expires {new Date(q.expires_at).toLocaleDateString()}
        </div>
      )}

      {/* Last updated */}
      <div className="quota-updated">
        Updated {new Date(q.updated_at).toLocaleTimeString()}
      </div>
    </div>
  );
}
