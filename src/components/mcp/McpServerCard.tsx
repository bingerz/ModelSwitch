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

export interface McpServerCardProps {
  server: McpServer;
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
        <span className={`mcp-status-badge mcp-status-${sKey}`}>
          {statusText}
        </span>
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
