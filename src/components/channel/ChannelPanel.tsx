import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, type Channel, type UpdateChannelData } from "../../lib/api";
import { useQuota } from "../../hooks/useQuota";
import type { QuotaInfo } from "../../lib/api";
import { useToast } from "../Toast";
import { ChannelCard } from "./ChannelCard";
import { ChannelForm } from "./ChannelForm";
import { EditChannelForm } from "./EditChannelForm";
import { BatchOperationsBar } from "./BatchOperationsBar";
import { DiagnosticsModal } from "./DiagnosticsModal";

/** Convert a Channel to UpdateChannelData, preserving all fields to prevent
 *  silent data loss on partial updates (toggle, drag-drop). */
function channelToUpdateData(ch: Channel, overrides: Partial<UpdateChannelData>): UpdateChannelData {
  return {
    name: ch.name,
    provider: ch.provider,
    priority: ch.priority,
    weight: ch.weight,
    cost_per_token: ch.cost_per_token,
    input_cost_per_mtok: ch.input_cost_per_mtok,
    output_cost_per_mtok: ch.output_cost_per_mtok,
    base_url: ch.base_url,
    enabled: ch.enabled,
    model_mapping: ch.model_mapping,
    cooldown_minutes: ch.cooldown_minutes,
    rpm_limit: ch.rpm_limit,
    tpm_limit: ch.tpm_limit,
    account_group: ch.account_group,
    excluded_models: ch.excluded_models,
    tags: ch.tags,
    models_endpoint: ch.models_endpoint,
    models_refresh_interval_secs: ch.models_refresh_interval_secs,
    max_concurrent: ch.max_concurrent,
    api_keys: ch.api_keys,
    proxy_url: ch.proxy_url,
    headers: ch.headers,
    max_retries: ch.max_retries,
    ...overrides,
  };
}

export function ChannelPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const [showAddForm, setShowAddForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const { quotas, channels, refresh, loading } = useQuota();

  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [overflowOpenId, setOverflowOpenId] = useState<string | null>(null);
  const [diagnosticsId, setDiagnosticsId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [statusFilter, setStatusFilter] = useState<
    "all" | "healthy" | "circuit_open" | "half_open" | "disabled"
  >("all");

  const handleDelete = async (id: string) => {
    // Two-click confirmation: first click shows "Confirm?", second click deletes
    if (confirmDeleteId !== id) {
      setConfirmDeleteId(id);
      return;
    }
    setConfirmDeleteId(null);
    try {
      await api.deleteChannel(id);
      toast.success(t("channels.deleted"));
      refresh();
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : t("channels.deleteFailed"));
    }
  };

  const handlePing = async (id: string) => {
    try {
      const result = await api.pingChannel(id);
      toast.success(result.success ? t("channels.pingOk", { latency: result.latency_ms }) : t("channels.pingFailed"));
    } catch {
      toast.error(t("channels.pingError"));
    }
  };

  const toggleSelect = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const handleTestAll = async () => {
    try {
      const results = await api.testAllChannels();
      const healthy = results.filter((r) => r.healthy).length;
      toast.success(t("channels.testAllResult", { healthy, total: results.length }));
    } catch {
      toast.error(t("channels.testAllFailed"));
    }
  };

  const handleTest = async (id: string) => {
    try {
      const result = await api.testChannel(id);
      if (result.healthy) {
        toast.success(t("channels.testOk", { latency: result.latency_ms }));
      } else {
        toast.error(t("channels.testFailed", { error: result.error }));
      }
    } catch {
      toast.error(t("channels.testError"));
    }
  };

  const handleDiagnostics = (id: string) => {
    setDiagnosticsId(id);
  };

  const handleResetCircuit = async (id: string) => {
    try {
      await api.resetCircuit(id);
      toast.success(t("channels.circuitResetSuccess"));
      refresh();
    } catch {
      toast.error(t("channels.circuitResetFailed"));
    }
  };

  const handleToggle = async (ch: Channel) => {
    try {
      await api.updateChannel(ch.id, channelToUpdateData(ch, { enabled: !ch.enabled }));
      refresh();
    } catch {
      toast.error(t("channels.toggleFailed"));
      refresh();
    }
  };

  // Drag-and-drop: move channel to a different priority
  const handleDragStart = (id: string) => setDragId(id);

  const handleDrop = async (targetPriority: number) => {
    if (!dragId) return;
    const ch = channels.find((c) => c.id === dragId);
    if (ch && ch.priority !== targetPriority) {
      try {
        await api.updateChannel(ch.id, channelToUpdateData(ch, { priority: targetPriority }));
        refresh();
      } catch {
        refresh();
      }
    }
    setDragId(null);
  };

  if (loading) return <div className="panel-loading">{t("channels.loadingChannels")}</div>;

  // Apply search and status filters
  const filteredChannels = channels.filter((ch) => {
    if (statusFilter !== "all") {
      if (statusFilter === "healthy" && ch.status !== "healthy") return false;
      if (statusFilter === "circuit_open" && ch.status !== "circuit_open") return false;
      if (statusFilter === "half_open" && ch.status !== "half_open") return false;
      if (statusFilter === "disabled" && ch.status !== "disabled") return false;
    }
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      return (
        ch.name.toLowerCase().includes(q) || ch.provider.toLowerCase().includes(q)
      );
    }
    return true;
  });
  const totalChannels = channels.length;
  const showingChannels = filteredChannels.length;

  // Group channels by priority
  const priorities: Map<number, Channel[]> = new Map();
  for (const ch of filteredChannels) {
    const list = priorities.get(ch.priority) || [];
    list.push(ch);
    priorities.set(ch.priority, list);
  }
  const allPriorities = [1, 2, 3];

  // Build quota lookup map
  const quotaMap = new Map<string, QuotaInfo>();
  for (const q of quotas) {
    quotaMap.set(q.channel_id, q);
  }

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">{t("channels.title")}</h2>
        <button className="btn btn-sm" onClick={handleTestAll} title={t("channels.testAll")}>
          {t("channels.testAll")}
        </button>
        <button className="btn btn-primary" onClick={() => setShowAddForm(!showAddForm)}>
          {showAddForm ? t("common.cancel") : t("channels.create")}
        </button>
      </div>

      {showAddForm && (
        <ChannelForm
          onSave={() => {
            setShowAddForm(false);
            refresh();
          }}
        />
      )}

      <div className="channel-toolbar">
        <div className="channel-search">
          <input
            type="text"
            className="channel-search-input"
            placeholder={t("channels.searchPlaceholder")}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          {searchQuery && (
            <button
              className="channel-search-clear"
              onClick={() => setSearchQuery("")}
              title={t("common.clear")}
            >
              {"\u00D7"}
            </button>
          )}
        </div>
        <select
          className="channel-status-filter"
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value as typeof statusFilter)}
        >
          <option value="all">{t("channels.allStatus")}</option>
          <option value="healthy">{t("channels.healthy")}</option>
          <option value="circuit_open">{t("channels.circuitOpen")}</option>
          <option value="half_open">{t("channels.halfOpen")}</option>
          <option value="disabled">{t("channels.disabled")}</option>
        </select>
      </div>
      {(searchQuery || statusFilter !== "all") && (
        <p className="filter-results-count">
          {t("channels.showingCount", { shown: showingChannels, total: totalChannels })}
        </p>
      )}

      <BatchOperationsBar
          selectedIds={Array.from(selectedIds)}
          onClear={() => setSelectedIds(new Set())}
          onDone={() => {
            refresh();
            setSelectedIds(new Set());
          }}
        />
        <div className="tiers-container">
        {allPriorities.map((priority) => {
          const tierLabel = t(`channels.priority${priority}Label`);
          const tierDesc = t(`channels.priority${priority}Desc`);
          const channelsInPriority = priorities.get(priority) || [];
          return (
            <div
              key={priority}
              className={`tier-column ${dragId ? "tier-drop-target" : ""}`}
              data-priority={priority}
              onDragOver={(e) => {
                e.preventDefault();
                e.dataTransfer.dropEffect = "move";
              }}
              onDrop={() => handleDrop(priority)}
            >
              <div className="tier-header">
                <span className="tier-label">{tierLabel}</span>
                <span className="tier-desc">{tierDesc}</span>
                <span className="tier-count">{channelsInPriority.length}</span>
              </div>
              {channelsInPriority.length === 0 ? (
                <div className="tier-empty">{t("channels.dropChannelHere")}</div>
              ) : (
                <div className="tier-channels">
                  {channelsInPriority.map((ch) => {
                    if (editingId === ch.id) {
                      return (
                        <EditChannelForm
                          key={ch.id}
                          channel={ch}
                          onSave={() => {
                            setEditingId(null);
                            refresh();
                          }}
                          onCancel={() => setEditingId(null)}
                        />
                      );
                    }
                    return (
                      <ChannelCard
                        key={ch.id}
                        channel={ch}
                        quota={quotaMap.get(ch.id)}
                        selected={selectedIds.has(ch.id)}
                        onToggleSelect={() => toggleSelect(ch.id)}
                        onTest={() => handleTest(ch.id)}
                        onDiagnostics={() => handleDiagnostics(ch.id)}
                        onResetCircuit={() => handleResetCircuit(ch.id)}
                        confirmDelete={confirmDeleteId === ch.id}
                        overflowOpen={overflowOpenId === ch.id}
                        onEdit={() => setEditingId(ch.id)}
                        onToggle={() => handleToggle(ch)}
                        onPing={() => handlePing(ch.id)}
                        onDelete={() => handleDelete(ch.id)}
                        onToggleOverflow={() =>
                          setOverflowOpenId(overflowOpenId === ch.id ? null : ch.id)
                        }
                        onCloseOverflow={() => setOverflowOpenId(null)}
                        onCancelDelete={() => setConfirmDeleteId(null)}
                        onDragStart={() => handleDragStart(ch.id)}
                        onDragEnd={() => setDragId(null)}
                      />
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {diagnosticsId && (
        <DiagnosticsModal
          channelId={diagnosticsId}
          channelName={
            channels.find((c) => c.id === diagnosticsId)?.name ?? ""
          }
          onClose={() => setDiagnosticsId(null)}
        />
      )}
    </section>
  );
}
