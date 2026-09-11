import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

export function LoginPage({ onSuccess }: { onSuccess: () => void }) {
  const { t, i18n } = useTranslation();
  const [token, setToken] = useState("");
  const [error, setError] = useState("");
  const [expiredMsg, setExpiredMsg] = useState(false);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (sessionStorage.getItem("auth_expired") === "1") {
      sessionStorage.removeItem("auth_expired");
      setExpiredMsg(true);
    }
  }, []);

  const toggleLanguage = () => {
    const next = i18n.language === "en" ? "zh" : "en";
    i18n.changeLanguage(next);
    localStorage.setItem("lang", next);
  };

  const handleLogin = async () => {
    if (loading) return;
    setLoading(true);
    setError("");
    localStorage.setItem("admin_token", token);
    try {
      const { request } = await import("../lib/api");
      await request("/api/gateway/info");
      onSuccess();
    } catch (err) {
      localStorage.removeItem("admin_token");
      if (err instanceof TypeError && err.message.includes("fetch")) {
        setError(t("login.networkError"));
      } else {
        setError(t("login.error"));
      }
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="login-page">
      <button
        type="button"
        className="login-lang-toggle"
        onClick={toggleLanguage}
        aria-label={t("common.switchLang")}
      >
        {i18n.language === "en" ? "中文" : "EN"}
      </button>
      <div className="login-card">
        <div className="login-brand">{t("login.brand")}</div>
        <div className="login-subtitle">{t("login.subtitle")}</div>
        <div className="login-mode-hint" style={{
          padding: "var(--space-3)",
          marginBottom: "var(--space-3)",
          background: "var(--color-bg-secondary)",
          borderRadius: "var(--radius-md)",
          fontSize: "var(--text-sm)",
          lineHeight: "1.5"
        }}>
          <strong>{t("login.modeTitle")}</strong>
          <div style={{ marginTop: "var(--space-2)" }}>
            {t("login.modeExplanation")}
          </div>
          <ul style={{
            marginTop: "var(--space-2)",
            marginLeft: "var(--space-4)",
            listStyle: "disc"
          }}>
            <li><strong>{t("login.adminMode")}:</strong> {t("login.adminModeDesc")}</li>
            <li><strong>{t("login.portalMode")}:</strong> {t("login.portalModeDesc")}</li>
          </ul>
        </div>
        <label className="login-label" htmlFor="login-token">
          {t("login.tokenLabel")}
        </label>
        <input
          id="login-token"
          className="login-input"
          type="password"
          placeholder={t("login.tokenPlaceholder")}
          value={token}
          onChange={(e) => setToken(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleLogin()}
          autoFocus
          disabled={loading}
          autoComplete="current-password"
        />
        {expiredMsg && (
          <div className="login-warning" role="status">
            {t("login.sessionExpired")}
          </div>
        )}
        {error && (
          <div className="login-error" role="alert">
            {error}
          </div>
        )}
        <button
          className="login-button"
          onClick={handleLogin}
          disabled={loading}
          aria-busy={loading}
        >
          {loading ? t("login.buttonLoading") : t("login.button")}
        </button>
        <div className="login-hint">{t("login.hint")}</div>
      </div>
    </div>
  );
}
