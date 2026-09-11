import type { ComponentType, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";

interface PanelLayoutProps {
  title: string;
  icon?: ComponentType<{ size?: number; className?: string }>;
  children: ReactNode;
  actions?: ReactNode;
  toolbar?: ReactNode;
  onRefresh?: () => void;
  refreshing?: boolean;
}

export function PanelLayout({
  title,
  icon: Icon,
  children,
  actions,
  toolbar,
  onRefresh,
  refreshing,
}: PanelLayoutProps) {
  const { t } = useTranslation();

  return (
    <section className="panel-layout">
      <div className="panel-layout-header">
        <div className="panel-layout-title-group">
          {Icon && <Icon size={20} className="panel-layout-icon" />}
          <h2 className="panel-layout-title">{title}</h2>
        </div>
        <div className="panel-layout-actions">
          {actions}
          {onRefresh && (
            <button
              className="btn btn-sm panel-layout-refresh"
              onClick={onRefresh}
              disabled={refreshing}
              title={t("common.refreshNow")}
            >
              <RefreshCw size={14} className={refreshing ? "ui-spin" : ""} />
              <span>{t("common.refresh")}</span>
            </button>
          )}
        </div>
      </div>
      {toolbar && <div className="panel-layout-toolbar">{toolbar}</div>}
      <div className="panel-layout-content">{children}</div>
    </section>
  );
}
