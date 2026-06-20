import { useState } from "react";
import { api } from "../../lib/api";
import { useToast } from "../Toast";
import type { McpServer, UpdateMcpServerData } from "./types";
import { argsToText, envToText, parseArgsText, parseEnvText } from "./types";

export interface McpServerEditFormProps {
  server: McpServer;
  onSave: () => void;
  onCancel: () => void;
}

export function McpServerEditForm({
  server,
  onSave,
  onCancel,
}: McpServerEditFormProps) {
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
