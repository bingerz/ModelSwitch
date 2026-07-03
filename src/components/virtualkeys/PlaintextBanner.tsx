import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CreateVirtualKeyResponse } from "../../lib/api";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { useToast } from "../Toast";

export interface PlaintextBannerProps {
  response: CreateVirtualKeyResponse;
  onClose: () => void;
}

// ponytail: Q8 — modal (not toast/banner). No backdrop/escape close;
// user MUST click "I've saved it" so the key cannot be lost to a stray click.
export function PlaintextBanner({
  response,
  onClose,
}: PlaintextBannerProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const dialogRef = useRef<HTMLDivElement>(null);
  const [copied, setCopied] = useState(false);

  // No onEscape: Escape must not dismiss — key would be permanently lost.
  useFocusTrap(dialogRef, true);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(response.plaintext);
      setCopied(true);
      toast.success(t("common.copiedToClipboard"));
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error(t("common.failed"));
    }
  };

  return (
    <div
      ref={dialogRef}
      className="vk-batch-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="vk-plaintext-title"
      // Intentionally no backdrop-click close: the key is shown once.
      onClick={(e) => e.stopPropagation()}
    >
      <div className="vk-batch-modal">
        <div className="vk-batch-modal-header">
          <h3 id="vk-plaintext-title" className="form-title">
            {t("virtualKeys.newKeyCreated", { name: response.key.name })}
          </h3>
        </div>

        <div className="vk-plaintext-warning">
          {t("virtualKeys.plaintextWarning")}
        </div>

        <div className="vk-plaintext-key-row">
          <input
            className="vk-plaintext-key mono"
            readOnly
            value={response.plaintext}
            aria-label={t("virtualKeys.newKeyCreated", { name: response.key.name })}
            onFocus={(e) => e.currentTarget.select()}
          />
          <button
            type="button"
            className={`btn btn-sm ${copied ? "btn-primary" : ""}`}
            onClick={handleCopy}
          >
            {copied ? t("common.copied") : t("common.copy")}
          </button>
        </div>

        <div style={{ display: "flex", gap: "var(--space-2)", marginTop: "var(--space-3)" }}>
          <button
            type="button"
            className="btn btn-primary"
            onClick={onClose}
          >
            {t("virtualKeys.iHaveSavedIt")}
          </button>
        </div>
      </div>
    </div>
  );
}
