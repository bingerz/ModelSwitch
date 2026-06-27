import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ScrollText } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api } from "../lib/api";

export function AuditLogPanel() {
  const { t } = useTranslation();
  const { data: entries = [], isLoading: loading, refetch } = useQuery({
    queryKey: ["audit-log"],
    queryFn: () => api.auditLog(200),
    refetchInterval: 15_000,
    // Silently fail — keep showing stale data
    retry: false,
  });

  const refresh = async () => {
    await refetch();
  };

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
