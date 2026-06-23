import { useState } from "react";
import { useTranslation } from "react-i18next";

export function LoginPage({ onSuccess }: { onSuccess: () => void }) {
  const { t } = useTranslation();
  const [token, setToken] = useState("");
  const [error, setError] = useState("");

  const handleLogin = async () => {
    // Store token temporarily and test against gateway
    localStorage.setItem("admin_token", token);
    try {
      // Test: fetch gateway info — if it succeeds, token is valid
      const { request } = await import("../lib/api");
      await request("/api/gateway/info");
      onSuccess();
    } catch {
      localStorage.removeItem("admin_token");
      setError(t("login.error"));
    }
  };

  return (
    <div className="login-page">
      <div className="login-card">
        <div className="login-brand">{t("login.brand")}</div>
        <div className="login-subtitle">{t("login.subtitle")}</div>
        <input
          className="login-input"
          type="password"
          placeholder={t("login.tokenPlaceholder")}
          value={token}
          onChange={(e) => setToken(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleLogin()}
          autoFocus
        />
        {error && <div className="login-error">{error}</div>}
        <button className="login-button" onClick={handleLogin}>
          {t("login.button")}
        </button>
        <div className="login-hint">
          {t("login.hint")}
        </div>
      </div>
    </div>
  );
}
