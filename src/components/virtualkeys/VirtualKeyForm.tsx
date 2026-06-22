import { useState } from "react";
import {
  api,
  type VirtualKey,
  type CreateVirtualKeyResponse,
  type CreateVirtualKeyData,
  type UpdateVirtualKeyData,
} from "../../lib/api";
import { useToast } from "../Toast";
import { centsToDollars, dollarsToCents } from "./types";

export interface VirtualKeyFormProps {
  mode: "create" | "edit";
  existingKey?: VirtualKey;
  onSave: (response: CreateVirtualKeyResponse | null) => void;
  onCancel: () => void;
}

export function VirtualKeyForm({
  mode,
  existingKey,
  onSave,
  onCancel,
}: VirtualKeyFormProps) {
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
