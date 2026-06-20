// Shared types, constants, and pure helpers for the quota panel.
// Kept in a `.ts` file so siblings can import without pulling JSX.

export type FilterMode = "all" | "balance" | "rate_limit" | "usage" | "error" | "low";

export type TimeWindow = 24 | 168; // hours

/** Distinct colors for per-channel bar segments in UsageChart */
export const CHANNEL_COLORS = [
  "var(--color-accent)",
  "var(--color-success)",
  "var(--color-warning)",
  "var(--color-danger)",
  "#8b5cf6",
  "#ec4899",
  "#14b8a6",
  "#f97316",
];

export function formatBalance(value: number | null, fallback = "—"): string {
  if (value == null) return fallback;
  if (value >= 1000) return `$${(value / 1000).toFixed(1)}k`;
  return `$${value.toFixed(2)}`;
}

export function sourceLabel(source: string): string {
  switch (source) {
    case "http_api": return "API";
    case "openai_compat": return "NewAPI";
    case "response_header": return "Rate-Limit";
    case "webview": return "WebView";
    case "jsonpath": return "Custom";
    default: return source;
  }
}

export function sourceColor(source: string): string {
  switch (source) {
    case "http_api": return "var(--color-success)";
    case "openai_compat": return "var(--color-accent)";
    case "response_header": return "var(--color-warning)";
    case "webview": return "var(--color-warning)";
    case "jsonpath": return "var(--color-accent)";
    default: return "var(--color-text-muted)";
  }
}

export function formatTokens(value: number | null, fallback = "—"): string {
  if (value == null) return fallback;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return value.toLocaleString();
}

export function balanceColor(balance: number, limit: number | null): string {
  if (limit == null || limit <= 0) return "var(--color-text)";
  const pct = balance / limit;
  if (pct < 0.1) return "var(--color-danger)";
  if (pct < 0.3) return "var(--color-warning)";
  return "var(--color-success)";
}

export function strategyHint(source: string): string | null {
  switch (source) {
    case "response_header":
      return "Rate-limit data will appear automatically when this channel proxies requests.";
    case "webview":
      return "Click \"WebView Scrape\" to fetch balance data from the provider console.";
    default:
      return null;
  }
}
