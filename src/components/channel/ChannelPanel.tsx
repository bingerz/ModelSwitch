import { useState } from "react";
import { api, type Channel, PRIORITY_TIERS } from "../../lib/api";
import { useQuota } from "../../hooks/useQuota";
import type { QuotaInfo } from "../../lib/api";
import { useToast } from "../Toast";
import { ChannelCard } from "./ChannelCard";
import { ChannelForm } from "./ChannelForm";
import { EditChannelForm } from "./EditChannelForm";

export function ChannelPanel() {
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
      toast.success("Channel deleted");
      refresh();
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "Failed to delete channel");
    }
  };

  const handlePing = async (id: string) => {
    try {
      const result = await api.pingChannel(id);
      toast.success(result.success ? `Ping OK (${result.latency_ms}ms)` : "Ping failed");
    } catch {
      toast.error("Ping error");
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
      toast.error("Failed to toggle channel");
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

  if (loading) return <div className="panel-loading">Loading channels...</div>;

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
        <h2 className="panel-title">Channels</h2>
        <button className="btn btn-primary" onClick={() => setShowAddForm(!showAddForm)}>
          {showAddForm ? "Cancel" : "+ Add Channel"}
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
            placeholder="Search channels..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          {searchQuery && (
            <button
              className="channel-search-clear"
              onClick={() => setSearchQuery("")}
              title="Clear search"
            >
              ×
            </button>
          )}
        </div>
        <select
          className="channel-status-filter"
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value as typeof statusFilter)}
        >
          <option value="all">All Status</option>
          <option value="healthy">Healthy</option>
          <option value="circuit_open">Circuit Open</option>
          <option value="disabled">Disabled</option>
        </select>
      </div>
      {(searchQuery || statusFilter !== "all") && (
        <p className="filter-results-count">
          Showing {showingChannels} of {totalChannels} channels
        </p>
      )}

      <div className="tiers-container">
        {allPriorities.map((priority) => {
          const meta = PRIORITY_TIERS[priority] || {
            label: `Priority ${priority}`,
            desc: "",
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
                <div className="tier-empty">Drop a channel here</div>
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
