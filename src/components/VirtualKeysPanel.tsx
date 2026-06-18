import { useCallback, useEffect, useState } from "react";
import {
  api,
  type VirtualKey,
  type CreateVirtualKeyResponse,
  type CreateVirtualKeyData,
  type UpdateVirtualKeyData,
} from "../lib/api";
import { useToast } from "./Toast";
import "../styles/pages-enhanced.css";

// ─── Formatting Helpers ─────────────────────────────────

function formatCents(cents: number): string {
  return `$${(cents / 100).toFixed(2)}`;
}

function formatDate(iso: string): string {
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

const CENTS_PER_DOLLAR = 100;

function dollarsToCents(dollars: string): number | null {
  const trimmed = dollars.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  if (!Number.isFinite(value) || value < 0) return null;
  return Math.round(value * CENTS_PER_DOLLAR);
}

function centsToDollars(cents: number | null): string {
  if (cents === null) return "";
  const dollars = cents / CENTS_PER_DOLLAR;
  // Strip trailing zeros for clean display, but keep at most 2 decimals.
  return Number.isInteger(dollars) ? String(dollars) : dollars.toFixed(2);
}

// ─── Budget Bar Helpers ─────────────────────────────────

interface BudgetBar {
  pct: number;
  color: string;
  label: string;
}

function computeBudgetBar(spendCents: number, budgetCents: number | null): BudgetBar {
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

// ─── Main Panel ─────────────────────────────────────────

export function VirtualKeysPanel() {
  const toast = useToast();
  const [keys, setKeys] = useState<VirtualKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [showAddForm, setShowAddForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [createdResponse, setCreatedResponse] =
    useState<CreateVirtualKeyResponse | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await api.virtualKeys.list();
      setKeys(list);
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to load virtual keys",
      );
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleCreated = (response: CreateVirtualKeyResponse) => {
    setCreatedResponse(response);
    setShowAddForm(false);
    refresh();
  };

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) {
      setConfirmDeleteId(id);
      return;
    }
    setConfirmDeleteId(null);
    setActionLoading(id);
    try {
      await api.virtualKeys.delete(id);
      toast.success("Virtual key deleted");
      await refresh();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to delete virtual key",
      );
    } finally {
      setActionLoading(null);
    }
  };

  const handleToggleEnabled = async (vk: VirtualKey) => {
    setActionLoading(vk.id);
    try {
      await api.virtualKeys.update(vk.id, { enabled: !vk.enabled });
      toast.success(vk.enabled ? "Key disabled" : "Key enabled");
      await refresh();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to update key",
      );
    } finally {
      setActionLoading(null);
    }
  };

  if (loading) {
    return (
      <div className="panel-loading-enhanced">
        <div className="spinner" />
        <span>Loading virtual keys...</span>
      </div>
    );
  }

  return (
    <section className="vk-panel">
      <div className="panel-header">
        <h2 className="panel-title">Virtual Keys</h2>
        <button
          className="btn btn-primary"
          onClick={() => setShowAddForm(!showAddForm)}
        >
          {showAddForm ? "Cancel" : "+ Add Key"}
        </button>
      </div>

      {createdResponse && (
        <PlaintextBanner
          response={createdResponse}
          onClose={() => setCreatedResponse(null)}
        />
      )}

      {showAddForm && (
        <VirtualKeyForm
          mode="create"
          onSave={(resp) => {
            if (resp) {
              handleCreated(resp);
            } else {
              setShowAddForm(false);
              refresh();
            }
          }}
          onCancel={() => setShowAddForm(false)}
        />
      )}

      {/* Summary stat cards */}
      {keys.length > 0 && (
        <div className="vk-summary-grid">
          <div className="stat-card">
            <span className="stat-card-icon">🔑</span>
            <div className="stat-value">{keys.length}</div>
            <div className="stat-label">Total Keys</div>
          </div>
          <div className="stat-card">
            <span className="stat-card-icon">✅</span>
            <div className="stat-value">{keys.filter((k) => k.enabled).length}</div>
            <div className="stat-label">Active Keys</div>
          </div>
          <div className="stat-card">
            <span className="stat-card-icon">💰</span>
            <div className="stat-value">
              {formatCents(keys.reduce((sum, k) => sum + k.spend.this_month.cents, 0))}
            </div>
            <div className="stat-label">Spend This Month</div>
          </div>
        </div>
      )}

      {keys.length === 0 && !showAddForm ? (
        <div className="empty-state">
          <div className="empty-state-icon">🔑</div>
          <div className="empty-state-title">No virtual keys configured</div>
          <div className="empty-state-description">
            Create virtual keys to distribute access with per-key budgets and rate limits. Click "Add Key" to get started.
          </div>
        </div>
      ) : (
        <div className="vk-list">
          {keys.map((vk) => {
            if (editingId === vk.id) {
              return (
                <VirtualKeyForm
                  key={vk.id}
                  mode="edit"
                  existingKey={vk}
                  onSave={() => {
                    setEditingId(null);
                    refresh();
                  }}
                  onCancel={() => setEditingId(null)}
                />
              );
            }
            return (
              <VirtualKeyCard
                key={vk.id}
                vk={vk}
                confirmDelete={confirmDeleteId === vk.id}
                actionLoading={actionLoading === vk.id}
                onEdit={() => setEditingId(vk.id)}
                onDelete={() => handleDelete(vk.id)}
                onToggleEnabled={() => handleToggleEnabled(vk)}
              />
            );
          })}
        </div>
      )}
    </section>
  );
}

// ─── Plaintext Banner (one-time display) ────────────────

function PlaintextBanner({
  response,
  onClose,
}: {
  response: CreateVirtualKeyResponse;
  onClose: () => void;
}) {
  const toast = useToast();
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(response.plaintext);
      setCopied(true);
      toast.success("Copied to clipboard");
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  };

  return (
    <div className="vk-plaintext-banner">
      <div className="vk-plaintext-header">
        <strong>New key created: {response.key.name}</strong>
        <button
          className="btn btn-sm"
          onClick={onClose}
          aria-label="Dismiss"
          title="Dismiss"
        >
          Close
        </button>
      </div>
      <div className="vk-plaintext-warning">
        This key will not be shown again. Copy it now and store it securely.
      </div>
      <div className="vk-plaintext-key-row">
        <code className="vk-plaintext-key mono">{response.plaintext}</code>
        <button
          className={`btn btn-sm ${copied ? "btn-primary" : ""}`}
          onClick={handleCopy}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
    </div>
  );
}

// ─── Virtual Key Card ───────────────────────────────────

interface VirtualKeyCardProps {
  vk: VirtualKey;
  confirmDelete: boolean;
  actionLoading: boolean;
  onEdit: () => void;
  onDelete: () => void;
  onToggleEnabled: () => void;
}

function VirtualKeyCard({
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

// ─── Virtual Key Form (Create + Edit) ───────────────────

interface VirtualKeyFormProps {
  mode: "create" | "edit";
  existingKey?: VirtualKey;
  onSave: (response: CreateVirtualKeyResponse | null) => void;
  onCancel: () => void;
}

function VirtualKeyForm({ mode, existingKey, onSave, onCancel }: VirtualKeyFormProps) {
  const toast = useToast();
  const [name, setName] = useState(existingKey?.name ?? "");
  const [dailyBudget, setDailyBudget] = useState(
    centsToDollars(existingKey?.daily_budget_cents ?? null),
  );
  const [monthlyBudget, setMonthlyBudget] = useState(
    centsToDollars(existingKey?.monthly_budget_cents ?? null),
  );
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (!name.trim()) {
      setError("Name is required");
      return;
    }

    const dailyCents = dollarsToCents(dailyBudget);
    const monthlyCents = dollarsToCents(monthlyBudget);

    if (dailyBudget.trim() !== "" && dailyCents === null) {
      setError("Daily budget must be a non-negative number");
      return;
    }
    if (monthlyBudget.trim() !== "" && monthlyCents === null) {
      setError("Monthly budget must be a non-negative number");
      return;
    }

    setSubmitting(true);
    try {
      if (mode === "create") {
        const payload: CreateVirtualKeyData = {
          name: name.trim(),
          daily_budget_cents: dailyCents,
          monthly_budget_cents: monthlyCents,
        };
        const response = await api.virtualKeys.create(payload);
        toast.success("Virtual key created");
        onSave(response);
      } else if (existingKey) {
        const payload: UpdateVirtualKeyData = {
          name: name.trim(),
          daily_budget_cents: dailyCents,
          monthly_budget_cents: monthlyCents,
        };
        await api.virtualKeys.update(existingKey.id, payload);
        toast.success("Virtual key updated");
        onSave(null);
      }
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : "Failed to save virtual key";
      setError(msg);
      toast.error(msg);
    } finally {
      setSubmitting(false);
    }
  };

  const title = mode === "create" ? "Add Virtual Key" : `Edit: ${existingKey?.name}`;

  return (
    <form className="channel-form" onSubmit={handleSubmit}>
      <h3 className="form-title">{title}</h3>
      <div className="form-grid">
        <label className="form-field">
          <span>Name</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. Production App"
            required
          />
        </label>
        <label className="form-field">
          <span>Daily Budget (USD, optional)</span>
          <input
            value={dailyBudget}
            onChange={(e) => setDailyBudget(e.target.value)}
            placeholder="e.g. 10.00"
            inputMode="decimal"
          />
        </label>
        <label className="form-field">
          <span>Monthly Budget (USD, optional)</span>
          <input
            value={monthlyBudget}
            onChange={(e) => setMonthlyBudget(e.target.value)}
            placeholder="e.g. 250.00"
            inputMode="decimal"
          />
        </label>
      </div>
      {error && <div className="form-error">{error}</div>}
      <div style={{ display: "flex", gap: "var(--space-2)" }}>
        <button type="submit" className="btn btn-primary" disabled={submitting}>
          {submitting
            ? "Saving..."
            : mode === "create"
              ? "Create Key"
              : "Save"}
        </button>
        <button
          type="button"
          className="btn"
          onClick={onCancel}
          disabled={submitting}
        >
          Cancel
        </button>
      </div>
    </form>
  );
}
