import { useTranslation } from "react-i18next";

/** Placeholder card for channels that don't have any quota entry from the backend */
export function PendingCard({ name, provider }: { name: string; provider: string }) {
  const { t } = useTranslation();
  return (
    <div className="quota-card quota-card-pending">
      <div className="quota-card-header">
        <div className="quota-card-title">
          <strong>{name}</strong>
          <span className="quota-source-badge" style={{ color: "var(--color-text-muted)" }}>
            {t("quota.pending")}
          </span>
        </div>
        <span className="quota-provider">{provider}</span>
      </div>
      <div className="quota-no-data">{t("quota.waitingForData")}</div>
    </div>
  );
}
