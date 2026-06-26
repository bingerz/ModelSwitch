/** Pure helper functions extracted from Portal.tsx for testing. */

/** Convert cents to USD display string. */
export function centsToUSD(cents: number): string {
  return `$${(cents / 100).toFixed(2)}`;
}

/** Format ISO timestamp to locale string. */
export function formatTime(iso: string): string {
  return new Date(iso).toLocaleString();
}

/** Compute progress bar state from spent/budget. */
export function computeProgressState(
  spent: number,
  budget: number | null,
): {
  pct: number;
  isOverBudget: boolean;
  isWarning: boolean;
  barColor: string;
} {
  const pct = budget ? Math.min(100, (spent / budget) * 100) : 0;
  const isOverBudget = budget !== null && spent >= budget;
  const isWarning = budget !== null && pct >= 80 && !isOverBudget;
  const barColor = isOverBudget
    ? "var(--color-danger)"
    : isWarning
      ? "var(--color-warning)"
      : "var(--color-success)";
  return { pct, isOverBudget, isWarning, barColor };
}
