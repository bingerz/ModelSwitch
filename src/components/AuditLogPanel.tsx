import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ScrollText } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api } from "../lib/api";

const PAGE_SIZE = 50;

export function AuditLogPanel() {
  const { t } = useTranslation();
  const [page, setPage] = useState(1);

  const { data: entries = [], isLoading: loading, refetch } = useQuery({
    queryKey: ["audit-log"],
    queryFn: () => api.auditLog(500),
    refetchInterval: 30_000,
    retry: false,
  });

  const refresh = async () => {
    await refetch();
  };

  const totalPages = Math.ceil(entries.length / PAGE_SIZE);
  const pageEntries = entries.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);

  // Reset page if entries shrink (e.g. after refresh)
  if (page > totalPages && totalPages > 0) setPage(1);

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
        <>
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
                {pageEntries.map((entry, i) => (
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
          {totalPages > 1 && (
            <div className="vk-pagination" style={{ marginTop: "0.75rem", display: "flex", alignItems: "center", gap: "0.5rem", justifyContent: "flex-end" }}>
              <span style={{ fontSize: "var(--text-xs)", color: "var(--color-text-secondary)" }}>
                {(page - 1) * PAGE_SIZE + 1}–{Math.min(page * PAGE_SIZE, entries.length)} / {entries.length}
              </span>
              <button
                className="btn btn-sm"
                disabled={page <= 1}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                title={t("common.previous")}
              >
                ←
              </button>
              <span style={{ fontSize: "var(--text-xs)" }}>{page} / {totalPages}</span>
              <button
                className="btn btn-sm"
                disabled={page >= totalPages}
                onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                title={t("common.next")}
              >
                →
              </button>
            </div>
          )}
        </>
      )}
    </section>
  );
}
