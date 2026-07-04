import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CheckCircle2,
  XCircle,
  AlertCircle,
  Clock,
  Server,
  Loader2,
} from "lucide-react";
import { api, type ChannelDiagnosticsResult } from "../../lib/api";
import { useFocusTrap } from "../../hooks/useFocusTrap";

interface DiagnosticsModalProps {
  channelId: string;
  channelName: string;
  onClose: () => void;
}

export function DiagnosticsModal({
  channelId,
  channelName,
  onClose,
}: DiagnosticsModalProps) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  const [result, setResult] = useState<ChannelDiagnosticsResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(dialogRef, true, onClose);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setLoading(true);
      setError(null);
      try {
        const res = await api.channelDiagnostics(channelId);
        if (!cancelled) setResult(res);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [channelId]);

  // Auth status rendering
  const authBadge = (status: string) => {
    if (status === "authenticated") {
      return (
        <span className="diag-badge diag-badge-ok">
          <CheckCircle2 size={14} /> {t("diagnostics.authOk")}
        </span>
      );
    }
    if (status === "unauthenticated") {
      return (
        <span className="diag-badge diag-badge-warn">
          <XCircle size={14} /> {t("diagnostics.authFailed")}
        </span>
      );
    }
    return (
      <span className="diag-badge diag-badge-err">
        <AlertCircle size={14} /> {t("diagnostics.authError")}
      </span>
    );
  };

  return (
    <div className="diag-modal-backdrop" onClick={onClose}>
      <div
        ref={dialogRef}
        className="diag-modal"
        role="dialog"
        aria-modal="true"
        aria-label={t("diagnostics.title", { name: channelName })}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="diag-modal-header">
          <h3>{t("diagnostics.title", { name: channelName })}</h3>
          <button
            className="diag-modal-close"
            onClick={onClose}
            aria-label="Close"
          >
            {"\u2715"}
          </button>
        </div>
        <div className="diag-modal-body">
          {loading && (
            <div className="diag-loading">
              <Loader2 size={24} className="ui-spin" />
              <span>{t("diagnostics.running")}</span>
            </div>
          )}
          {error && !loading && (
            <div className="diag-error-card">
              <AlertCircle size={16} />
              <span>{error}</span>
            </div>
          )}
          {result && !loading && (
            <>
              {/* Status row */}
              <div className="diag-status-row">
                {authBadge(result.auth_status)}
                {result.status_code && (
                  <span className="diag-meta">
                    <Server size={14} /> HTTP {result.status_code}
                  </span>
                )}
                <span className="diag-meta">
                  <Clock size={14} /> {result.latency_ms}ms
                </span>
              </div>

              {/* Error message */}
              {result.error && (
                <div className="diag-error-detail">{result.error}</div>
              )}

              {/* Available models */}
              <div className="diag-models-section">
                <h4 className="diag-section-title">
                  {t("diagnostics.availableModels", {
                    count: result.available_models.length,
                  })}
                </h4>
                {result.available_models.length > 0 ? (
                  <div className="diag-models-grid">
                    {result.available_models.map((model) => (
                      <span key={model} className="diag-model-tag">
                        {model}
                      </span>
                    ))}
                  </div>
                ) : (
                  <p className="diag-no-models">{t("diagnostics.noModels")}</p>
                )}
              </div>

              {/* Timestamp */}
              <div className="diag-timestamp">
                {t("diagnostics.testedAt", {
                  time: new Date(result.tested_at).toLocaleString(),
                })}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
