import type { ApiFormat } from "../../lib/presets";
import type { QuotaInfo } from "../../lib/api";

export const API_FORMAT_COLORS: Record<ApiFormat, string> = {
  openai: "#10A37F",
  anthropic: "#D97757",
  gemini: "#4285F4",
};

export const API_FORMAT_LABELS: Record<ApiFormat, string> = {
  openai: "OpenAI Chat",
  anthropic: "Anthropic",
  gemini: "Gemini",
};

export const PROVIDER_BADGES: Record<string, { bg: string; color: string }> = {
  openai: { bg: "rgba(16, 163, 127, 0.15)", color: "#10A37F" },
  anthropic: { bg: "rgba(217, 119, 87, 0.15)", color: "#D97757" },
  deepseek: { bg: "rgba(99, 102, 241, 0.15)", color: "#6366F1" },
  gemini: { bg: "rgba(66, 133, 244, 0.15)", color: "#4285F4" },
  openrouter: { bg: "rgba(168, 85, 247, 0.15)", color: "#A855F7" },
};

/** Derive API format from the channel's provider string. */
export function providerToFormat(provider: string): ApiFormat {
  if (provider === "anthropic") return "anthropic";
  if (provider === "gemini") return "gemini";
  return "openai";
}

export function providerBadgeStyle(provider: string): React.CSSProperties {
  const badge = PROVIDER_BADGES[provider];
  if (badge) return { backgroundColor: badge.bg, color: badge.color };
  return {
    backgroundColor: "var(--color-surface-hover)",
    color: "var(--color-text-secondary)",
  };
}

/** Format ISO timestamp into a human-readable recovery countdown. */
export function formatRecoveryTime(isoUntil: string): string {
  const until = new Date(isoUntil);
  const now = new Date();
  const diffMs = until.getTime() - now.getTime();
  if (diffMs <= 0) return "recovering";
  const mins = Math.floor(diffMs / 60000);
  const secs = Math.floor((diffMs % 60000) / 1000);
  if (mins > 0) return `${mins}m ${secs}s`;
  return `${secs}s`;
}

/** Compact quota badge for inline display in channel cards. */
export function QuotaBadge({ quota }: { quota: QuotaInfo | undefined }) {
  if (!quota || quota.error) return null;

  const hasBalance = quota.balance != null;
  const hasRateLimit = quota.rate_limit_remaining_req != null;

  if (!hasBalance && !hasRateLimit) return null;

  if (hasBalance) {
    const pct =
      quota.limit != null && quota.limit > 0
        ? ((quota.limit - quota.balance!) / quota.limit) * 100
        : null;
    const color = pct != null
      ? pct > 80
        ? "var(--color-danger)"
        : pct > 50
          ? "var(--color-warning)"
          : "var(--color-success)"
      : "var(--color-success)";
    return (
      <span className="meta-tag mono" style={{ color, fontSize: "0.7rem" }}>
        ${quota.balance!.toFixed(2)}
      </span>
    );
  }

  if (hasRateLimit && quota.rate_limit_limit_req != null) {
    return (
      <span className="meta-tag mono" style={{ fontSize: "0.7rem" }}>
        {quota.rate_limit_remaining_req}/{quota.rate_limit_limit_req} RPM
      </span>
    );
  }

  return null;
}
