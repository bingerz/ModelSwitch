import { useTranslation } from "react-i18next";
import { DollarSign, CircleAlert, TriangleAlert } from "./icons";
import { formatCost } from "./helpers";

export interface QuotaSummaryProps {
  totalBalance: number;
  channelsWithData: number;
  totalChannels: number;
  lowBalance: number;
  errors: number;
}

export function QuotaSummary({
  totalBalance,
  channelsWithData,
  totalChannels,
  lowBalance,
  errors,
}: QuotaSummaryProps) {
  const { t } = useTranslation();
  if (totalChannels === 0) return null;

  const tone = errors > 0 ? "error" : lowBalance > 0 ? "warn" : "healthy";
  const toneClass = `dsh-quota-card dsh-quota-tone-${tone}`;

  return (
    <div className={toneClass}>
      <div className="dsh-card-title-row">
        <span className="dsh-card-title">{t("dashboard.quotaTitle")}</span>
        <span className={`dsh-quota-tone-badge dsh-quota-tone-badge-${tone}`}>
          {tone === "healthy" ? t("dashboard.quotaHealthy") : tone === "warn" ? t("dashboard.quotaLow") : t("dashboard.quotaErrors")}
        </span>
      </div>
      <div className="dsh-quota-balance">
        <span className="dsh-quota-balance-icon">
          <DollarSign size={20} />
        </span>
        <span className="dsh-quota-balance-value">
          {formatCost(totalBalance)}
        </span>
      </div>
      <div className="dsh-quota-secondary">
        <span className="dsh-quota-secondary-item">
          {t("quota.channelsReporting", { withData: channelsWithData, total: totalChannels })}
        </span>
        {lowBalance > 0 && (
          <span className="dsh-quota-warn">
            <TriangleAlert size={12} /> {t("quota.lowBalance", { count: lowBalance })}
          </span>
        )}
        {errors > 0 && (
          <span className="dsh-quota-error">
            <CircleAlert size={12} /> {t("quota.errors", { count: errors })}
          </span>
        )}
      </div>
      <p className="dsh-quota-hint">{t("dashboard.quotaHint")}</p>
    </div>
  );
}
