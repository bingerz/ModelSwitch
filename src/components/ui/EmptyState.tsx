import type { ComponentType } from "react";

interface EmptyStateProps {
  icon: ComponentType<{ size?: number; className?: string }>;
  title: string;
  description: string;
}

export function EmptyState({ icon: Icon, title, description }: EmptyStateProps) {
  return (
    <div className="ui-empty-state">
      <div className="ui-empty-state-icon">
        <Icon size={32} />
      </div>
      <div className="ui-empty-state-title">{title}</div>
      <div className="ui-empty-state-desc">{description}</div>
    </div>
  );
}
