import { useCallback, useEffect, useState } from "react";
import { KeyRound, CheckCircle, DollarSign } from "lucide-react";
import {
  api,
  type VirtualKey,
  type CreateVirtualKeyResponse,
} from "../../lib/api";
import { formatCents } from "../../lib/format";
import { useToast } from "../Toast";
import { StatTile } from "../ui/StatTile";
import { SectionHeader } from "../ui/SectionHeader";
import { EmptyState } from "../ui/EmptyState";
import "../../styles/pages-enhanced.css";
import { PlaintextBanner } from "./PlaintextBanner";
import { VirtualKeyCard } from "./VirtualKeyCard";
import { VirtualKeyForm } from "./VirtualKeyForm";

export function VirtualKeysPanel() {
  const toast = useToast();
  const [keys, setKeys] = useState<VirtualKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [showAddForm, setShowAddForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [createdResponse, setCreatedResponse] =
    useState<CreateVirtualKeyResponse | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await api.virtualKeys.list();
      setKeys(list);
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to load virtual keys",
      );
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleCreated = (response: CreateVirtualKeyResponse) => {
    setCreatedResponse(response);
    setShowAddForm(false);
    refresh();
  };

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) {
      setConfirmDeleteId(id);
      return;
    }
    setConfirmDeleteId(null);
    setActionLoading(id);
    try {
      await api.virtualKeys.delete(id);
      toast.success("Virtual key deleted");
      await refresh();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to delete virtual key",
      );
    } finally {
      setActionLoading(null);
    }
  };

  const handleToggleEnabled = async (vk: VirtualKey) => {
    setActionLoading(vk.id);
    try {
      await api.virtualKeys.update(vk.id, { enabled: !vk.enabled });
      toast.success(vk.enabled ? "Key disabled" : "Key enabled");
      await refresh();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to update key",
      );
    } finally {
      setActionLoading(null);
    }
  };

  if (loading) {
    return (
      <div className="panel-loading-enhanced">
        <div className="spinner" />
        <span>Loading virtual keys...</span>
      </div>
    );
  }

  return (
    <section className="vk-panel">
      <SectionHeader
        title="Virtual Keys"
        icon={KeyRound}
        action={
          <button
            className="btn btn-primary"
            onClick={() => setShowAddForm(!showAddForm)}
          >
            {showAddForm ? "Cancel" : "+ Add Key"}
          </button>
        }
      />

      {createdResponse && (
        <PlaintextBanner
          response={createdResponse}
          onClose={() => setCreatedResponse(null)}
        />
      )}

      {showAddForm && (
        <VirtualKeyForm
          mode="create"
          onSave={(resp) => {
            if (resp) {
              handleCreated(resp);
            } else {
              setShowAddForm(false);
              refresh();
            }
          }}
          onCancel={() => setShowAddForm(false)}
        />
      )}

      {/* Summary stat cards */}
      {keys.length > 0 && (
        <div className="vk-summary-grid">
          <StatTile
            icon={KeyRound}
            value={keys.length}
            label="Total Keys"
            accent="blue"
          />
          <StatTile
            icon={CheckCircle}
            value={keys.filter((k) => k.enabled).length}
            label="Active Keys"
            accent="green"
          />
          <StatTile
            icon={DollarSign}
            value={formatCents(keys.reduce((sum, k) => sum + k.spend.this_month.cents, 0))}
            label="Spend This Month"
            accent="amber"
          />
        </div>
      )}

      {keys.length === 0 && !showAddForm ? (
        <EmptyState
          icon={KeyRound}
          title="No virtual keys configured"
          description='Create virtual keys to distribute access with per-key budgets and rate limits. Click "Add Key" to get started.'
        />
      ) : (
        <div className="vk-list">
          {keys.map((vk) => {
            if (editingId === vk.id) {
              return (
                <VirtualKeyForm
                  key={vk.id}
                  mode="edit"
                  existingKey={vk}
                  onSave={() => {
                    setEditingId(null);
                    refresh();
                  }}
                  onCancel={() => setEditingId(null)}
                />
              );
            }
            return (
              <VirtualKeyCard
                key={vk.id}
                vk={vk}
                confirmDelete={confirmDeleteId === vk.id}
                actionLoading={actionLoading === vk.id}
                onEdit={() => setEditingId(vk.id)}
                onDelete={() => handleDelete(vk.id)}
                onToggleEnabled={() => handleToggleEnabled(vk)}
              />
            );
          })}
        </div>
      )}
    </section>
  );
}
