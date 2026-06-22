import type {
  VirtualKey,
  CreateVirtualKeyResponse,
} from "../../lib/api";
import { formatCents } from "../../lib/format";

// Re-export shared types so sibling components import from a single location.
export type { VirtualKey, CreateVirtualKeyResponse };

// ─── Formatting Helpers ─────────────────────────────────

export function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  } catch {
    return iso;
  }
}

export const CENTS_PER_DOLLAR = 100;

export function dollarsToCents(dollars: string): number | null {
  const trimmed = dollars.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  if (!Number.isFinite(value) || value < 0) return null;
  return Math.round(value * CENTS_PER_DOLLAR);
}

export function centsToDollars(cents: number | null): string {
  if (cents === null) return "";
  const dollars = cents / CENTS_PER_DOLLAR;
  // Strip trailing zeros for clean display, but keep at most 2 decimals.
  return Number.isInteger(dollars) ? String(dollars) : dollars.toFixed(2);
}

// ─── Budget Bar Helpers ─────────────────────────────────

export interface BudgetBar {
  pct: number;
  color: string;
  label: string;
}

export function computeBudgetBar(
  spendCents: number,
  budgetCents: number | null,
): BudgetBar {
  if (budgetCents === null || budgetCents <= 0) {
    return {
      pct: 0,
      color: "var(--color-text-muted)",
      label: `${formatCents(spendCents)} / unlimited`,
    };
  }
  const pct = Math.min(100, (spendCents / budgetCents) * 100);
  let color = "var(--color-success)";
  if (pct >= 80) color = "var(--color-danger)";
  else if (pct >= 50) color = "var(--color-warning)";
  return {
    pct,
    color,
    label: `${formatCents(spendCents)} / ${formatCents(budgetCents)}`,
  };
}
