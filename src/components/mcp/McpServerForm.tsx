import { useState } from "react";
import { api } from "../../lib/api";
import { useToast } from "../Toast";
import type { CreateMcpServerData } from "./types";
import { parseArgsText, parseEnvText } from "./types";

export interface McpServerFormProps {
  onSave: () => void;
}

export function McpServerForm({ onSave }: McpServerFormProps) {
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
