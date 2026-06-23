import { useState } from "react";

export function LoginPage({ onSuccess }: { onSuccess: () => void }) {
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
      setError("Invalid token or gateway not reachable");
    }
  };

  return (
    <div className="login-page">
      <div className="login-card">
        <div className="login-brand">ModelSwitch</div>
        <div className="login-subtitle">Web Console</div>
        <input
          className="login-input"
          type="password"
          placeholder="Admin Token"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleLogin()}
          autoFocus
        />
        {error && <div className="login-error">{error}</div>}
        <button className="login-button" onClick={handleLogin}>
          Login
        </button>
        <div className="login-hint">
          Enter the admin token from your gateway config.
          Leave empty if no token is configured.
        </div>
      </div>
    </div>
  );
}
