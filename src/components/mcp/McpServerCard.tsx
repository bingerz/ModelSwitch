import type { McpServer, McpToolDetail } from "./types";
import {
  STATUS_COLOR,
  argsToText,
  statusErrorMessage,
  statusIsRunning,
  statusKey,
  statusLabel,
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
  const sKey = statusKey(server.status);
  const isRunning = statusIsRunning(server.status);
  const errMsg = statusErrorMessage(server.status);

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
          {!server.enabled && <span className="meta-tag">disabled</span>}
          {!server.expose_tools && <span className="meta-tag">hidden tools</span>}
        </div>
        <span className={`mcp-status-badge mcp-status-${sKey}`}>
          {statusLabel(server.status)}
        </span>
      </div>

      <div className="mcp-server-meta">
        <span className="meta-tag mono">{server.command}</span>
        {server.args.length > 0 && (
          <span className="meta-tag mono">{argsToText(server.args)}</span>
        )}
        {Object.keys(server.env).length > 0 && (
          <span className="meta-tag">{Object.keys(server.env).length} env vars</span>
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
            {actionLoading ? "Stopping..." : "Stop"}
          </button>
        ) : (
          <button
            className="btn btn-sm"
            onClick={onStart}
            disabled={actionLoading || !server.enabled}
            title={!server.enabled ? "Enable the server first" : undefined}
          >
            {actionLoading ? "Starting..." : "Start"}
          </button>
        )}
        <button
          className="btn btn-sm"
          onClick={onToggleExpand}
          disabled={!isRunning}
          title={!isRunning ? "Start the server to view tools" : undefined}
        >
          {expanded ? "Hide Tools" : "Tools"}
        </button>
        <button className="btn btn-sm" onClick={onEdit}>
          Edit
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
          {confirmDelete ? "Confirm?" : "Delete"}
        </button>
      </div>

      {expanded && tools !== undefined && (
        <div className="mcp-tools-panel">
          <div className="mcp-tools-title">
            Tools ({tools.length})
          </div>
          {tools.length === 0 ? (
            <div className="mcp-tools-empty">No tools available</div>
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
