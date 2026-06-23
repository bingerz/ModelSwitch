// Shared types, constants, and pure helpers for the quota panel.
// Kept in a `.ts` file so siblings can import without pulling JSX.

import { formatTokens as formatTokensValue } from "../../lib/format";

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

export function sourceLabelKey(source: string): string {
  switch (source) {
    case "http_api": return "quota.sourceApi";
    case "openai_compat": return "quota.sourceNewApi";
    case "response_header": return "quota.sourceRateLimit";
    case "webview": return "quota.sourceWebview";
    case "jsonpath": return "quota.sourceCustom";
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
  return formatTokensValue(value);
}

export function balanceColor(balance: number, limit: number | null): string {
  if (limit == null || limit <= 0) return "var(--color-text)";
  const pct = balance / limit;
  if (pct < 0.1) return "var(--color-danger)";
  if (pct < 0.3) return "var(--color-warning)";
  return "var(--color-success)";
}

export function strategyHintKey(source: string): string | null {
  switch (source) {
    case "response_header": return "quota.hintResponseHeader";
    case "webview": return "quota.hintWebview";
    default: return null;
  }
}
