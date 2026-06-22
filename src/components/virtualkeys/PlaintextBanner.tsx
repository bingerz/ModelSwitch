import { useState } from "react";
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
  const toast = useToast();
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(response.plaintext);
      setCopied(true);
      toast.success("Copied to clipboard");
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  };

  return (
    <div className="vk-plaintext-banner">
      <div className="vk-plaintext-header">
        <strong>New key created: {response.key.name}</strong>
        <button
          className="btn btn-sm"
          onClick={onClose}
          aria-label="Dismiss"
          title="Dismiss"
        >
          Close
        </button>
      </div>
      <div className="vk-plaintext-warning">
        This key will not be shown again. Copy it now and store it securely.
      </div>
      <div className="vk-plaintext-key-row">
        <code className="vk-plaintext-key mono">{response.plaintext}</code>
        <button
          className={`btn btn-sm ${copied ? "btn-primary" : ""}`}
          onClick={handleCopy}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
    </div>
  );
}
