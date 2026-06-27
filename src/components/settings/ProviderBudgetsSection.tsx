import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Wallet } from "lucide-react";
import { api, type ProviderBudgetEntry } from "../../lib/api";
import { formatCents } from "../../lib/format";
import { useToast } from "../Toast";
import { ProgressBar } from "../ui/ProgressBar";

interface ProviderBudgetsSectionProps {
  budgets: ProviderBudgetEntry[];
  onRefresh: () => Promise<void>;
}

interface BudgetFormState {
  provider: string;
  daily: string;
  monthly: string;
}

const EMPTY_BUDGET_FORM: BudgetFormState = {
  provider: "",
  daily: "",
  monthly: "",
};

/** Provider budget table + add/remove form. */
export function ProviderBudgetsSection({
  budgets,
  onRefresh,
}: ProviderBudgetsSectionProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [showBudgetForm, setShowBudgetForm] = useState(false);
  const [budgetForm, setBudgetForm] = useState<BudgetFormState>(EMPTY_BUDGET_FORM);
  const [budgetSaving, setBudgetSaving] = useState(false);
  const [deletingBudget, setDeletingBudget] = useState<string | null>(null);

  const resetBudgetForm = () => {
    setBudgetForm(EMPTY_BUDGET_FORM);
    setShowBudgetForm(false);
  };

  const handleSaveBudget = async () => {
    const provider = budgetForm.provider.trim();
    if (!provider) {
      toast.error(t("settings.providerNameRequired"));
      return;
    }
    const daily = budgetForm.daily.trim() === "" ? null : Number(budgetForm.daily);
    const monthly =
      budgetForm.monthly.trim() === "" ? null : Number(budgetForm.monthly);
    if (daily != null && (!Number.isFinite(daily) || daily < 0)) {
      toast.error(t("settings.budgetInvalid"));
      return;
    }
    if (monthly != null && (!Number.isFinite(monthly) || monthly < 0)) {
      toast.error(t("settings.budgetInvalid"));
      return;
    }
    if (daily != null && monthly != null && monthly < daily) {
      toast.error(t("settings.monthlyBelowDaily"));
      return;
    }
    setBudgetSaving(true);
    try {
      await api.setProviderBudget(provider, {
        daily_budget_cents: daily,
        monthly_budget_cents: monthly,
      });
      toast.success(t("settings.budgetSaved"));
      resetBudgetForm();
      await onRefresh();
    } catch {
      toast.error(t("settings.budgetSaveFailed"));
    } finally {
      setBudgetSaving(false);
    }
  };

  const handleDeleteBudget = async (provider: string) => {
    setDeletingBudget(provider);
    try {
      await api.deleteProviderBudget(provider);
      toast.success(t("settings.budgetDeleted"));
      await onRefresh();
    } catch {
      toast.error(t("settings.budgetDeleteFailed"));
    } finally {
      setDeletingBudget(null);
    }
  };

  return (
    <div className="settings-section">
      <h3 className="settings-section-title">
        <Wallet size={14} className="icon-inline" />
        {t("settings.providerBudgets")}
      </h3>
      {budgets.length > 0 && (
        <div className="settings-table-wrapper">
          <table className="settings-table">
            <caption className="sr-only">
              {t("settings.providerBudgets")}
            </caption>
            <thead>
              <tr>
                <th scope="col">{t("common.provider")}</th>
                <th scope="col">{t("settings.today")}</th>
                <th scope="col">{t("settings.dailyBudget")}</th>
                <th scope="col">{t("settings.thisMonthColumn")}</th>
                <th scope="col">{t("settings.monthlyBudget")}</th>
                <th scope="col">{t("settings.total")}</th>
                <th scope="col" aria-label={t("common.actions")} />
              </tr>
            </thead>
            <tbody>
              {budgets.map((b) => (
                <tr key={b.provider}>
                  <th scope="row" className="mono">
                    {b.provider}
                  </th>
                  <td className="mono">{formatCents(b.spend.today.cents)}</td>
                  <td className="mono">
                    {b.daily_budget_cents != null
                      ? formatCents(b.daily_budget_cents)
                      : "\u221e"}
                    {b.daily_budget_cents != null && (
                      <div style={{ marginTop: "4px", maxWidth: "120px" }}>
                        <ProgressBar
                          value={b.spend.today.cents}
                          max={b.daily_budget_cents}
                          thresholds={[
                            { upto: 50, color: "var(--color-success)" },
                            { upto: 80, color: "var(--color-warning)" },
                            { upto: 100, color: "var(--color-danger)" },
                          ]}
                          height={4}
                        />
                      </div>
                    )}
                  </td>
                  <td className="mono">
                    {formatCents(b.spend.this_month.cents)}
                  </td>
                  <td className="mono">
                    {b.monthly_budget_cents != null
                      ? formatCents(b.monthly_budget_cents)
                      : "\u221e"}
                    {b.monthly_budget_cents != null && (
                      <div style={{ marginTop: "4px", maxWidth: "120px" }}>
                        <ProgressBar
                          value={b.spend.this_month.cents}
                          max={b.monthly_budget_cents}
                          thresholds={[
                            { upto: 50, color: "var(--color-success)" },
                            { upto: 80, color: "var(--color-warning)" },
                            { upto: 100, color: "var(--color-danger)" },
                          ]}
                          height={4}
                        />
                      </div>
                    )}
                  </td>
                  <td className="mono">{formatCents(b.spend.total_cents)}</td>
                  <td>
                    <button
                      className="btn btn-sm"
                      style={{ color: "var(--color-danger)" }}
                      onClick={() => handleDeleteBudget(b.provider)}
                      disabled={deletingBudget === b.provider}
                    >
                      {deletingBudget === b.provider
                        ? t("common.deleting")
                        : t("settings.deleteBudget")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {showBudgetForm ? (
        <div
          style={{
            display: "flex",
            gap: "var(--space-2)",
            alignItems: "flex-end",
            flexWrap: "wrap",
            marginTop: "var(--space-2)",
          }}
        >
          <div>
            <label className="settings-stat-label">
              {t("settings.providerName")}
            </label>
            <input
              type="text"
              className="settings-input"
              value={budgetForm.provider}
              onChange={(e) =>
                setBudgetForm((prev) => ({ ...prev, provider: e.target.value }))
              }
              placeholder="e.g. openai"
              style={{ width: "150px" }}
            />
          </div>
          <div>
            <label className="settings-stat-label">
              {t("settings.budgetDailyCents")}
            </label>
            <input
              type="number"
              className="settings-input"
              value={budgetForm.daily}
              onChange={(e) =>
                setBudgetForm((prev) => ({ ...prev, daily: e.target.value }))
              }
              placeholder="e.g. 500"
              min="0"
              step="1"
              style={{ width: "120px" }}
            />
          </div>
          <div>
            <label className="settings-stat-label">
              {t("settings.budgetMonthlyCents")}
            </label>
            <input
              type="number"
              className="settings-input"
              value={budgetForm.monthly}
              onChange={(e) =>
                setBudgetForm((prev) => ({ ...prev, monthly: e.target.value }))
              }
              placeholder="e.g. 15000"
              min="0"
              step="1"
              style={{ width: "120px" }}
            />
          </div>
          <button
            className="btn btn-sm btn-primary"
            onClick={handleSaveBudget}
            disabled={budgetSaving}
          >
            {budgetSaving ? t("common.saving") : t("common.save")}
          </button>
          <button
            className="btn btn-sm"
            onClick={resetBudgetForm}
            disabled={budgetSaving}
          >
            {t("common.cancel")}
          </button>
        </div>
      ) : (
        <div className="settings-actions">
          <button className="btn btn-sm" onClick={() => setShowBudgetForm(true)}>
            {t("settings.addBudget")}
          </button>
        </div>
      )}
    </div>
  );
}
