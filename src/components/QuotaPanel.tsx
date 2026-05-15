import { useMemo, useState } from "react";
import type { QuotaInfo, UsageBucket } from "../lib/api";
import { useQuota } from "../hooks/useQuota";

type FilterMode = "all" | "balance" | "rate_limit" | "usage" | "error" | "low";

function formatBalance(value: number | null, fallback = "—"): string {
  if (value == null) return fallback;
  if (value >= 1000) return `$${(value / 1000).toFixed(1)}k`;
  return `$${value.toFixed(2)}`;
}

function sourceLabel(source: string): string {
  switch (source) {
    case "http_api": return "API";
    case "openai_compat": return "NewAPI";
    case "response_header": return "Rate-Limit";
    case "webview": return "WebView";
    case "jsonpath": return "Custom";
    default: return source;
  }
}

function sourceColor(source: string): string {
  switch (source) {
    case "http_api": return "var(--color-success)";
    case "openai_compat": return "var(--color-accent)";
    case "response_header": return "var(--color-warning)";
    case "webview": return "var(--color-warning)";
    case "jsonpath": return "var(--color-accent)";
    default: return "var(--color-text-muted)";
  }
}

function formatTokens(value: number | null, fallback = "—"): string {
  if (value == null) return fallback;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return value.toLocaleString();
}

function balanceColor(balance: number, limit: number | null): string {
  if (limit == null || limit <= 0) return "var(--color-text)";
  const pct = balance / limit;
  if (pct < 0.1) return "var(--color-danger)";
  if (pct < 0.3) return "var(--color-warning)";
  return "var(--color-success)";
}

function strategyHint(source: string): string | null {
  switch (source) {
    case "response_header":
      return "Rate-limit data will appear automatically when this channel proxies requests.";
    case "webview":
      return "Click \"WebView Scrape\" to fetch balance data from the provider console.";
    default:
      return null;
  }
}

/** Providers that support WebView quota scraping */
const WEBVIEW_SCRAPE_PROVIDERS = new Set(["anthropic", "baidu", "aliyun", "doubao"]);

async function scrapeWebView(channelId: string): Promise<QuotaInfo | null> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const result = await invoke<QuotaInfo>("scrape_webview_quota", {
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

function QuotaCard({ q, onScrape }: { q: QuotaInfo; onScrape?: (info: QuotaInfo) => void }) {
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

/** Placeholder card for channels that don't have any quota entry from the backend */
function PendingCard({ name, provider }: { name: string; provider: string }) {
  return (
    <div className="quota-card quota-card-pending">
      <div className="quota-card-header">
        <div className="quota-card-title">
          <strong>{name}</strong>
          <span className="quota-source-badge" style={{ color: "var(--color-text-muted)" }}>
            Pending
          </span>
        </div>
        <span className="quota-provider">{provider}</span>
      </div>
      <div className="quota-no-data">Waiting for quota data...</div>
    </div>
  );
}

/** Distinct colors for per-channel bar segments */
const CHANNEL_COLORS = [
  "var(--color-accent)",
  "var(--color-success)",
  "var(--color-warning)",
  "var(--color-danger)",
  "#8b5cf6",
  "#ec4899",
  "#14b8a6",
  "#f97316",
];

type TimeWindow = 24 | 168; // hours

function UsageChart({ buckets, window, onWindowChange }: { buckets: UsageBucket[]; window: TimeWindow; onWindowChange: (w: TimeWindow) => void }) {
  // Aggregate cache stats across all buckets
  const cacheStats = useMemo(() => {
    let hits = 0;
    let misses = 0;
    for (const b of buckets) {
      hits += b.cache_hit_tokens;
      misses += b.cache_miss_tokens;
    }
    const total = hits + misses;
    return { hits, misses, total, ratio: total > 0 ? hits / total : null };
  }, [buckets]);

  const windowLabels: { value: TimeWindow; label: string }[] = [
    { value: 24, label: "24h" },
    { value: 168, label: "7d" },
  ];

  // Build hourly data with all 24 hours filled in
  const hourlyData = useMemo(() => {
    const isDaily = window >= 168;
    if (isDaily) {
      // Aggregate into daily buckets
      const dayMap = new Map<string, {
        label: string;
        segments: Map<string, { name: string; model: string; tokens: number; cost: number; requests: number; cacheHits: number; cacheMisses: number }>;
      }>();

      for (const b of buckets) {
        const dayKey = b.timestamp.slice(0, 10);
        if (!dayMap.has(dayKey)) {
          const d = new Date(b.timestamp);
          dayMap.set(dayKey, { label: d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" }), segments: new Map() });
        }
        const day = dayMap.get(dayKey)!;
        const segKey = `${b.channel_id}:${b.model}`;
        const tokens = b.input_tokens + b.output_tokens;
        const existing = day.segments.get(segKey);
        if (existing) {
          existing.tokens += tokens;
          existing.cost += b.estimated_cost;
          existing.requests += b.request_count;
          existing.cacheHits += b.cache_hit_tokens;
          existing.cacheMisses += b.cache_miss_tokens;
        } else {
          day.segments.set(segKey, {
            name: b.channel_name,
            model: b.model,
            tokens,
            cost: b.estimated_cost,
            requests: b.request_count,
            cacheHits: b.cache_hit_tokens,
            cacheMisses: b.cache_miss_tokens,
          });
        }
      }

      return Array.from(dayMap.entries())
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([, v]) => v);
    }

    // Hourly mode — fill all 24 hours
    const now = new Date();
    const currentHour = now.getHours();
    const hourMap = new Map<string, {
      label: string;
      segments: Map<string, { name: string; model: string; tokens: number; cost: number; requests: number; cacheHits: number; cacheMisses: number }>;
    }>();

    // Pre-fill all 24 hours
    for (let i = 0; i < 24; i++) {
      const h = (currentHour - 23 + i + 24) % 24;
      const label = `${h}`;
      hourMap.set(label, { label, segments: new Map() });
    }

    for (const b of buckets) {
      const h = new Date(b.timestamp).getHours();
      const label = `${h}`;
      const hour = hourMap.get(label)!;
      const segKey = `${b.channel_id}:${b.model}`;
      const tokens = b.input_tokens + b.output_tokens;
      const existing = hour.segments.get(segKey);
      if (existing) {
        existing.tokens += tokens;
        existing.cost += b.estimated_cost;
        existing.requests += b.request_count;
        existing.cacheHits += b.cache_hit_tokens;
        existing.cacheMisses += b.cache_miss_tokens;
      } else {
        hour.segments.set(segKey, {
          name: b.channel_name,
          model: b.model,
          tokens,
          cost: b.estimated_cost,
          requests: b.request_count,
          cacheHits: b.cache_hit_tokens,
          cacheMisses: b.cache_miss_tokens,
        });
      }
    }

    return Array.from(hourMap.values());
  }, [buckets, window]);

  // Assign stable colors per channel
  const channelColorMap = useMemo(() => {
    const ids = [...new Set(buckets.map((b) => b.channel_id))];
    const map = new Map<string, string>();
    ids.forEach((id, i) => map.set(id, CHANNEL_COLORS[i % CHANNEL_COLORS.length]));
    return map;
  }, [buckets]);

  const maxTokens = useMemo(
    () =>
      Math.max(
        1,
        ...hourlyData.map((h) =>
          [...h.segments.values()].reduce((s, c) => s + c.tokens, 0)
        ),
      ),
    [hourlyData],
  );

  const hasAnyData = useMemo(
    () => hourlyData.some((h) => [...h.segments.values()].reduce((s, c) => s + c.tokens, 0) > 0),
    [hourlyData],
  );

  if (!hasAnyData) {
    return (
      <div>
        <div className="usage-chart-tabs">
          {windowLabels.map((w) => (
            <button
              key={w.value}
              className={`usage-chart-tab${window === w.value ? " active" : ""}`}
              onClick={() => onWindowChange(w.value)}
            >
              {w.label}
            </button>
          ))}
        </div>
        <div className="usage-chart-empty">
          No usage data in this period. Data will appear once channels process requests.
        </div>
      </div>
    );
  }

  return (
    <div>
      <div className="usage-chart-tabs">
        {windowLabels.map((w) => (
          <button
            key={w.value}
            className={`usage-chart-tab${window === w.value ? " active" : ""}`}
            onClick={() => onWindowChange(w.value)}
          >
            {w.label}
          </button>
        ))}
      </div>
      <div className="usage-chart">
        <div className="usage-chart-bars">
          {hourlyData.map((hour) => {
            const segments = [...hour.segments.entries()];
            const totalTokens = segments.reduce((s, [, c]) => s + c.tokens, 0);
            const totalCost = segments.reduce((s, [, c]) => s + c.cost, 0);
            const totalRequests = segments.reduce((s, [, c]) => s + c.requests, 0);
            const totalCacheHits = segments.reduce((s, [, c]) => s + c.cacheHits, 0);
            const heightPct = totalTokens > 0 ? (totalTokens / maxTokens) * 100 : 0;

            const tooltipLines: string[] = [`${hour.label}:00`];
            if (totalTokens > 0) {
              tooltipLines.push(`Tokens: ${formatTokens(totalTokens)}`);
              tooltipLines.push(`Requests: ${totalRequests}`);
              if (totalCost > 0) tooltipLines.push(`Cost: $${totalCost.toFixed(4)}`);
              if (totalCacheHits > 0) {
                const cacheTotal = totalCacheHits + segments.reduce((s, [, c]) => s + c.cacheMisses, 0);
                tooltipLines.push(`Cache hit: ${((totalCacheHits / cacheTotal) * 100).toFixed(0)}%`);
              }
              if (segments.length > 1) {
                tooltipLines.push("---");
                for (const [, seg] of segments) {
                  tooltipLines.push(`${seg.name} | ${seg.model}: ${formatTokens(seg.tokens)} tok, ${seg.requests} req`);
                }
              }
            } else {
              tooltipLines.push("No data");
            }

            return (
              <div key={hour.label} className="usage-chart-col" title={tooltipLines.join("\n")}>
                {totalTokens > 0 && (
                  <span className="usage-chart-bar-value">{formatTokens(totalTokens)}</span>
                )}
                <div className="usage-chart-bar-stack" style={{ height: `${heightPct}%` }}>
                  {segments.map(([segKey, seg]) => {
                    if (seg.tokens === 0) return null;
                    const channelId = segKey.split(":")[0];
                    const segPct = totalTokens > 0 ? (seg.tokens / totalTokens) * 100 : 0;
                    const segCacheTotal = seg.cacheHits + seg.cacheMisses;
                    const segTooltip = `${seg.name} (${seg.model}): ${formatTokens(seg.tokens)} tokens (${seg.requests} reqs)${segCacheTotal > 0 ? ` | Cache: ${((seg.cacheHits / segCacheTotal) * 100).toFixed(0)}% hit` : ""}`;
                    return (
                      <div
                        key={segKey}
                        className="usage-chart-bar-segment"
                        style={{
                          height: `${segPct}%`,
                          background: channelColorMap.get(channelId) ?? "var(--color-accent)",
                        }}
                        title={segTooltip}
                      />
                    );
                  })}
                </div>
                <span className="usage-chart-col-label">{hour.label}</span>
              </div>
            );
          })}
        </div>
        {/* Legend + Cache stats */}
        <div className="usage-chart-footer">
          {channelColorMap.size > 1 && (
            <div className="usage-chart-legend">
              {[...channelColorMap.entries()].map(([id, color]) => {
                const name = buckets.find((b) => b.channel_id === id)?.channel_name ?? id;
                return (
                  <span key={id} className="usage-chart-legend-item">
                    <span className="usage-chart-legend-dot" style={{ background: color }} />
                    {name}
                  </span>
                );
              })}
            </div>
          )}
          {cacheStats.total > 0 && (
            <div className="usage-chart-cache-stats">
              Cache: <span className="mono">{formatTokens(cacheStats.hits)}</span> hit / <span className="mono">{formatTokens(cacheStats.misses)}</span> miss
              {cacheStats.ratio != null && (
                <span className="usage-chart-cache-pct"> ({(cacheStats.ratio * 100).toFixed(0)}% hit)</span>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export function QuotaPanel() {
  const {
    quotas,
    channels,
    usageHistory,
    refresh,
    fetchUsageHistory,
    totalBalance,
    channelsWithData,
    lowBalanceCount,
    errorCount,
    totalChannels,
  } = useQuota();
  const [filter, setFilter] = useState<FilterMode>("all");
  const [refreshing, setRefreshing] = useState(false);
  const [usageWindow, setUsageWindow] = useState<TimeWindow>(24);

  const handleRefresh = async () => {
    setRefreshing(true);
    await refresh();
    setRefreshing(false);
  };

  // Filter quotas — no longer hiding errored entries
  const filtered = quotas.filter((q) => {
    switch (filter) {
      case "balance":
        return q.balance != null && q.error === null;
      case "rate_limit":
        return q.rate_limit_remaining_req != null && q.error === null;
      case "usage":
        return (q.total_input_tokens != null || q.total_output_tokens != null) && q.error === null;
      case "error":
        return q.error !== null;
      case "low":
        return q.balance != null && q.limit != null && q.limit > 0
          ? q.balance / q.limit < 0.2
          : false;
      default:
        return true;
    }
  });

  // Find channels that have no quota entry at all
  const quotaChannelIds = new Set(quotas.map((q) => q.channel_id));
  const missingChannels = channels.filter(
    (c) => !quotaChannelIds.has(c.id),
  );

  const filterOptions: { id: FilterMode; label: string }[] = [
    { id: "all", label: `All (${quotas.length})` },
    {
      id: "balance",
      label: `Balance (${quotas.filter((q) => q.balance != null && q.error === null).length})`,
    },
    {
      id: "rate_limit",
      label: `Rate-Limit (${quotas.filter((q) => q.rate_limit_remaining_req != null && q.error === null).length})`,
    },
    {
      id: "usage",
      label: `Usage (${quotas.filter((q) => (q.total_input_tokens != null || q.total_output_tokens != null) && q.error === null).length})`,
    },
    ...(errorCount > 0
      ? [{ id: "error" as FilterMode, label: `Errors (${errorCount})` }]
      : []),
    ...(lowBalanceCount > 0
      ? [{ id: "low" as FilterMode, label: `Low (${lowBalanceCount})` }]
      : []),
  ];

  const hasAnyContent = filtered.length > 0 || (filter === "all" && missingChannels.length > 0);

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">Token Quota</h2>
        <button
          className="btn"
          onClick={handleRefresh}
          disabled={refreshing}
        >
          {refreshing ? "Refreshing..." : "Refresh"}
        </button>
      </div>

      {/* Summary cards */}
      <div className="quota-summary-grid">
        <div className="stat-card">
          <div className="stat-value">
            {formatBalance(totalBalance, "$0")}
          </div>
          <div className="stat-label">Total Balance</div>
        </div>
        <div className="stat-card stat-success">
          <div className="stat-value">{channelsWithData}/{totalChannels}</div>
          <div className="stat-label">Channels with Data</div>
        </div>
        {errorCount > 0 && (
          <div className="stat-card stat-danger">
            <div className="stat-value">{errorCount}</div>
            <div className="stat-label">Errors</div>
          </div>
        )}
        {lowBalanceCount > 0 && (
          <div className="stat-card stat-danger">
            <div className="stat-value">{lowBalanceCount}</div>
            <div className="stat-label">Low Balance (&lt;20%)</div>
          </div>
        )}
      </div>

      {/* Usage history chart */}
      {usageHistory && (
        <div className="usage-chart-section">
          <h3 className="usage-chart-title">Usage</h3>
          <div className="usage-chart-summary">
            <span className="mono">{formatTokens(usageHistory.total_input_tokens + usageHistory.total_output_tokens)}</span> tokens
            {" across "}
            <span className="mono">{usageHistory.total_requests}</span> requests
            {usageHistory.total_cost > 0 && (
              <>{" "}&middot; <span className="mono">${usageHistory.total_cost.toFixed(4)}</span> est. cost</>
            )}
          </div>
          <UsageChart buckets={usageHistory.buckets} window={usageWindow} onWindowChange={(w) => { setUsageWindow(w); fetchUsageHistory(w); }} />
        </div>
      )}

      {/* Filter bar */}
      <div className="quota-filter-bar">
        {filterOptions.map((opt) => (
          <button
            key={opt.id}
            className={`quota-filter-btn ${filter === opt.id ? "active" : ""}`}
            onClick={() => setFilter(opt.id)}
          >
            {opt.label}
          </button>
        ))}
      </div>

      {/* Quota cards grid */}
      {hasAnyContent ? (
        <div className="quota-grid">
          {filtered.map((q) => (
            <QuotaCard key={q.channel_id} q={q} onScrape={() => refresh()} />
          ))}
          {filter === "all" && missingChannels.map((c) => (
            <PendingCard key={c.id} name={c.name} provider={c.provider} />
          ))}
        </div>
      ) : (
        <div className="quota-empty">
          <div className="quota-empty-icon">📊</div>
          <p>
            {totalChannels === 0
              ? "No channels configured yet. Add channels in the Channels tab to start monitoring quota."
              : filter === "error"
                ? "No channels with errors."
                : filter === "usage"
                  ? "No token usage data yet. Usage will appear once channels start processing requests."
                  : "No quota data matching this filter. Quota information will appear here once channels start reporting balance, rate-limit, or usage data."}
          </p>
        </div>
      )}
    </section>
  );
}
