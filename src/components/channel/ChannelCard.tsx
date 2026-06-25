import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, type Channel } from "../../lib/api";
import type { QuotaInfo } from "../../lib/api";
import { STATUS_DOT, type ChannelStatus } from "./types";
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
}: ChannelCardProps) {
  const { t } = useTranslation();
  const statusKey = (ch.status as ChannelStatus) ?? "disabled";
  const modelCount = Object.keys(ch.model_mapping).length;

  const [cooldown, setCooldown] = useState<{
    in_cooldown: boolean;
    cooldown_remaining_secs: number;
    circuit_open_until: string | null;
  } | null>(null);

  useEffect(() => {
    api.channelCooldown(ch.id).then(setCooldown).catch(() => {});
  }, [ch.id]);

  return (
    <div
      className={`channel-card ${ch.status === "circuit_open" ? "channel-card-warning" : ""} ${!ch.enabled ? "channel-card-disabled" : ""}`}
      draggable
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
    >
      <div className="channel-card-header">
        <div className="channel-card-title">
          {onToggleSelect && (
            <input
              type="checkbox"
              checked={selected ?? false}
              onChange={onToggleSelect}
              onClick={(e) => e.stopPropagation()}
              style={{ width: 16, height: 16, cursor: "pointer", marginRight: "var(--space-1)" }}
            />
          )}
          <span
            className={`status-dot ${ch.status === "healthy" ? "healthy" : ""}`}
            style={{ background: STATUS_DOT[statusKey] }}
          />
          <strong>{ch.name}</strong>
        </div>
        <span
          className="channel-provider channel-provider-badge"
          style={providerBadgeStyle(ch.provider)}
        >
          {ch.provider}
        </span>
        {!ch.enabled && (
          <span className="channel-disabled-tag">{t("common.disabled")}</span>
        )}
        {ch.status === "circuit_open" && (
          <span className="channel-circuit-tag">
            {t("channels.circuitOpen")}
            {ch.circuit_open_until
              ? ` \u00B7 ${formatRecoveryTime(ch.circuit_open_until)}`
              : ""}
          </span>
        )}
        {cooldown?.in_cooldown && (
          <span
            className="channel-circuit-tag"
            style={{ background: "var(--color-warning)", color: "var(--color-text-primary)" }}
            title={
              cooldown.circuit_open_until
                ? t("channels.circuitOpen")
                : undefined
            }
          >
            {t("channels.cooldownActive")}
            {" \u00B7 "}
            {t("channels.cooldownRemaining", {
              secs: Math.ceil(cooldown.cooldown_remaining_secs),
            })}
          </span>
        )}
      </div>
      <div className="channel-card-meta">
        <span
          className="api-format-card-badge"
          style={{ color: API_FORMAT_COLORS[providerToFormat(ch.provider)] }}
        >
          {API_FORMAT_LABELS[providerToFormat(ch.provider)]}
        </span>
        <span className="meta-tag">{t("channels.weightShort", { weight: ch.weight })}</span>
        {modelCount > 0 && (
          <span className="meta-tag">
            {t("common.modelsCount", { count: modelCount })}
          </span>
        )}
        {ch.avg_latency_ms > 0 && (
          <span className="meta-tag">
            {Math.round(ch.avg_latency_ms)}ms
          </span>
        )}
        <QuotaBadge quota={quota} />
      </div>
      {ch.tags && ch.tags.length > 0 && (
        <div className="channel-card-tags" style={{ display: "flex", gap: "var(--space-1)", flexWrap: "wrap", marginBottom: "var(--space-1)" }}>
          {ch.tags.map((tag) => (
            <span
              key={tag}
              className="meta-tag"
              style={{ fontSize: "var(--text-xs)", background: "var(--color-bg-secondary)" }}
            >
              {tag}
            </span>
          ))}
        </div>
      )}
      <div className="channel-card-actions">
        <button className="btn btn-sm" onClick={onEdit}>
          {t("common.edit")}
        </button>
        <button className="btn btn-sm" onClick={onToggle}>
          {ch.enabled ? t("common.disable") : t("common.enable")}
        </button>
        <div className="channel-actions-overflow-wrapper">
          <button
            className="btn btn-sm btn-overflow"
            onClick={onToggleOverflow}
            title={t("common.moreActions")}
          >
            {"\u22EF"}
          </button>
          {overflowOpen && (
            <>
              <div
                className="channel-overflow-backdrop"
                onClick={onCloseOverflow}
              />
              <div className="channel-overflow-menu">
                <button
                  className="channel-overflow-item"
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
                    onClick={() => {
                      onCloseOverflow();
                      onTest();
                    }}
                  >
                    {t("channels.testAll")}
                  </button>
                )}
                <button
                  className={`channel-overflow-item ${confirmDelete ? "danger-confirm" : "danger"}`}
                  onClick={onDelete}
                  onBlur={onCancelDelete}
                >
                  {confirmDelete ? t("common.confirmDelete") : t("common.delete")}
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
