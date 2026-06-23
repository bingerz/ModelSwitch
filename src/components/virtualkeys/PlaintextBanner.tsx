import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { CreateVirtualKeyResponse } from "../../lib/api";
import { useToast } from "../Toast";

export interface PlaintextBannerProps {
  response: CreateVirtualKeyResponse;
  onClose: () => void;
}

export function PlaintextBanner({
  response,
  onClose,
}: PlaintextBannerProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [copied, setCopied] = useState(false);

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
    <div className="vk-plaintext-banner">
      <div className="vk-plaintext-header">
        <strong>{t("virtualKeys.newKeyCreated", { name: response.key.name })}</strong>
        <button
          className="btn btn-sm"
          onClick={onClose}
          aria-label={t("common.dismiss")}
          title={t("common.dismiss")}
        >
          {t("common.close")}
        </button>
      </div>
      <div className="vk-plaintext-warning">
        {t("virtualKeys.plaintextWarning")}
      </div>
      <div className="vk-plaintext-key-row">
        <code className="vk-plaintext-key mono">{response.plaintext}</code>
        <button
          className={`btn btn-sm ${copied ? "btn-primary" : ""}`}
          onClick={handleCopy}
        >
          {copied ? t("common.copied") : t("common.copy")}
        </button>
      </div>
    </div>
  );
}
