import type { ReactNode } from "react";
import { AlertTriangle } from "lucide-react";

interface ErrorStateProps {
  title: string;
  description?: string;
  action?: ReactNode;
}

export function ErrorState({ title, description, action }: ErrorStateProps) {
  return (
    <div className="ui-error-state" role="alert">
      <div className="ui-error-state-icon" aria-hidden="true">
        <AlertTriangle size={24} />
      </div>
      <div className="ui-error-state-title">{title}</div>
      {description ? (
        <div className="ui-error-state-desc">{description}</div>
      ) : null}
      {action ? <div className="ui-error-state-action">{action}</div> : null}
    </div>
  );
}
