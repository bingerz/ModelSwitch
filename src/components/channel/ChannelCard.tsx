import type { Channel } from "../../lib/api";
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
}: ChannelCardProps) {
  const statusKey = (ch.status as ChannelStatus) ?? "disabled";

  return (
    <div
      className={`channel-card ${ch.status === "circuit_open" ? "channel-card-warning" : ""} ${!ch.enabled ? "channel-card-disabled" : ""}`}
      draggable
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
    >
      <div className="channel-card-header">
        <div className="channel-card-title">
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
          <span className="channel-disabled-tag">Disabled</span>
        )}
        {ch.status === "circuit_open" && (
          <span className="channel-circuit-tag">
            Circuit Open
            {ch.circuit_open_until
              ? ` · ${formatRecoveryTime(ch.circuit_open_until)}`
              : ""}
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
        <span className="meta-tag">W:{ch.weight}</span>
        {Object.keys(ch.model_mapping).length > 0 && (
          <span className="meta-tag">
            {Object.keys(ch.model_mapping).length} models
          </span>
        )}
        {ch.avg_latency_ms > 0 && (
          <span className="meta-tag">
            {Math.round(ch.avg_latency_ms)}ms
          </span>
        )}
        <QuotaBadge quota={quota} />
      </div>
      <div className="channel-card-actions">
        <button className="btn btn-sm" onClick={onEdit}>
          Edit
        </button>
        <button className="btn btn-sm" onClick={onToggle}>
          {ch.enabled ? "Disable" : "Enable"}
        </button>
        <div className="channel-actions-overflow-wrapper">
          <button
            className="btn btn-sm btn-overflow"
            onClick={onToggleOverflow}
            title="More actions"
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
                  Ping
                </button>
                <button
                  className={`channel-overflow-item ${confirmDelete ? "danger-confirm" : "danger"}`}
                  onClick={onDelete}
                  onBlur={onCancelDelete}
                >
                  {confirmDelete ? "Confirm Delete?" : "Delete"}
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
