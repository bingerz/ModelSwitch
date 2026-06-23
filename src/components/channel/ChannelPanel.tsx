import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, type Channel, PRIORITY_TIERS } from "../../lib/api";
import { useQuota } from "../../hooks/useQuota";
import type { QuotaInfo } from "../../lib/api";
import { useToast } from "../Toast";
import { ChannelCard } from "./ChannelCard";
import { ChannelForm } from "./ChannelForm";
import { EditChannelForm } from "./EditChannelForm";

export function ChannelPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const [showAddForm, setShowAddForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const { quotas, channels, refresh, loading } = useQuota();

  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [overflowOpenId, setOverflowOpenId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<
    "all" | "healthy" | "circuit_open" | "disabled"
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

  const handleToggle = async (ch: Channel) => {
    try {
      await api.updateChannel(ch.id, {
        name: ch.name,
        provider: ch.provider,
        priority: ch.priority,
        weight: ch.weight,
        cost_per_token: ch.cost_per_token,
        base_url: ch.base_url,
        enabled: !ch.enabled,
        model_mapping: ch.model_mapping,
        cooldown_minutes: ch.cooldown_minutes,
      });
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
        await api.updateChannel(ch.id, {
          name: ch.name,
          provider: ch.provider,
          priority: targetPriority,
          weight: ch.weight,
          cost_per_token: ch.cost_per_token,
          base_url: ch.base_url,
          enabled: ch.enabled,
          model_mapping: ch.model_mapping,
          cooldown_minutes: ch.cooldown_minutes,
        });
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
          <option value="disabled">{t("channels.disabled")}</option>
        </select>
      </div>
      {(searchQuery || statusFilter !== "all") && (
        <p className="filter-results-count">
          {t("channels.showingCount", { shown: showingChannels, total: totalChannels })}
        </p>
      )}

      <div className="tiers-container">
        {allPriorities.map((priority) => {
          const meta = PRIORITY_TIERS[priority] || {
            label: t(`channels.priority${priority}Label`),
            desc: t(`channels.priority${priority}Desc`),
          };
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
                <span className="tier-label">{meta.label}</span>
                <span className="tier-desc">{meta.desc}</span>
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
    </section>
  );
}
