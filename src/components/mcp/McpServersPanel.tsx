import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Server } from "lucide-react";
import { api } from "../../lib/api";
import { useToast } from "../Toast";
import { PanelLayout } from "../ui/PanelLayout";
import { LoadingState } from "../ui/LoadingState";
import { EmptyState } from "../ui/EmptyState";
import "../../styles/pages-enhanced.css";
import type { McpServer, McpToolDetail } from "./types";
import { statusIsRunning } from "./types";
import { McpServerCard } from "./McpServerCard";
import { McpServerForm } from "./McpServerForm";
import { McpServerEditForm } from "./McpServerEditForm";
import { McpAllToolsPanel } from "./McpAllToolsPanel";

interface McpHealthEntry {
  name: string;
  healthy: boolean;
  last_check: string | null;
  last_error: string | null;
  consecutive_failures: number;
}

interface McpListResult {
  servers: McpServer[];
  healthMap: Record<string, McpHealthEntry>;
}

export function McpServersPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const [showAddForm, setShowAddForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [toolsCache, setToolsCache] = useState<Record<string, McpToolDetail[]>>({});
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);

  const { data, isLoading: loading, error, refetch } = useQuery({
    queryKey: ["mcp-servers"],
    queryFn: async (): Promise<McpListResult> => {
      const [list, healthList] = await Promise.all([
        api.mcp.listServers(),
        api.mcpHealth().catch(() => [] as McpHealthEntry[]),
      ]);
      const byName: Record<string, McpHealthEntry> = {};
      for (const h of healthList) byName[h.name] = h;
      return { servers: list, healthMap: byName };
    },
    refetchInterval: 15_000,
    retry: false,
  });

  const servers = data?.servers ?? [];
  const healthMap = data?.healthMap ?? {};

  // Show toast when fetch error changes
  useEffect(() => {
    if (error) {
      toast.error(error instanceof Error ? error.message : t("mcp.loadFailed"));
    }
  }, [error, t, toast]);

  const refresh = async () => {
    await refetch();
  };

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

  const healthValues = Object.values(healthMap);
  const healthyCount = healthValues.filter((h) => h.healthy).length;
  const showHealthSummary = healthValues.length > 0;

  if (loading) {
    return (
      <PanelLayout title={t("mcp.title")} icon={Server}>
        <LoadingState message={t("mcp.loading")} icon={Server} />
      </PanelLayout>
    );
  }

  const panelActions = (
    <button
      className="btn btn-primary"
      onClick={() => setShowAddForm(!showAddForm)}
    >
      {showAddForm ? t("common.cancel") : t("mcp.create")}
    </button>
  );

  return (
    <PanelLayout
      title={t("mcp.title")}
      icon={Server}
      actions={panelActions}
      onRefresh={refresh}
    >

      {showHealthSummary && (
        <div
          className="mcp-health-summary"
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-2)",
            padding: "var(--space-2) var(--space-3)",
            marginBottom: "var(--space-3)",
            background: "var(--color-surface-2, var(--color-surface))",
            border: "1px solid var(--color-border)",
            borderRadius: "6px",
            fontSize: "var(--text-sm)",
          }}
        >
          <span
            className="status-dot"
            style={{
              width: 8,
              height: 8,
              borderRadius: "50%",
              display: "inline-block",
              background:
                healthyCount === healthValues.length
                  ? "var(--color-success)"
                  : healthyCount === 0
                    ? "var(--color-danger)"
                    : "var(--color-warning)",
            }}
          />
          <strong>{t("mcp.healthStatus")}:</strong>
          <span>
            {t("mcp.healthSummary", { healthy: healthyCount, total: healthValues.length })}
          </span>
        </div>
      )}

      {servers.length > 0 && (
        <>
          <div className="settings-hint" style={{ marginBottom: "0.75rem", padding: "0.5rem 0.75rem", background: "var(--color-bg-secondary, #f5f5f5)", borderRadius: "6px", fontSize: "var(--text-sm, 0.875rem)" }}>
            {t("mcp.streamWarning")}
          </div>
          <McpAllToolsPanel servers={servers} />
        </>
      )}

      {showAddForm && (
        <McpServerForm
          onSave={() => {
            setShowAddForm(false);
            refresh();
          }}
        />
      )}

      {servers.length === 0 && !showAddForm ? (
        <EmptyState
          icon={Server}
          title={t("mcp.empty")}
          description={t("mcp.emptyHint")}
        />
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
                health={healthMap[server.name]}
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
    </PanelLayout>
  );
}
