import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { KeyRound, CheckCircle, DollarSign, Search, ChevronLeft, ChevronRight } from "lucide-react";
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

const PAGE_SIZE = 20;

export function VirtualKeysPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const [keys, setKeys] = useState<VirtualKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [showAddForm, setShowAddForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [createdResponse, setCreatedResponse] =
    useState<CreateVirtualKeyResponse | null>(null);

  // Pagination + search state
  const [page, setPage] = useState(1);
  const [total, setTotal] = useState(0);
  const [searchInput, setSearchInput] = useState("");
  const [activeSearch, setActiveSearch] = useState("");
  const searchDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const limit = PAGE_SIZE;
  const totalPages = Math.max(1, Math.ceil(total / limit));

  const refresh = useCallback(async () => {
    try {
      const res = await api.virtualKeys.list({ page, limit, search: activeSearch || undefined });
      setKeys(res.data);
      setTotal(res.total);
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : t("virtualKeys.loadFailed"),
      );
    } finally {
      setLoading(false);
    }
  }, [toast, t, page, limit, activeSearch]);

  // Debounced search: when searchInput changes, debounce then commit to activeSearch + reset page
  useEffect(() => {
    if (searchDebounceRef.current) {
      clearTimeout(searchDebounceRef.current);
    }
    searchDebounceRef.current = setTimeout(() => {
      setActiveSearch((prev) => {
        if (prev !== searchInput) {
          setPage(1);
          return searchInput;
        }
        return prev;
      });
    }, 300);
    return () => {
      if (searchDebounceRef.current) {
        clearTimeout(searchDebounceRef.current);
      }
    };
  }, [searchInput]);

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
      toast.success(t("virtualKeys.deleted"));
      await refresh();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : t("virtualKeys.deleteFailed"),
      );
    } finally {
      setActionLoading(null);
    }
  };

  const handleToggleEnabled = async (vk: VirtualKey) => {
    setActionLoading(vk.id);
    try {
      await api.virtualKeys.update(vk.id, { enabled: !vk.enabled });
      toast.success(vk.enabled ? t("virtualKeys.keyDisabled") : t("virtualKeys.keyEnabled"));
      await refresh();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : t("virtualKeys.updateFailed"),
      );
    } finally {
      setActionLoading(null);
    }
  };

  if (loading) {
    return (
      <div className="panel-loading-enhanced">
        <div className="spinner" />
        <span>{t("virtualKeys.loading")}</span>
      </div>
    );
  }

  return (
    <section className="vk-panel">
      <SectionHeader
        title={t("virtualKeys.title")}
        icon={KeyRound}
        action={
          <button
            className="btn btn-primary"
            onClick={() => setShowAddForm(!showAddForm)}
          >
            {showAddForm ? t("common.cancel") : t("virtualKeys.create")}
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

      {/* Search + pagination toolbar */}
      <div className="vk-toolbar">
        <div className="vk-search">
          <Search size={14} className="vk-search-icon" />
          <input
            type="text"
            className="vk-search-input"
            placeholder={t("virtualKeys.searchPlaceholder")}
            value={searchInput}
            onChange={(e) => setSearchInput(e.target.value)}
          />
          {searchInput && (
            <button
              className="vk-search-clear"
              onClick={() => setSearchInput("")}
              title={t("common.clear")}
            >
              {"\u00D7"}
            </button>
          )}
        </div>
        {totalPages > 1 && (
          <div className="vk-pagination">
            <button
              className="btn btn-sm"
              disabled={page <= 1}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              title={t("common.previous")}
            >
              <ChevronLeft size={16} />
            </button>
            <span className="vk-page-info">
              {page} / {totalPages}
            </span>
            <button
              className="btn btn-sm"
              disabled={page >= totalPages}
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
              title={t("common.next")}
            >
              <ChevronRight size={16} />
            </button>
          </div>
        )}
      </div>

      {/* Summary stat cards */}
      {keys.length > 0 && (
        <div className="vk-summary-grid">
          <StatTile
            icon={KeyRound}
            value={total}
            label={t("virtualKeys.totalKeys")}
            accent="blue"
          />
          <StatTile
            icon={CheckCircle}
            value={keys.filter((k) => k.enabled).length}
            label={t("virtualKeys.activeKeys")}
            accent="green"
          />
          <StatTile
            icon={DollarSign}
            value={formatCents(keys.reduce((sum, k) => sum + k.spend.this_month.cents, 0))}
            label={t("virtualKeys.spendThisMonth")}
            accent="amber"
          />
        </div>
      )}

      {keys.length === 0 && !showAddForm ? (
        <EmptyState
          icon={KeyRound}
          title={t("virtualKeys.empty")}
          description={t("virtualKeys.emptyHint")}
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
