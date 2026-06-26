import { useTranslation } from "react-i18next";
import type { VirtualKey } from "../../lib/api";
import { formatCents } from "../../lib/format";
import {
  computeBudgetBar,
  formatDate,
} from "./types";

/** Compact integer formatting for rate-limit badges (e.g. 100000 -> "100K"). */
function formatCompactNumber(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(value % 1_000_000 === 0 ? 0 : 1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(value % 1_000 === 0 ? 0 : 1)}K`;
  }
  return String(value);
}

export interface VirtualKeyCardProps {
  vk: VirtualKey;
  confirmDelete: boolean;
  actionLoading: boolean;
  onEdit: () => void;
  onDelete: () => void;
  onToggleEnabled: () => void;
}

export function VirtualKeyCard({
  vk,
  confirmDelete,
  actionLoading,
  onEdit,
  onDelete,
  onToggleEnabled,
}: VirtualKeyCardProps) {
  const { t } = useTranslation();
  const dailyBar = computeBudgetBar(vk.spend.today.cents, vk.daily_budget_cents);
  const monthlyBar = computeBudgetBar(
    vk.spend.this_month.cents,
    vk.monthly_budget_cents,
  );
  const dailyLabel = dailyBar.unlimited
    ? `${dailyBar.spendLabel} / ${t("virtualKeys.unlimited")}`
    : dailyBar.label;
  const monthlyLabel = monthlyBar.unlimited
    ? `${monthlyBar.spendLabel} / ${t("virtualKeys.unlimited")}`
    : monthlyBar.label;

  return (
    <div className="vk-card">
      <div className="vk-card-header">
        <div className="vk-card-title">
          <span
            className={`status-dot ${vk.enabled ? "healthy" : ""}`}
            style={{
              background: vk.enabled
                ? "var(--color-success)"
                : "var(--color-text-muted)",
            }}
          />
          <strong>{vk.name}</strong>
          <span className="meta-tag mono">{vk.key_prefix}...</span>
          <span
            className={`meta-tag ${vk.enabled ? "" : "channel-disabled-tag"}`}
          >
            {vk.enabled ? t("virtualKeys.enabled") : t("virtualKeys.disabled")}
          </span>
        </div>
        <div className="vk-card-actions">
          <button
            className="btn btn-sm"
            onClick={onToggleEnabled}
            disabled={actionLoading}
          >
            {actionLoading ? "..." : vk.enabled ? t("common.disable") : t("common.enable")}
          </button>
          <button className="btn btn-sm" onClick={onEdit} disabled={actionLoading}>
            {t("common.edit")}
          </button>
          <button
            className={`btn btn-sm ${confirmDelete ? "btn-danger" : ""}`}
            style={
              confirmDelete ? undefined : { color: "var(--color-danger)" }
            }
            onClick={onDelete}
            disabled={actionLoading}
          >
            {confirmDelete ? t("common.confirmQuestion") : t("common.delete")}
          </button>
        </div>
      </div>

      <div className="vk-budget-grid">
        <div className="vk-budget-row">
          <div className="vk-budget-label">
            <span className="vk-budget-window">{t("common.today")}</span>
            <span className="vk-budget-value">{dailyLabel}</span>
          </div>
          <div className="vk-budget-bar">
            <div
              className="vk-budget-fill"
              style={{
                width: `${dailyBar.pct}%`,
                background: dailyBar.color,
              }}
            />
          </div>
        </div>

        <div className="vk-budget-row">
          <div className="vk-budget-label">
            <span className="vk-budget-window">
              {t("virtualKeys.monthLabel", { month: vk.spend.this_month.month })}
            </span>
            <span className="vk-budget-value">{monthlyLabel}</span>
          </div>
          <div className="vk-budget-bar">
            <div
              className="vk-budget-fill"
              style={{
                width: `${monthlyBar.pct}%`,
                background: monthlyBar.color,
              }}
            />
          </div>
        </div>
      </div>

      {(vk.allowed_ips.length > 0 || (vk.allowed_models && vk.allowed_models.length > 0) || vk.denied_models.length > 0) && (
        <div className="vk-card-restrictions" style={{ display: "flex", gap: "var(--space-1)", flexWrap: "wrap", marginBottom: "var(--space-2)" }}>
          {vk.allowed_ips.length > 0 && (
            <span
              className="meta-tag"
              title={vk.allowed_ips.join(", ")}
              style={{ fontSize: "var(--text-xs)", background: "var(--color-bg-secondary)" }}
            >
              IP: {vk.allowed_ips.length > 2
                ? `${vk.allowed_ips[0]} +${vk.allowed_ips.length - 1}`
                : vk.allowed_ips.join(", ")}
            </span>
          )}
          {vk.allowed_models && vk.allowed_models.length > 0 && (
            <span
              className="meta-tag"
              title={vk.allowed_models.join(", ")}
              style={{ fontSize: "var(--text-xs)", background: "var(--color-success-bg, rgba(34,197,94,0.1))" }}
            >
              {t("common.modelsCount", { count: vk.allowed_models.length })} ✓
            </span>
          )}
          {vk.denied_models.length > 0 && (
            <span
              className="meta-tag"
              title={vk.denied_models.join(", ")}
              style={{ fontSize: "var(--text-xs)", background: "var(--color-danger-bg, rgba(239,68,68,0.1))" }}
            >
              {t("common.modelsCount", { count: vk.denied_models.length })} ✕
            </span>
          )}
        </div>
      )}

      {(vk.group || vk.rpm_limit !== null || vk.tpm_limit !== null || vk.expires_at) && (
        <div className="vk-card-advanced" style={{ display: "flex", gap: "var(--space-1)", flexWrap: "wrap", marginBottom: "var(--space-2)" }}>
          {vk.group && (
            <span
              className="meta-tag"
              title={t("virtualKeys.groupLabel")}
              style={{
                fontSize: "var(--text-xs)",
                background: "var(--color-bg-secondary)",
                fontWeight: 600,
              }}
            >
              {vk.group}
            </span>
          )}
          {(vk.rpm_limit !== null || vk.tpm_limit !== null) && (
            <span
              className="meta-tag mono"
              style={{ fontSize: "var(--text-xs)", background: "var(--color-bg-secondary)" }}
            >
              {[
                vk.rpm_limit !== null && `${formatCompactNumber(vk.rpm_limit)} RPM`,
                vk.tpm_limit !== null && `${formatCompactNumber(vk.tpm_limit)} TPM`,
              ].filter(Boolean).join(" · ")}
            </span>
          )}
          {vk.expires_at && (
            <span
              className="meta-tag"
              title={vk.expires_at}
              style={{
                fontSize: "var(--text-xs)",
                background: "var(--color-warning-bg, rgba(245,158,11,0.1))",
              }}
            >
              {t("virtualKeys.expiresOn", { date: formatDate(vk.expires_at) })}
            </span>
          )}
        </div>
      )}

      <div className="vk-card-footer">
        <span className="meta-tag">{t("virtualKeys.totalSpent", { amount: formatCents(vk.spend.total_cents) })}</span>
        <span className="meta-tag">{t("virtualKeys.createdDate", { date: formatDate(vk.created_at) })}</span>
        <span className="meta-tag mono">{vk.id}</span>
      </div>
    </div>
  );
}
