import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useToast } from "../Toast";
import "../../styles/pages-enhanced.css";
import type { McpServer, McpToolDetail } from "./types";
import { statusIsRunning } from "./types";
import { McpServerCard } from "./McpServerCard";
import { McpServerForm } from "./McpServerForm";
import { McpServerEditForm } from "./McpServerEditForm";

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
