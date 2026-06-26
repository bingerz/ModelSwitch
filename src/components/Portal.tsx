import { centsToUSD, formatTime, computeProgressState } from "./portal-utils";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { API_BASE } from "../lib/runtime";

// ─── Types ─────────────────────────────────────────────

interface PortalUsage {
  key_name: string;
  key_prefix: string;
  daily_budget_cents: number | null;
  monthly_budget_cents: number | null;
  daily_spent_cents: number;
  monthly_spent_cents: number;
  total_spent_cents: number;
  rpm_limit: number | null;
  tpm_limit: number | null;
  expires_at: string | null;
  allowed_models: string[] | null;
  is_active: boolean;
  is_expired: boolean;
}

interface PortalLog {
  id: string;
  timestamp: string;
  request_model: string;
  channel_name: string;
  latency_ms: number;
  success: boolean;
  estimated_cost: number | null;
  input_tokens: number | null;
  output_tokens: number | null;
}

// ─── Helpers ───────────────────────────────────────────



async function portalFetch<T>(
  path: string,
  token: string
): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
  });
  if (res.status === 401) {
    throw new Error("INVALID_KEY");
  }
  if (!res.ok) {
    throw new Error(`API error: ${res.status}`);
  }
  const json = await res.json();
  if (json && typeof json === "object" && "ok" in json && "data" in json) {
    return json.data as T;
  }
  return json as T;
}

// ─── Progress Bar ──────────────────────────────────────

interface ProgressBarProps {
  label: string;
  spent: number;
  budget: number | null;
}

function ProgressBar({ label, spent, budget }: ProgressBarProps) {
  const { t } = useTranslation();
  const { pct, barColor } = computeProgressState(spent, budget);

  return (
    <div className="portal-progress-item">
      <div className="portal-progress-header">
        <span className="portal-progress-label">{label}</span>
        <span className="portal-progress-values">
          {budget !== null ? (
            <>
              {centsToUSD(spent)} / {centsToUSD(budget)} ({pct.toFixed(0)}%)
            </>
          ) : (
            <>
              {centsToUSD(spent)} · {t("portal.unlimited")}
            </>
          )}
        </span>
      </div>
      <div className="portal-progress-track">
        <div
          className="portal-progress-fill"
          style={{
            width: budget ? `${pct}%` : "100%",
            background: budget ? barColor : "var(--color-accent)",
            opacity: budget ? 1 : 0.3,
          }}
        />
      </div>
    </div>
  );
}

// ─── Portal Dashboard ──────────────────────────────────

function PortalDashboard({ token, onLogout }: { token: string; onLogout: () => void }) {
  const { t } = useTranslation();
  const [usage, setUsage] = useState<PortalUsage | null>(null);
  const [logs, setLogs] = useState<PortalLog[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  const fetchData = useCallback(async () => {
    try {
      const [u, l] = await Promise.all([
        portalFetch<PortalUsage>("/api/portal/usage", token),
        portalFetch<PortalLog[]>("/api/portal/logs?limit=10", token),
      ]);
      setUsage(u);
      setLogs(l);
      setError("");
    } catch (err) {
      if (err instanceof Error && err.message === "INVALID_KEY") {
        onLogout();
      } else {
        setError(err instanceof Error ? err.message : "Unknown error");
      }
    } finally {
      setLoading(false);
    }
  }, [token, onLogout]);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 30_000);
    return () => clearInterval(interval);
  }, [fetchData]);

  if (loading && !usage) {
    return <div className="portal-loading">{t("common.loading")}</div>;
  }

  if (error && !usage) {
    return (
      <div className="portal-error-state">
        <p>{t("portal.loadFailed")}</p>
        <p className="portal-error-detail">{error}</p>
        <button className="portal-btn" onClick={onLogout}>
          {t("portal.backToLogin")}
        </button>
      </div>
    );
  }

  if (!usage) return null;

  const statusClass = !usage.is_active
    ? "portal-status-disabled"
    : usage.is_expired
      ? "portal-status-expired"
      : "portal-status-active";

  const statusLabel = !usage.is_active
    ? t("portal.statusDisabled")
    : usage.is_expired
      ? t("portal.statusExpired")
      : t("portal.statusActive");

  return (
    <div className="portal-dashboard">
      <header className="portal-header">
        <div className="portal-header-left">
          <h1 className="portal-title">{t("portal.title")}</h1>
          <div className="portal-key-info">
            <span className="portal-key-name">{usage.key_name}</span>
            <span className="portal-key-prefix">{usage.key_prefix}…</span>
          </div>
        </div>
        <div className="portal-header-right">
          <span className={`portal-status-badge ${statusClass}`}>
            {statusLabel}
          </span>
          <button className="portal-btn portal-btn-ghost" onClick={onLogout}>
            {t("portal.logout")}
          </button>
        </div>
      </header>

      {error && <div className="portal-warning">{t("portal.refreshError")}</div>}

      {/* Budget Section */}
      <section className="portal-section">
        <h2 className="portal-section-title">{t("portal.budgetTitle")}</h2>
        <div className="portal-budget-grid">
          <ProgressBar
            label={t("portal.dailyBudget")}
            spent={usage.daily_spent_cents}
            budget={usage.daily_budget_cents}
          />
          <ProgressBar
            label={t("portal.monthlyBudget")}
            spent={usage.monthly_spent_cents}
            budget={usage.monthly_budget_cents}
          />
        </div>
        <div className="portal-total-spent">
          {t("portal.totalSpent")}: <strong>{centsToUSD(usage.total_spent_cents)}</strong>
        </div>
      </section>

      {/* Key Details Section */}
      <section className="portal-section">
        <h2 className="portal-section-title">{t("portal.detailsTitle")}</h2>
        <div className="portal-details-grid">
          {usage.allowed_models !== null && (
            <div className="portal-detail-item">
              <span className="portal-detail-label">{t("portal.allowedModels")}</span>
              <div className="portal-model-tags">
                {usage.allowed_models.length > 0 ? (
                  usage.allowed_models.map((m) => (
                    <span key={m} className="portal-model-tag">{m}</span>
                  ))
                ) : (
                  <span className="portal-detail-muted">{t("portal.noModels")}</span>
                )}
              </div>
            </div>
          )}
          {usage.rpm_limit !== null && (
            <div className="portal-detail-item">
              <span className="portal-detail-label">{t("portal.rpmLimit")}</span>
              <span className="portal-detail-value">{usage.rpm_limit}</span>
            </div>
          )}
          {usage.tpm_limit !== null && (
            <div className="portal-detail-item">
              <span className="portal-detail-label">{t("portal.tpmLimit")}</span>
              <span className="portal-detail-value">{usage.tpm_limit}</span>
            </div>
          )}
          {usage.expires_at && (
            <div className="portal-detail-item">
              <span className="portal-detail-label">{t("portal.expiresAt")}</span>
              <span className="portal-detail-value">{formatTime(usage.expires_at)}</span>
            </div>
          )}
        </div>
      </section>

      {/* Recent Activity Section */}
      <section className="portal-section">
        <h2 className="portal-section-title">{t("portal.recentActivity")}</h2>
        {logs.length === 0 ? (
          <div className="portal-empty-logs">{t("portal.noLogs")}</div>
        ) : (
          <div className="portal-table-wrapper">
            <table className="portal-table">
              <thead>
                <tr>
                  <th>{t("portal.colTime")}</th>
                  <th>{t("portal.colModel")}</th>
                  <th>{t("portal.colTokens")}</th>
                  <th>{t("portal.colCost")}</th>
                  <th>{t("portal.colStatus")}</th>
                </tr>
              </thead>
              <tbody>
                {logs.map((log) => {
                  const totalTokens =
                    (log.input_tokens ?? 0) + (log.output_tokens ?? 0);
                  return (
                    <tr key={log.id}>
                      <td className="portal-cell-time">{formatTime(log.timestamp)}</td>
                      <td className="portal-cell-model">{log.request_model}</td>
                      <td className="portal-cell-tokens">
                        {totalTokens > 0 ? totalTokens.toLocaleString() : "—"}
                      </td>
                      <td className="portal-cell-cost">
                        {log.estimated_cost !== null
                          ? `$${log.estimated_cost.toFixed(4)}`
                          : "—"}
                      </td>
                      <td>
                        <span
                          className={`portal-log-status ${
                            log.success ? "portal-log-ok" : "portal-log-fail"
                          }`}
                        >
                          {log.success ? t("common.ok") : t("common.fail")}
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <footer className="portal-footer">
        <span className="portal-auto-refresh">{t("portal.autoRefresh")}</span>
      </footer>
    </div>
  );
}

// ─── Portal Login ──────────────────────────────────────

function PortalLogin({ onSuccess }: { onSuccess: (token: string) => void }) {
  const { t } = useTranslation();
  const [token, setToken] = useState("");
  const [error, setError] = useState("");
  const [testing, setTesting] = useState(false);

  const handleLogin = async () => {
    const trimmed = token.trim();
    if (!trimmed) {
      setError(t("portal.keyRequired"));
      return;
    }

    setTesting(true);
    setError("");

    try {
      await portalFetch<{ message: string }>("/api/portal/test", trimmed);
      onSuccess(trimmed);
    } catch {
      setError(t("portal.invalidKey"));
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="portal-login-page">
      <div className="portal-login-card">
        <div className="portal-login-brand">{t("portal.title")}</div>
        <div className="portal-login-subtitle">{t("portal.loginSubtitle")}</div>
        <input
          className="portal-login-input"
          type="password"
          placeholder={t("portal.keyPlaceholder")}
          value={token}
          onChange={(e) => setToken(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !testing) handleLogin();
          }}
          autoFocus
        />
        {error && <div className="portal-login-error">{error}</div>}
        <button
          className="portal-login-button"
          onClick={handleLogin}
          disabled={testing}
        >
          {testing ? t("common.loading") : t("portal.loginButton")}
        </button>
        <div className="portal-login-hint">{t("portal.loginHint")}</div>
      </div>
    </div>
  );
}

// ─── Portal Root ───────────────────────────────────────

export function Portal() {
  const { t, i18n } = useTranslation();
  const [token, setToken] = useState<string | null>(
    () => sessionStorage.getItem("portal_token")
  );

  const handleLogout = useCallback(() => {
    sessionStorage.removeItem("portal_token");
    setToken(null);
  }, []);

  const toggleLang = () => {
    const next = i18n.language === "en" ? "zh" : "en";
    i18n.changeLanguage(next);
  };

  // Initialize language from storage or browser preference
  useEffect(() => {
    const stored = localStorage.getItem("lang");
    if (stored === "en" || stored === "zh") {
      i18n.changeLanguage(stored);
    } else if (navigator.language.startsWith("zh")) {
      i18n.changeLanguage("zh");
    }
  }, [i18n]);

  return (
    <div className="portal-root">
      <button
        className="portal-lang-toggle"
        onClick={toggleLang}
        title={t("common.switchLang")}
      >
        {i18n.language === "en" ? "中文" : "EN"}
      </button>
      {token ? (
        <PortalDashboard token={token} onLogout={handleLogout} />
      ) : (
        <PortalLogin
          onSuccess={(t) => {
            sessionStorage.setItem("portal_token", t);
            setToken(t);
          }}
        />
      )}
    </div>
  );
}
