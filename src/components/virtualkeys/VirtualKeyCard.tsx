import { useTranslation } from "react-i18next";
import type { VirtualKey } from "../../lib/api";
import { formatCents } from "../../lib/format";
import { computeBudgetBar, formatDate } from "./types";

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
  const monthlyBar = computeBudgetBar(
    vk.spend.this_month.cents,
    vk.monthly_budget_cents,
  );
  const monthlyLabel = monthlyBar.unlimited
    ? `${monthlyBar.spendLabel} / ${t("virtualKeys.unlimited")}`
    : monthlyBar.label;

  const rateLimit = [
    vk.rpm_limit !== null && `${formatCompactNumber(vk.rpm_limit)} RPM`,
    vk.tpm_limit !== null && `${formatCompactNumber(vk.tpm_limit)} TPM`,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div className={`vk-row ${vk.enabled ? "" : "vk-row-disabled"}`}>
      <div className="vk-row-name">
        <span
          className={`status-dot ${vk.enabled ? "healthy" : ""}`}
          style={{
            background: vk.enabled
              ? "var(--color-success)"
              : "var(--color-text-muted)",
          }}
          aria-hidden="true"
        />
        <div className="vk-row-name-text">
          <strong>{vk.name}</strong>
          <span className="vk-row-prefix mono">{vk.key_prefix}…</span>
        </div>
      </div>

      <div className="vk-row-status">
        <span className={`vk-status-pill ${vk.enabled ? "on" : "off"}`}>
          {vk.enabled ? t("virtualKeys.enabled") : t("virtualKeys.disabled")}
        </span>
        {vk.group ? <span className="meta-tag">{vk.group}</span> : null}
      </div>

      <div className="vk-row-spend" title={monthlyLabel}>
        <div className="vk-row-spend-value">{monthlyLabel}</div>
        <div className="vk-budget-bar vk-row-spend-bar">
          <div
            className="vk-budget-fill"
            style={{
              width: `${monthlyBar.pct}%`,
              background: monthlyBar.color,
            }}
          />
        </div>
      </div>

      <div className="vk-row-meta mono">
        {rateLimit || "—"}
        {vk.expires_at ? (
          <span className="vk-row-expires">
            {t("virtualKeys.expiresOn", { date: formatDate(vk.expires_at) })}
          </span>
        ) : null}
      </div>

      <div className="vk-row-created">{formatDate(vk.created_at)}</div>

      <div className="vk-row-actions">
        <button
          className="btn btn-sm"
          onClick={onToggleEnabled}
          disabled={actionLoading}
          aria-label={
            vk.enabled ? t("common.disable") : t("common.enable")
          }
        >
          {actionLoading
            ? "..."
            : vk.enabled
              ? t("common.disable")
              : t("common.enable")}
        </button>
        <button
          className="btn btn-sm"
          onClick={onEdit}
          disabled={actionLoading}
        >
          {t("common.edit")}
        </button>
        <button
          className={`btn btn-sm ${confirmDelete ? "btn-danger" : ""}`}
          style={confirmDelete ? undefined : { color: "var(--color-danger)" }}
          onClick={onDelete}
          disabled={actionLoading}
        >
          {confirmDelete ? t("common.confirmQuestion") : t("common.delete")}
        </button>
      </div>

      <div className="vk-row-footer">
        <span className="meta-tag">
          {t("virtualKeys.totalSpent", {
            amount: formatCents(vk.spend.total_cents),
          })}
        </span>
        {vk.allowed_ips.length > 0 && (
          <span className="meta-tag" title={vk.allowed_ips.join(", ")}>
            IP ×{vk.allowed_ips.length}
          </span>
        )}
        {vk.allowed_models && vk.allowed_models.length > 0 && (
          <span className="meta-tag" title={vk.allowed_models.join(", ")}>
            {t("common.modelsCount", { count: vk.allowed_models.length })} ✓
          </span>
        )}
        {vk.denied_models.length > 0 && (
          <span className="meta-tag" title={vk.denied_models.join(", ")}>
            {t("common.modelsCount", { count: vk.denied_models.length })} ✕
          </span>
        )}
        <span className="meta-tag mono">{vk.id}</span>
      </div>
    </div>
  );
}
