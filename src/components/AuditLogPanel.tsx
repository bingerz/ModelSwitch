import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ScrollText } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api } from "../lib/api";

interface AuditEntry {
  timestamp: string;
  action: string;
  actor: string;
  target: string;
  details: string | null;
}

export function AuditLogPanel() {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    try {
      const data = await api.auditLog(200);
      setEntries(data);
    } catch {
      // silently fail
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 15000);
    return () => clearInterval(interval);
  }, [refresh]);

  return (
    <section>
      <SectionHeader title={t("audit.title")} icon={ScrollText} onRefresh={refresh} refreshing={loading} />

      {entries.length === 0 ? (
        <div className="empty-state">
          <div className="empty-state-icon">📋</div>
          <div className="empty-state-title">{t("audit.empty")}</div>
          <div className="empty-state-description">{t("audit.emptyHint")}</div>
        </div>
      ) : (
        <div className="settings-table-wrapper">
          <table className="settings-table">
            <thead>
              <tr>
                <th>{t("audit.timestamp")}</th>
                <th>{t("audit.action")}</th>
                <th>{t("audit.actor")}</th>
                <th>{t("audit.target")}</th>
                <th>{t("audit.details")}</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry, i) => (
                <tr key={i}>
                  <td className="mono" style={{ whiteSpace: "nowrap" }}>
                    {new Date(entry.timestamp).toLocaleString()}
                  </td>
                  <td>
                    <span
                      className="audit-action-badge"
                      style={{
                        padding: "2px 8px",
                        borderRadius: "4px",
                        fontSize: "var(--text-xs)",
                        background:
                          entry.action.includes("delete")
                            ? "var(--color-danger-bg, rgba(239,68,68,0.15))"
                            : entry.action.includes("create")
                            ? "var(--color-success-bg, rgba(34,197,94,0.15))"
                            : "var(--color-bg-secondary)",
                      }}
                    >
                      {entry.action}
                    </span>
                  </td>
                  <td className="mono">{entry.actor}</td>
                  <td className="mono">{entry.target}</td>
                  <td style={{ maxWidth: "300px", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {entry.details ?? "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
