import { useTranslation } from "react-i18next";
import type { McpServer, McpToolDetail } from "./types";
import {
  STATUS_COLOR,
  argsToText,
  statusErrorMessage,
  statusIsRunning,
  statusKey,
  statusToolCount,
  truncate,
} from "./types";

export interface McpHealthEntry {
  name: string;
  healthy: boolean;
  last_check: string | null;
  last_error: string | null;
  consecutive_failures: number;
}

export interface McpServerCardProps {
  server: McpServer;
  health?: McpHealthEntry;
  expanded: boolean;
  tools: McpToolDetail[] | undefined;
  confirmDelete: boolean;
  actionLoading: boolean;
  onStart: () => void;
  onStop: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onToggleExpand: () => void;
}

export function McpServerCard({
  server,
  health,
  expanded,
  tools,
  confirmDelete,
  actionLoading,
  onStart,
  onStop,
  onEdit,
  onDelete,
  onToggleExpand,
}: McpServerCardProps) {
  const { t } = useTranslation();
  const sKey = statusKey(server.status);
  const isRunning = statusIsRunning(server.status);
  const errMsg = statusErrorMessage(server.status);
  const statusText = isRunning
    ? t("mcp.runningTools", { count: statusToolCount(server.status) })
    : errMsg
      ? t("mcp.errorLabel", { message: truncate(errMsg, 50) })
      : t(sKey === "error" ? "mcp.error" : "mcp.stopped");

  const healthLabel = health
    ? health.healthy
      ? t("mcp.healthHealthy")
      : t("mcp.healthUnhealthy")
    : t("mcp.healthUnknown");
  const healthColor = !health
    ? "var(--color-text-muted)"
    : health.healthy
      ? "var(--color-success)"
      : "var(--color-danger)";
  const healthTitle = health
    ? health.healthy
      ? t("mcp.healthHealthy")
      : `${t("mcp.healthUnhealthy")} · ${t("mcp.healthFailures", { count: health.consecutive_failures })}`
    : t("mcp.healthUnknown");

  return (
    <div className="mcp-server-card">
      <div className="mcp-server-header">
        <div className="mcp-server-title">
          <span
            className={`status-dot ${isRunning ? "healthy" : ""}`}
            style={{ background: STATUS_COLOR[sKey] }}
          />
          <strong>{server.name}</strong>
          <span className="meta-tag mono">{server.id}</span>
          {!server.enabled && <span className="meta-tag">{t("common.disabled")}</span>}
          {!server.expose_tools && <span className="meta-tag">{t("mcp.hiddenTools")}</span>}
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
          <span
            className="meta-tag"
            title={healthTitle}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: "4px",
              color: healthColor,
              cursor: "help",
            }}
          >
            <span
              style={{
                width: 7,
                height: 7,
                borderRadius: "50%",
                display: "inline-block",
                background: healthColor,
              }}
            />
            {t("mcp.healthStatus")}: {healthLabel}
          </span>
          <span className={`mcp-status-badge mcp-status-${sKey}`}>
            {statusText}
          </span>
        </div>
      </div>

      <div className="mcp-server-meta">
        <span className="meta-tag mono">{server.command}</span>
        {server.args.length > 0 && (
          <span className="meta-tag mono">{argsToText(server.args)}</span>
        )}
        {Object.keys(server.env).length > 0 && (
          <span className="meta-tag">
            {t("common.envVars", { count: Object.keys(server.env).length })}
          </span>
        )}
      </div>

      {errMsg && (
        <div className="mcp-error-message mono">{errMsg}</div>
      )}

      <div className="mcp-server-actions">
        {isRunning ? (
          <button
            className="btn btn-sm"
            onClick={onStop}
            disabled={actionLoading}
          >
            {actionLoading ? t("mcp.stopping") : t("mcp.stop")}
          </button>
        ) : (
          <button
            className="btn btn-sm"
            onClick={onStart}
            disabled={actionLoading || !server.enabled}
            title={!server.enabled ? t("mcp.enableFirst") : undefined}
          >
            {actionLoading ? t("mcp.starting") : t("mcp.start")}
          </button>
        )}
        <button
          className="btn btn-sm"
          onClick={onToggleExpand}
          disabled={!isRunning}
          title={!isRunning ? t("mcp.startToViewTools") : undefined}
        >
          {expanded ? t("mcp.hideTools") : t("mcp.tools")}
        </button>
        <button className="btn btn-sm" onClick={onEdit}>
          {t("common.edit")}
        </button>
        <button
          className={`btn btn-sm ${confirmDelete ? "btn-danger" : ""}`}
          style={confirmDelete ? undefined : { color: "var(--color-danger)" }}
          onClick={onDelete}
          onBlur={() => {
            /* parent handles confirm state */
          }}
          disabled={actionLoading}
        >
          {confirmDelete ? t("common.confirmQuestion") : t("common.delete")}
        </button>
      </div>

      {expanded && tools !== undefined && (
        <div className="mcp-tools-panel">
          <div className="mcp-tools-title">
            {t("mcp.toolsCount", { count: tools.length })}
          </div>
          {tools.length === 0 ? (
            <div className="mcp-tools-empty">{t("mcp.noTools")}</div>
          ) : (
            <div className="mcp-tools-list">
              {tools.map((tool) => (
                <div key={tool.name} className="mcp-tool-item">
                  <span className="mcp-tool-name mono">{tool.name}</span>
                  {tool.description && (
                    <span className="mcp-tool-desc">{truncate(tool.description, 100)}</span>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
