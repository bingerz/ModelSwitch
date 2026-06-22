// Unified formatting helpers — single source of truth for the entire frontend.
// Do not duplicate these functions in component files; import from here instead.

/** Format large numbers: 1.2k, 3.4M */
export function formatNumber(n: number): string {
  const abs = Math.abs(n);
  if (abs < 1000) return Math.round(n).toString();
  if (abs < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** Format USD cost: $1.23, $1.2k, $1.2M */
export function formatCost(n: number): string {
  const abs = Math.abs(n);
  if (abs < 1000) return `$${n.toFixed(2)}`;
  if (abs < 1_000_000) return `$${(n / 1000).toFixed(1)}k`;
  return `$${(n / 1_000_000).toFixed(1)}M`;
}

/** Format cents to dollar display: 1234 cents -> $12.34 */
export function formatCents(cents: number): string {
  return `$${(cents / 100).toFixed(2)}`;
}

/** Format token counts: 1.2M / 3.4k */
export function formatTokens(n: number): string {
  return formatNumber(n);
}

/** Format relative time from Date or ISO string */
export function formatRelativeTime(input: Date | string): string {
  const from = input instanceof Date ? input : new Date(input);
  const diff = Math.max(0, Date.now() - from.getTime());
  if (diff < 60_000) return "Just now";
  const min = Math.floor(diff / 60_000);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  return `${day}d ago`;
}

/**
 * Latency color based on unified thresholds:
 *   < 500ms green, < 2000ms warning, < 5000ms orange, else red
 */
export function latencyColor(ms: number): string {
  if (ms < 500) return "var(--color-success)";
  if (ms < 2000) return "var(--color-warning)";
  if (ms < 5000) return "#f97316";
  return "var(--color-danger)";
}
