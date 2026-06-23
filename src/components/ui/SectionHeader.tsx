import type { ComponentType, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";

interface SectionHeaderProps {
  title: string;
  icon?: ComponentType<{ size?: number; className?: string }>;
  onRefresh?: () => void;
  refreshing?: boolean;
  action?: ReactNode;
}

export function SectionHeader({
  title,
  icon: Icon,
  onRefresh,
  refreshing,
  action,
}: SectionHeaderProps) {
  const { t } = useTranslation();
  return (
    <div className="ui-section-header">
      <div className="ui-section-title-group">
        {Icon && <Icon size={18} className="ui-section-icon" />}
        <h2 className="ui-section-title">{title}</h2>
      </div>
      <div className="ui-section-actions">
        {action}
        {onRefresh && (
          <button
            className="ui-refresh-btn"
            onClick={onRefresh}
            disabled={refreshing}
          >
            <RefreshCw size={14} className={refreshing ? "ui-spin" : ""} />
            <span>{t("common.refresh")}</span>
          </button>
        )}
      </div>
    </div>
  );
}
