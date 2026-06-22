import type { VirtualKey } from "../../lib/api";
import { formatCents } from "../../lib/format";
import {
  computeBudgetBar,
  formatDate,
} from "./types";

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
  const dailyBar = computeBudgetBar(vk.spend.today.cents, vk.daily_budget_cents);
  const monthlyBar = computeBudgetBar(
    vk.spend.this_month.cents,
    vk.monthly_budget_cents,
  );

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
            {vk.enabled ? "enabled" : "disabled"}
          </span>
        </div>
        <div className="vk-card-actions">
          <button
            className="btn btn-sm"
            onClick={onToggleEnabled}
            disabled={actionLoading}
          >
            {actionLoading ? "..." : vk.enabled ? "Disable" : "Enable"}
          </button>
          <button className="btn btn-sm" onClick={onEdit} disabled={actionLoading}>
            Edit
          </button>
          <button
            className={`btn btn-sm ${confirmDelete ? "btn-danger" : ""}`}
            style={
              confirmDelete ? undefined : { color: "var(--color-danger)" }
            }
            onClick={onDelete}
            disabled={actionLoading}
          >
            {confirmDelete ? "Confirm?" : "Delete"}
          </button>
        </div>
      </div>

      <div className="vk-budget-grid">
        <div className="vk-budget-row">
          <div className="vk-budget-label">
            <span className="vk-budget-window">Today</span>
            <span className="vk-budget-value">{dailyBar.label}</span>
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
              Month ({vk.spend.this_month.month})
            </span>
            <span className="vk-budget-value">{monthlyBar.label}</span>
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

      <div className="vk-card-footer">
        <span className="meta-tag">Total: {formatCents(vk.spend.total_cents)}</span>
        <span className="meta-tag">Created {formatDate(vk.created_at)}</span>
        <span className="meta-tag mono">{vk.id}</span>
      </div>
    </div>
  );
}
