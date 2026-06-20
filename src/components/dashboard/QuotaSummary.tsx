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
  if (totalChannels === 0) return null;

  const tone = errors > 0 ? "error" : lowBalance > 0 ? "warn" : "healthy";
  const toneClass = `dsh-quota-card dsh-quota-tone-${tone}`;

  return (
    <div className={toneClass}>
      <div className="dsh-card-title-row">
        <span className="dsh-card-title">Quota</span>
        <span className={`dsh-quota-tone-badge dsh-quota-tone-badge-${tone}`}>
          {tone === "healthy" ? "Healthy" : tone === "warn" ? "Low" : "Errors"}
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
          {channelsWithData}/{totalChannels} channels reporting
        </span>
        {lowBalance > 0 && (
          <span className="dsh-quota-warn">
            <TriangleAlert size={12} /> {lowBalance} low balance
          </span>
        )}
        {errors > 0 && (
          <span className="dsh-quota-error">
            <CircleAlert size={12} /> {errors} errors
          </span>
        )}
      </div>
      <p className="dsh-quota-hint">Detailed quotas on the Provider Quota tab.</p>
    </div>
  );
}
