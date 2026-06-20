import { useMemo } from "react";
import { type UsageBucket } from "../../lib/api";
import { CHANNEL_COLORS, formatTokens, type TimeWindow } from "./types";

export function UsageChart({ buckets, window, onWindowChange }: { buckets: UsageBucket[]; window: TimeWindow; onWindowChange: (w: TimeWindow) => void }) {
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
