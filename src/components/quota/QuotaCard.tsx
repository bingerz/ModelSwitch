import { useState } from "react";
import { useTranslation } from "react-i18next";
import { type QuotaInfo, invokeTauri } from "../../lib/api";
import { isTauri } from "../../lib/runtime";
import {
  balanceColor,
  formatBalance,
  formatTokens,
  sourceColor,
  sourceLabelKey,
  strategyHintKey,
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

// ─── Provider Data Section (balance, items, groups, rate limits) ───────

function ProviderDataSection({ q }: { q: QuotaInfo }) {
  const { t } = useTranslation();
  const hasBalance = q.balance != null;
  const hasUsage = q.usage != null && q.usage > 0;
  const hasGroups = q.groups.length > 0;
  const hasRateLimit =
    q.rate_limit_remaining_req != null || q.rate_limit_remaining_tok != null;
  const usagePct =
    hasBalance && q.limit != null && q.limit > 0
      ? Math.min(100, ((q.limit - q.balance!) / q.limit) * 100)
      : null;

  return (
    <>
      <SectionDivider label={t("quota.providerData")} />

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
              <span className="quota-limit mono"> / {formatBalance(q.limit)}</span>
            )}
          </div>
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
              {usagePct != null
                ? t("quota.usedLabel", { amount: formatBalance(q.usage), pct: usagePct.toFixed(0) })
                : t("quota.used", { amount: formatBalance(q.usage) })}
            </div>
          )}
        </div>
      )}

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
                {g.utilization_pct != null ? `${g.utilization_pct.toFixed(0)}%` : "—"}
              </span>
              {g.resets_at && (
                <span className="quota-group-resets">
                  {t("quota.resetsAt", { time: new Date(g.resets_at).toLocaleTimeString() })}
                </span>
              )}
            </div>
          ))}
        </div>
      )}

      {hasRateLimit && (
        <div className="quota-ratelimit">
          {q.rate_limit_remaining_req != null && q.rate_limit_limit_req != null && (
            <span className="quota-rl-item">
              <span className="quota-rl-label">{t("quota.rpm")}</span>
              <span className="mono">
                {q.rate_limit_remaining_req}/{q.rate_limit_limit_req}
              </span>
            </span>
          )}
          {q.rate_limit_remaining_tok != null && q.rate_limit_limit_tok != null && (
            <span className="quota-rl-item">
              <span className="quota-rl-label">{t("quota.tpm")}</span>
              <span className="mono">
                {(q.rate_limit_remaining_tok / 1000).toFixed(0)}k/
                {(q.rate_limit_limit_tok / 1000).toFixed(0)}k
              </span>
            </span>
          )}
          {q.rate_limit_updated_at && (
            <span className="quota-rl-time">
              {t("quota.updatedAt", { time: new Date(q.rate_limit_updated_at).toLocaleTimeString() })}
            </span>
          )}
        </div>
      )}
    </>
  );
}

// ─── Proxy Statistics Section (software-calculated token usage) ────────

function ProxyStatisticsSection({ q }: { q: QuotaInfo }) {
  const { t } = useTranslation();
  return (
    <>
      <SectionDivider label={t("quota.proxyStatistics")} />
      <div className="quota-proxy-stats">
        <div className="quota-tokens-row">
          <span className="quota-tokens-label">{t("quota.totalTokens")}</span>
          <span className="quota-tokens-value">
            {formatTokens((q.total_input_tokens ?? 0) + (q.total_output_tokens ?? 0))}
          </span>
        </div>
        {q.total_requests_counted != null && (
          <div className="quota-tokens-row">
            <span className="quota-tokens-label">{t("quota.requests")}</span>
            <span className="quota-tokens-value">
              {q.total_requests_counted.toLocaleString()}
            </span>
          </div>
        )}
        {q.total_estimated_cost != null && q.total_estimated_cost > 0 && (
          <div className="quota-tokens-row">
            <span className="quota-tokens-label">{t("quota.estCost")}</span>
            <span className="quota-tokens-value mono">
              {q.provider === "deepseek" ? "¥" : "$"}{q.total_estimated_cost.toFixed(2)}
            </span>
          </div>
        )}
        {(q.total_cache_hit_tokens != null || q.total_cache_miss_tokens != null) && (
          <div className="quota-tokens-row quota-tokens-row-sub">
            <span className="quota-tokens-label">{t("quota.cacheHit")}</span>
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
  );
}

// ─── Main Card ─────────────────────────────────────────────────────────

export function QuotaCard({ q, onScrape }: { q: QuotaInfo; onScrape?: (info: QuotaInfo) => void }) {
  const { t } = useTranslation();
  const [scraping, setScraping] = useState(false);
  const canScrape = WEBVIEW_SCRAPE_PROVIDERS.has(q.provider);
  const hasError = q.error !== null;
  const hasBalance = q.balance != null;
  const hasRateLimit =
    q.rate_limit_remaining_req != null || q.rate_limit_remaining_tok != null;
  const hasUsage = q.usage != null && q.usage > 0;
  const hasTokenUsage = q.total_input_tokens != null || q.total_output_tokens != null;
  const hasGroups = q.groups.length > 0;
  const hintKey = strategyHintKey(q.source);

  const showProviderData =
    hasBalance || hasUsage || hasGroups || hasRateLimit || q.items.length > 0;

  return (
    <div className={`quota-card${hasError ? " quota-card-error" : ""}`}>
      <div className="quota-card-header">
        <div className="quota-card-title">
          <strong>{q.channel_name}</strong>
          <span className="quota-source-badge" style={{ color: sourceColor(q.source) }}>
            {t(sourceLabelKey(q.source))}
          </span>
        </div>
        <span className="quota-provider">{q.provider}</span>
        {isTauri && canScrape && (
          <button
            className="btn btn-sm"
            disabled={scraping}
            onClick={async () => {
              setScraping(true);
              const result = await scrapeWebView(q.channel_id);
              if (result && onScrape) onScrape(result);
              setScraping(false);
            }}
            title={t("quota.scrapeTooltip")}
          >
            {scraping ? t("quota.scraping") : t("quota.scrapeWebview")}
          </button>
        )}
      </div>

      {hasError && <div className="quota-error">{q.error}</div>}

      {!hasError && !hasBalance && !hasTokenUsage && hintKey && (
        <div className="quota-hint">{t(hintKey)}</div>
      )}

      {hasTokenUsage && <ProxyStatisticsSection q={q} />}

      {showProviderData && <ProviderDataSection q={q} />}

      {!hasBalance && !hasRateLimit && !hasUsage && !hasGroups && !hasTokenUsage && !hasError && (
        <div className="quota-no-data">{t("quota.noDataHint")}</div>
      )}

      {q.expires_at && (
        <div className="quota-expires">
          {t("quota.expires", { date: new Date(q.expires_at).toLocaleDateString() })}
        </div>
      )}

      <div className="quota-updated">
        {t("quota.updatedLabel", { time: new Date(q.updated_at).toLocaleTimeString() })}
      </div>
    </div>
  );
}
