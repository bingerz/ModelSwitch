import type { ComponentType } from "react";

interface LoadingStateProps {
  icon?: ComponentType<{ size?: number; className?: string }>;
  message?: string;
}

export function LoadingState({ icon: Icon, message }: LoadingStateProps) {
  return (
    <div className="ui-loading-state">
      <div className="ui-loading-spinner" />
      {Icon && (
        <div className="ui-loading-icon">
          <Icon size={24} />
        </div>
      )}
      {message && <div className="ui-loading-message">{message}</div>}
    </div>
  );
}
