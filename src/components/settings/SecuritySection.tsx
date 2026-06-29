import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Shield,
  Key,
  Users,
  Lock,
  CheckCircle2,
  XCircle,
} from "lucide-react";
import { api, type AuthStatus } from "../../lib/api";
import { useToast } from "../Toast";
import { SectionHeader } from "../ui/SectionHeader";

const BADGE_STYLE: Record<"red" | "blue" | "gray", React.CSSProperties> = {
  red: {
    background: "rgba(220, 38, 38, 0.12)",
    color: "#dc2626",
    border: "1px solid rgba(220, 38, 38, 0.3)",
  },
  blue: {
    background: "rgba(37, 99, 235, 0.12)",
    color: "#2563eb",
    border: "1px solid rgba(37, 99, 235, 0.3)",
  },
  gray: {
    background: "rgba(100, 116, 139, 0.12)",
    color: "#64748b",
    border: "1px solid rgba(100, 116, 139, 0.3)",
  },
};

const BADGE_BASE: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: "var(--space-1)",
  fontSize: "var(--text-xs)",
  fontWeight: 600,
  padding: "2px var(--space-2)",
  borderRadius: "var(--radius-sm)",
  whiteSpace: "nowrap",
};

const SUCCESS_COLOR = "#16a34a";
const MUTED_COLOR = "var(--color-text-muted)";

/** Security & access control overview (admin token, RBAC, LDAP, OIDC). */
export function SecuritySection() {
  const { t } = useTranslation();
  const toast = useToast();
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = await api.authStatus();
        if (!cancelled) setStatus(data);
      } catch {
        if (!cancelled) toast.error(t("common.error"));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [t, toast]);

  if (loading) {
    return <div className="panel-loading">{t("common.loading")}</div>;
  }
  if (!status) return null;

  return (
    <section>
      <SectionHeader title={t("security.title")} icon={Shield} />

      <div className="settings-section">
        <div className="settings-stats-grid">
          {/* Card 1: Admin Token */}
          <div className="settings-stat">
            <span className="settings-stat-label">
              <Key size={14} className="icon-inline" />
              {t("security.adminToken")}
            </span>
            <span
              className="settings-stat-value"
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--space-1)",
              }}
            >
              {status.admin_token_set ? (
                <>
                  <CheckCircle2
                    size={16}
                    style={{ color: SUCCESS_COLOR, flexShrink: 0 }}
                  />
                  <span
                    style={{
                      fontSize: "var(--text-sm)",
                      fontWeight: 500,
                      color: SUCCESS_COLOR,
                    }}
                  >
                    {t("security.adminTokenSet")}
                  </span>
                </>
              ) : (
                <>
                  <XCircle
                    size={16}
                    style={{ color: MUTED_COLOR, flexShrink: 0 }}
                  />
                  <span
                    style={{
                      fontSize: "var(--text-sm)",
                      fontWeight: 500,
                      color: MUTED_COLOR,
                    }}
                  >
                    {t("security.adminTokenNotSet")}
                  </span>
                </>
              )}
            </span>
          </div>

          {/* Card 2: RBAC Roles */}
          <div className="settings-stat">
            <span className="settings-stat-label">
              <Users size={14} className="icon-inline" />
              {t("security.rbacRoles")}
            </span>
            <span className="settings-stat-value">
              {status.rbac.enabled ? (
                <div
                  style={{
                    display: "flex",
                    flexWrap: "wrap",
                    gap: "var(--space-1)",
                    marginTop: "2px",
                  }}
                >
                  <span style={{ ...BADGE_BASE, ...BADGE_STYLE.red }}>
                    {t("security.superAdmin")}: {status.rbac.super_admin_count}
                  </span>
                  <span style={{ ...BADGE_BASE, ...BADGE_STYLE.blue }}>
                    {t("security.keyManager")}: {status.rbac.key_manager_count}
                  </span>
                  <span style={{ ...BADGE_BASE, ...BADGE_STYLE.gray }}>
                    {t("security.auditor")}: {status.rbac.auditor_count}
                  </span>
                </div>
              ) : (
                <span
                  style={{
                    fontSize: "var(--text-sm)",
                    fontWeight: 500,
                    color: MUTED_COLOR,
                  }}
                >
                  {t("security.rbacDisabled")}
                </span>
              )}
            </span>
          </div>

          {/* Card 3: LDAP / Active Directory */}
          <div className="settings-stat">
            <span className="settings-stat-label">
              <Lock size={14} className="icon-inline" />
              {t("security.ldap")}
            </span>
            <span className="settings-stat-value">
              {status.ldap ? (
                <div
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: "2px",
                    fontWeight: 500,
                  }}
                >
                  <span
                    className="mono"
                    style={{
                      fontSize: "var(--text-xs)",
                      wordBreak: "break-all",
                    }}
                  >
                    {status.ldap.url}
                  </span>
                  <span style={{ fontSize: "var(--text-xs)" }}>
                    {t("security.starttls")}:{" "}
                    {status.ldap.starttls
                      ? t("security.enabled")
                      : t("security.disabled")}
                  </span>
                  <span style={{ fontSize: "var(--text-xs)" }}>
                    {t("security.defaultGroup")}: {status.ldap.default_group}
                  </span>
                </div>
              ) : (
                <span
                  style={{
                    fontSize: "var(--text-sm)",
                    fontWeight: 500,
                    color: MUTED_COLOR,
                  }}
                >
                  {t("security.ldapDisabled")}
                </span>
              )}
            </span>
          </div>

          {/* Card 4: OIDC Single Sign-On */}
          <div className="settings-stat">
            <span className="settings-stat-label">
              <Shield size={14} className="icon-inline" />
              {t("security.oidc")}
            </span>
            <span className="settings-stat-value">
              {status.oidc ? (
                <div
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: "2px",
                    fontWeight: 500,
                  }}
                >
                  <span
                    className="mono"
                    style={{
                      fontSize: "var(--text-xs)",
                      wordBreak: "break-all",
                    }}
                  >
                    {status.oidc.issuer}
                  </span>
                  <span style={{ fontSize: "var(--text-xs)" }}>
                    {t("security.clientId")}: {status.oidc.client_id}
                  </span>
                  <span style={{ fontSize: "var(--text-xs)" }}>
                    {t("security.redirectUri")}: {status.oidc.redirect_uri}
                  </span>
                  <span style={{ fontSize: "var(--text-xs)" }}>
                    {t("security.scopes")}: {status.oidc.scopes.join(", ")}
                  </span>
                </div>
              ) : (
                <span
                  style={{
                    fontSize: "var(--text-sm)",
                    fontWeight: 500,
                    color: MUTED_COLOR,
                  }}
                >
                  {t("security.oidcDisabled")}
                </span>
              )}
            </span>
          </div>
        </div>
      </div>
    </section>
  );
}
