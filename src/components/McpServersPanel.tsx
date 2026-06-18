import { useCallback, useEffect, useState } from "react";
import {
  api,
  type McpServer,
  type McpServerStatus,
  type McpToolDetail,
  type CreateMcpServerData,
  type UpdateMcpServerData,
} from "../lib/api";
import { useToast } from "./Toast";
import "../styles/pages-enhanced.css";

// ─── Status Helpers ─────────────────────────────────────

function statusIsRunning(status: McpServerStatus): boolean {
  return typeof status === "object" && status !== null && "running" in status;
}

function statusIsError(status: McpServerStatus): boolean {
  return typeof status === "object" && status !== null && "error" in status;
}

function statusToolCount(status: McpServerStatus): number {
  if (statusIsRunning(status)) {
    return (status as { running: { tool_count: number } }).running.tool_count;
  }
  return 0;
}

function statusErrorMessage(status: McpServerStatus): string | null {
  if (statusIsError(status)) {
    return (status as { error: { message: string } }).error.message;
  }
  return null;
}

const STATUS_COLOR: Record<string, string> = {
  stopped: "var(--color-text-muted)",
  running: "var(--color-success)",
  error: "var(--color-danger)",
};

function statusKey(status: McpServerStatus): string {
  if (status === "stopped") return "stopped";
  if (statusIsRunning(status)) return "running";
  if (statusIsError(status)) return "error";
  return "stopped";
}

function statusLabel(status: McpServerStatus): string {
  if (status === "stopped") return "Stopped";
  if (statusIsRunning(status)) return `Running (${statusToolCount(status)} tools)`;
  const errMsg = statusErrorMessage(status);
  return errMsg ? `Error: ${truncate(errMsg, 50)}` : "Error";
}

function truncate(s: string, max: number): string {
  return s.length > max ? s.slice(0, max) + "..." : s;
}

// ─── Env Helpers ────────────────────────────────────────

function parseEnvText(text: string): Record<string, string> {
  const env: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eqIdx = trimmed.indexOf("=");
    if (eqIdx === -1) continue;
    const key = trimmed.slice(0, eqIdx).trim();
    const value = trimmed.slice(eqIdx + 1).trim();
    if (key) env[key] = value;
  }
  return env;
}

function envToText(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}

function parseArgsText(text: string): string[] {
  return text
    .trim()
    .split(/\s+/)
    .filter((s) => s.length > 0);
}

function argsToText(args: string[]): string {
  return args.join(" ");
}

// ─── Main Panel ─────────────────────────────────────────

export function McpServersPanel() {
  const toast = useToast();
  const [servers, setServers] = useState<McpServer[]>([]);
  const [loading, setLoading] = useState(true);
  const [showAddForm, setShowAddForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [toolsCache, setToolsCache] = useState<Record<string, McpToolDetail[]>>({});
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await api.mcp.listServers();
      setServers(list);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to load MCP servers");
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleStart = async (id: string) => {
    setActionLoading(id);
    try {
      await api.mcp.startServer(id);
      toast.success("Server started");
      await refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to start server");
    } finally {
      setActionLoading(null);
    }
  };

  const handleStop = async (id: string) => {
    setActionLoading(id);
    try {
      await api.mcp.stopServer(id);
      toast.success("Server stopped");
      await refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to stop server");
    } finally {
      setActionLoading(null);
    }
  };

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) {
      setConfirmDeleteId(id);
      return;
    }
    setConfirmDeleteId(null);
    setActionLoading(id);
    try {
      await api.mcp.deleteServer(id);
      toast.success("Server deleted");
      await refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to delete server");
    } finally {
      setActionLoading(null);
    }
  };

  const handleExpandTools = async (server: McpServer) => {
    const id = server.id;
    if (expandedId === id) {
      setExpandedId(null);
      return;
    }
    setExpandedId(id);
    if (!statusIsRunning(server.status)) {
      setToolsCache((prev) => ({ ...prev, [id]: [] }));
      return;
    }
    try {
      const tools = await api.mcp.listServerTools(id);
      setToolsCache((prev) => ({ ...prev, [id]: tools }));
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to list tools");
      setToolsCache((prev) => ({ ...prev, [id]: [] }));
    }
  };

  if (loading) {
    return (
      <div className="panel-loading-enhanced">
        <div className="spinner" />
        <span>Loading MCP servers...</span>
      </div>
    );
  }

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">MCP Servers</h2>
        <button className="btn btn-primary" onClick={() => setShowAddForm(!showAddForm)}>
          {showAddForm ? "Cancel" : "+ Add Server"}
        </button>
      </div>

      {showAddForm && (
        <McpServerForm
          onSave={() => {
            setShowAddForm(false);
            refresh();
          }}
        />
      )}

      {servers.length === 0 && !showAddForm ? (
        <div className="empty-state">
          <div className="empty-state-icon">🔌</div>
          <div className="empty-state-title">No MCP servers configured</div>
          <div className="empty-state-description">
            Add an MCP server to enable tool injection for LLM requests. Supports stdio-based servers with custom commands and environment variables.
          </div>
        </div>
      ) : (
        <div className="mcp-server-list">
          {servers.map((server) => {
            if (editingId === server.id) {
              return (
                <McpServerEditForm
                  key={server.id}
                  server={server}
                  onSave={() => {
                    setEditingId(null);
                    refresh();
                  }}
                  onCancel={() => setEditingId(null)}
                />
              );
            }
            return (
              <McpServerCard
                key={server.id}
                server={server}
                expanded={expandedId === server.id}
                tools={expandedId === server.id ? toolsCache[server.id] : undefined}
                confirmDelete={confirmDeleteId === server.id}
                actionLoading={actionLoading === server.id}
                onStart={() => handleStart(server.id)}
                onStop={() => handleStop(server.id)}
                onEdit={() => setEditingId(server.id)}
                onDelete={() => handleDelete(server.id)}
                onToggleExpand={() => handleExpandTools(server)}
              />
            );
          })}
        </div>
      )}
    </section>
  );
}

// ─── Server Card ────────────────────────────────────────

interface McpServerCardProps {
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

function McpServerCard({
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

// ─── Create Form ────────────────────────────────────────

function McpServerForm({ onSave }: { onSave: () => void }) {
  const toast = useToast();
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [env, setEnv] = useState("");
  const [cwd, setCwd] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [exposeTools, setExposeTools] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (!id.trim()) return setError("ID is required");
    if (!name.trim()) return setError("Name is required");
    if (!command.trim()) return setError("Command is required");

    const data: CreateMcpServerData = {
      id: id.trim(),
      name: name.trim(),
      command: command.trim(),
      args: parseArgsText(args),
      env: parseEnvText(env),
      cwd: cwd.trim() || null,
      enabled,
      expose_tools: exposeTools,
    };

    setSubmitting(true);
    try {
      await api.mcp.createServer(data);
      toast.success("Server created");
      onSave();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Failed to create server";
      setError(msg);
      toast.error(msg);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form className="channel-form" onSubmit={handleSubmit}>
      <h3 className="form-title">Add MCP Server</h3>
      <div className="form-grid">
        <label className="form-field">
          <span>ID</span>
          <input
            value={id}
            onChange={(e) => setId(e.target.value)}
            placeholder="e.g. filesystem"
            required
          />
        </label>
        <label className="form-field">
          <span>Name</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. Filesystem Server"
            required
          />
        </label>
        <label className="form-field">
          <span>Command</span>
          <input
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            placeholder="e.g. npx"
            required
          />
        </label>
        <label className="form-field">
          <span>Arguments</span>
          <input
            value={args}
            onChange={(e) => setArgs(e.target.value)}
            placeholder="e.g. -y @modelcontextprotocol/server-filesystem /tmp"
          />
        </label>
        <label className="form-field span-2">
          <span>Environment Variables (one KEY=VALUE per line)</span>
          <textarea
            value={env}
            onChange={(e) => setEnv(e.target.value)}
            placeholder={"API_KEY=abc123\nNODE_ENV=production"}
            rows={3}
            style={{
              width: "100%",
              padding: "var(--space-2) var(--space-3)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "var(--color-surface)",
              color: "var(--color-text)",
              fontFamily: "inherit",
              fontSize: "var(--text-sm)",
              outline: "none",
              resize: "vertical",
            }}
          />
        </label>
        <label className="form-field">
          <span>Working Directory</span>
          <input
            value={cwd}
            onChange={(e) => setCwd(e.target.value)}
            placeholder="/optional/path"
          />
        </label>
        <div className="form-field" style={{ flexDirection: "row", gap: "var(--space-4)", alignItems: "center" }}>
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
            />
            <span>Enabled</span>
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={exposeTools}
              onChange={(e) => setExposeTools(e.target.checked)}
            />
            <span>Expose Tools</span>
          </label>
        </div>
      </div>
      {error && <div className="form-error">{error}</div>}
      <button type="submit" className="btn btn-primary" disabled={submitting}>
        {submitting ? "Creating..." : "Create Server"}
      </button>
    </form>
  );
}

// ─── Edit Form ──────────────────────────────────────────

function McpServerEditForm({
  server,
  onSave,
  onCancel,
}: {
  server: McpServer;
  onSave: () => void;
  onCancel: () => void;
}) {
  const toast = useToast();
  const [name, setName] = useState(server.name);
  const [command, setCommand] = useState(server.command);
  const [args, setArgs] = useState(argsToText(server.args));
  const [env, setEnv] = useState(envToText(server.env));
  const [cwd, setCwd] = useState(server.cwd ?? "");
  const [enabled, setEnabled] = useState(server.enabled);
  const [exposeTools, setExposeTools] = useState(server.expose_tools);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (!name.trim()) return setError("Name is required");
    if (!command.trim()) return setError("Command is required");

    const data: UpdateMcpServerData = {
      name: name.trim(),
      command: command.trim(),
      args: parseArgsText(args),
      env: parseEnvText(env),
      cwd: cwd.trim() || null,
      enabled,
      expose_tools: exposeTools,
    };

    setSubmitting(true);
    try {
      await api.mcp.updateServer(server.id, data);
      toast.success("Server updated");
      onSave();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Failed to update server";
      setError(msg);
      toast.error(msg);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form className="channel-form" onSubmit={handleSubmit}>
      <h3 className="form-title">Edit Server: {server.id}</h3>
      <div className="form-grid">
        <label className="form-field">
          <span>Name</span>
          <input value={name} onChange={(e) => setName(e.target.value)} required />
        </label>
        <label className="form-field">
          <span>Command</span>
          <input value={command} onChange={(e) => setCommand(e.target.value)} required />
        </label>
        <label className="form-field span-2">
          <span>Arguments</span>
          <input
            value={args}
            onChange={(e) => setArgs(e.target.value)}
            placeholder="space-separated arguments"
          />
        </label>
        <label className="form-field span-2">
          <span>Environment Variables (one KEY=VALUE per line)</span>
          <textarea
            value={env}
            onChange={(e) => setEnv(e.target.value)}
            rows={3}
            style={{
              width: "100%",
              padding: "var(--space-2) var(--space-3)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "var(--color-surface)",
              color: "var(--color-text)",
              fontFamily: "inherit",
              fontSize: "var(--text-sm)",
              outline: "none",
              resize: "vertical",
            }}
          />
        </label>
        <label className="form-field">
          <span>Working Directory</span>
          <input value={cwd} onChange={(e) => setCwd(e.target.value)} />
        </label>
        <div className="form-field" style={{ flexDirection: "row", gap: "var(--space-4)", alignItems: "center" }}>
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
            />
            <span>Enabled</span>
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={exposeTools}
              onChange={(e) => setExposeTools(e.target.checked)}
            />
            <span>Expose Tools</span>
          </label>
        </div>
      </div>
      {error && <div className="form-error">{error}</div>}
      <div style={{ display: "flex", gap: "var(--space-2)" }}>
        <button type="submit" className="btn btn-primary" disabled={submitting}>
          {submitting ? "Saving..." : "Save"}
        </button>
        <button type="button" className="btn" onClick={onCancel} disabled={submitting}>
          Cancel
        </button>
      </div>
    </form>
  );
}
