import type { ComponentType } from "react";
import { TriangleAlert } from "lucide-react";

interface ErrorStateProps {
  icon?: ComponentType<{ size?: number; className?: string }>;
  title: string;
  message: string;
  onRetry?: () => void;
  retryLabel?: string;
}

export function ErrorState({
  icon: Icon = TriangleAlert,
  title,
  message,
  onRetry,
  retryLabel = "Retry",
}: ErrorStateProps) {
  return (
    <div className="ui-error-state">
      <div className="ui-error-icon">
        <Icon size={32} />
      </div>
      <div className="ui-error-title">{title}</div>
      <div className="ui-error-message">{message}</div>
      {onRetry && (
        <button className="btn btn-primary ui-error-retry" onClick={onRetry}>
          {retryLabel}
        </button>
      )}
    </div>
  );
}
