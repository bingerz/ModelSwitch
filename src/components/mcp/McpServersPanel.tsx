import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../lib/api";
import { useToast } from "../Toast";
import "../../styles/pages-enhanced.css";
import type { McpServer, McpToolDetail } from "./types";
import { statusIsRunning } from "./types";
import { McpServerCard } from "./McpServerCard";
import { McpServerForm } from "./McpServerForm";
import { McpServerEditForm } from "./McpServerEditForm";

export function McpServersPanel() {
  const { t } = useTranslation();
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
      toast.error(err instanceof Error ? err.message : t("mcp.loadFailed"));
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
      toast.success(t("mcp.started"));
      await refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : t("mcp.startFailed"));
    } finally {
      setActionLoading(null);
    }
  };

  const handleStop = async (id: string) => {
    setActionLoading(id);
    try {
      await api.mcp.stopServer(id);
      toast.success(t("mcp.stoppedToast"));
      await refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : t("mcp.stopFailed"));
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
      toast.success(t("mcp.deleted"));
      await refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : t("mcp.deleteFailed"));
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
      toast.error(err instanceof Error ? err.message : t("mcp.listToolsFailed"));
      setToolsCache((prev) => ({ ...prev, [id]: [] }));
    }
  };

  if (loading) {
    return (
      <div className="panel-loading-enhanced">
        <div className="spinner" />
        <span>{t("mcp.loading")}</span>
      </div>
    );
  }

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">{t("mcp.title")}</h2>
        <button className="btn btn-primary" onClick={() => setShowAddForm(!showAddForm)}>
          {showAddForm ? t("common.cancel") : t("mcp.create")}
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
          <div className="empty-state-title">{t("mcp.empty")}</div>
          <div className="empty-state-description">
            {t("mcp.emptyHint")}
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
