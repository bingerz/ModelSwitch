import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { RotateCcw } from "lucide-react";
import { api, type Channel } from "../../lib/api";
import type { QuotaInfo } from "../../lib/api";
import { STATUS_DOT, STATUS_LABEL_KEY, type ChannelStatus } from "./types";
import {
  API_FORMAT_COLORS,
  API_FORMAT_LABELS,
  QuotaBadge,
  formatRecoveryTime,
  providerBadgeStyle,
  providerToFormat,
} from "./ui";

export interface ChannelCardProps {
  channel: Channel;
  quota: QuotaInfo | undefined;
  confirmDelete: boolean;
  overflowOpen: boolean;
  onEdit: () => void;
  onToggle: () => void;
  onPing: () => void;
  onDelete: () => void;
  onToggleOverflow: () => void;
  onCloseOverflow: () => void;
  onCancelDelete: () => void;
  onDragStart: () => void;
  onDragEnd: () => void;
  selected?: boolean;
  onToggleSelect?: () => void;
  onTest?: () => void;
  onDiagnostics?: () => void;
  onResetCircuit?: () => void | Promise<void>;
}

export function ChannelCard({
  channel: ch,
  quota,
  confirmDelete,
  overflowOpen,
  onEdit,
  onToggle,
  onPing,
  onDelete,
  onToggleOverflow,
  onCloseOverflow,
  onCancelDelete,
  onDragStart,
  onDragEnd,
  selected,
  onToggleSelect,
  onTest,
  onDiagnostics,
  onResetCircuit,
}: ChannelCardProps) {
  const { t } = useTranslation();
  const statusKey = (ch.status as ChannelStatus) ?? "disabled";
  const modelCount = Object.keys(ch.model_mapping).length;
  const circuitOpen = ch.status === "circuit_open" || ch.status === "half_open";

  const [cooldown, setCooldown] = useState<{
    in_cooldown: boolean;
    cooldown_remaining_secs: number;
    circuit_open_until: string | null;
  } | null>(null);

  const [resetting, setResetting] = useState(false);

  useEffect(() => {
    api.channelCooldown(ch.id).then(setCooldown).catch(() => {});
  }, [ch.id]);

  const handleResetCircuit = async () => {
    if (!onResetCircuit || resetting) return;
    setResetting(true);
    try {
      await onResetCircuit();
    } finally {
      setResetting(false);
    }
  };

  return (
    <div
      className={`channel-row ${circuitOpen && ch.status === "circuit_open" ? "channel-row-warning" : ""} ${!ch.enabled ? "channel-row-disabled" : ""}`}
      draggable
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
    >
      <div className="channel-row-main">
        {onToggleSelect && (
          <input
            type="checkbox"
            className="channel-row-check"
            checked={selected ?? false}
            onChange={onToggleSelect}
            onClick={(e) => e.stopPropagation()}
            aria-label={t("common.select")}
          />
        )}

        <span
          className={`status-dot ${ch.status === "healthy" ? "healthy" : ""}`}
          style={{ background: STATUS_DOT[statusKey] }}
          aria-hidden="true"
        />

        <div className="channel-row-identity">
          <div className="channel-row-title">
            <strong>{ch.name}</strong>
            <span
              className="channel-provider channel-provider-badge"
              style={providerBadgeStyle(ch.provider)}
            >
              {ch.provider}
            </span>
            {!ch.enabled && (
              <span className="channel-disabled-tag">{t("common.disabled")}</span>
            )}
          </div>
          <div className="channel-row-status-line">
            <span className="status-label-text" aria-label={t(STATUS_LABEL_KEY[statusKey])}>
              {t(STATUS_LABEL_KEY[statusKey])}
            </span>
            {ch.status === "circuit_open" && (
              <span className="channel-circuit-tag">
                {t("channels.circuitOpen")}
                {ch.circuit_open_until
                  ? ` · ${formatRecoveryTime(ch.circuit_open_until)}`
                  : ""}
              </span>
            )}
            {cooldown?.in_cooldown && (
              <span
                className="channel-circuit-tag channel-circuit-tag-warn"
                title={
                  cooldown.circuit_open_until
                    ? t("channels.circuitOpen")
                    : undefined
                }
              >
                {t("channels.cooldownActive")}
                {" · "}
                {t("channels.cooldownRemaining", {
                  secs: Math.ceil(cooldown.cooldown_remaining_secs),
                })}
              </span>
            )}
          </div>
        </div>

        <div className="channel-row-meta">
          <span
            className="api-format-card-badge"
            style={{ color: API_FORMAT_COLORS[providerToFormat(ch.provider)] }}
          >
            {API_FORMAT_LABELS[providerToFormat(ch.provider)]}
          </span>
          <span className="meta-tag">
            {t("channels.weightShort", { weight: ch.weight })}
          </span>
          {modelCount > 0 && (
            <span className="meta-tag">
              {t("common.modelsCount", { count: modelCount })}
            </span>
          )}
          {ch.avg_latency_ms > 0 && (
            <span className="meta-tag mono">
              {Math.round(ch.avg_latency_ms)}ms
            </span>
          )}
          <QuotaBadge quota={quota} />
          {ch.tags?.map((tag) => (
            <span key={tag} className="meta-tag channel-tag-chip">
              {tag}
            </span>
          ))}
        </div>

        <div className="channel-row-actions">
          <button className="btn btn-sm" onClick={onEdit}>
            {t("common.edit")}
          </button>
          {onResetCircuit && circuitOpen && (
            <button
              className="btn btn-sm"
              onClick={handleResetCircuit}
              disabled={resetting}
              title={t("channels.resetCircuit")}
            >
              <RotateCcw
                size={14}
                className={resetting ? "ui-spin" : ""}
                style={{
                  display: "inline",
                  verticalAlign: "middle",
                  marginRight: "var(--space-1)",
                }}
              />
              {t("channels.resetCircuit")}
            </button>
          )}
          <div className="channel-actions-overflow-wrapper">
            <button
              className="btn btn-sm btn-overflow"
              onClick={onToggleOverflow}
              title={t("common.moreActions")}
              aria-expanded={overflowOpen}
            >
              {"\u22EF"}
            </button>
            {overflowOpen && (
              <>
                <div
                  className="channel-overflow-backdrop"
                  onClick={onCloseOverflow}
                />
                <div className="channel-overflow-menu" role="menu">
                  <button
                    className="channel-overflow-item"
                    role="menuitem"
                    onClick={() => {
                      onCloseOverflow();
                      onToggle();
                    }}
                  >
                    {ch.enabled ? t("common.disable") : t("common.enable")}
                  </button>
                  <button
                    className="channel-overflow-item"
                    role="menuitem"
                    onClick={() => {
                      onCloseOverflow();
                      onPing();
                    }}
                  >
                    {t("channels.ping")}
                  </button>
                  {onTest && (
                    <button
                      className="channel-overflow-item"
                      role="menuitem"
                      onClick={() => {
                        onCloseOverflow();
                        onTest();
                      }}
                    >
                      {t("channels.testConnection")}
                    </button>
                  )}
                  {onDiagnostics && (
                    <button
                      className="channel-overflow-item"
                      role="menuitem"
                      onClick={() => {
                        onCloseOverflow();
                        onDiagnostics();
                      }}
                    >
                      {t("diagnostics.button")}
                    </button>
                  )}
                  <button
                    className={`channel-overflow-item ${confirmDelete ? "danger-confirm" : "danger"}`}
                    role="menuitem"
                    onClick={onDelete}
                    onBlur={onCancelDelete}
                  >
                    {confirmDelete
                      ? t("common.confirmDelete")
                      : t("common.delete")}
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
