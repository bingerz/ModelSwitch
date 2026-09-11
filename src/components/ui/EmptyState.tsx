import type { ComponentType, ReactNode } from "react";

interface EmptyStateProps {
  icon: ComponentType<{ size?: number; className?: string }>;
  title: string;
  description: string;
  action?: ReactNode;
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
}: EmptyStateProps) {
  return (
    <div className="ui-empty-state">
      <div className="ui-empty-state-icon">
        <Icon size={28} />
      </div>
      <div className="ui-empty-state-title">{title}</div>
      <div className="ui-empty-state-desc">{description}</div>
      {action ? <div className="ui-empty-state-action">{action}</div> : null}
    </div>
  );
}
