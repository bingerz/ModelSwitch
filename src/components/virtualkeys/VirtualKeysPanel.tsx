import { useEffect, useRef, useState } from "react";
import { useQuery, useMutation, useQueryClient, keepPreviousData } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { KeyRound, CheckCircle, DollarSign, Search, ChevronLeft, ChevronRight, Layers, FileUp } from "lucide-react";
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
import { BatchCreateModal } from "./BatchCreateModal";
import { CsvImportModal } from "./CsvImportModal";

const PAGE_SIZE = 20;

export function VirtualKeysPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const queryClient = useQueryClient();

  // UI state
  const [showAddForm, setShowAddForm] = useState(false);
  const [showBatchModal, setShowBatchModal] = useState(false);
  const [showCsvModal, setShowCsvModal] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [createdResponse, setCreatedResponse] =
    useState<CreateVirtualKeyResponse | null>(null);

  // Pagination + search state
  const [page, setPage] = useState(1);
  const [searchInput, setSearchInput] = useState("");
  const [activeSearch, setActiveSearch] = useState("");
  const searchDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const limit = PAGE_SIZE;

  // Group filter state
  const [selectedGroup, setSelectedGroup] = useState<string>("");

  // ── Queries ────────────────────────────────────────────

  const keysQuery = useQuery({
    queryKey: ["virtualKeys", page, limit, activeSearch],
    queryFn: () =>
      api.virtualKeys.list({ page, limit, search: activeSearch || undefined }),
    refetchInterval: 15_000,
    placeholderData: keepPreviousData,
  });

  const groupsQuery = useQuery({
    queryKey: ["virtualKeyGroups"],
    queryFn: () => api.virtualKeys.groups(),
  });

  const keys = keysQuery.data?.data ?? [];
  const total = keysQuery.data?.total ?? 0;
  const loading = keysQuery.isLoading;
  const groups = groupsQuery.data ?? [];
  const totalPages = Math.max(1, Math.ceil(total / limit));

  // Client-side filtered keys (group filter is applied locally to the current page)
  const displayedKeys =
    selectedGroup === ""
      ? keys
      : keys.filter((k) => (k.group ?? "") === selectedGroup);

  // Surface fetch errors via toast
  useEffect(() => {
    if (keysQuery.error) {
      toast.error(
        keysQuery.error instanceof Error
          ? keysQuery.error.message
          : t("virtualKeys.loadFailed"),
      );
    }
  }, [keysQuery.error, t, toast]);

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

  // ── Mutations ──────────────────────────────────────────

  const invalidateKeys = () =>
    queryClient.invalidateQueries({ queryKey: ["virtualKeys"] });
  const invalidateGroups = () =>
    queryClient.invalidateQueries({ queryKey: ["virtualKeyGroups"] });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.virtualKeys.delete(id),
    onSuccess: () => {
      invalidateKeys();
      invalidateGroups();
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      api.virtualKeys.update(id, { enabled }),
    onSuccess: () => {
      invalidateKeys();
    },
  });

  const handleCreated = (response: CreateVirtualKeyResponse) => {
    setCreatedResponse(response);
    setShowAddForm(false);
    invalidateKeys();
  };

  const handleBatchCreated = () => {
    invalidateKeys();
    invalidateGroups();
  };

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) {
      setConfirmDeleteId(id);
      return;
    }
    setConfirmDeleteId(null);
    setActionLoading(id);
    try {
      await deleteMutation.mutateAsync(id);
      toast.success(t("virtualKeys.deleted"));
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
      await toggleMutation.mutateAsync({ id: vk.id, enabled: !vk.enabled });
      toast.success(
        vk.enabled ? t("virtualKeys.keyDisabled") : t("virtualKeys.keyEnabled"),
      );
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
          <div style={{ display: "flex", gap: "var(--space-2)" }}>
            <button
              className="btn"
              onClick={() => setShowCsvModal(true)}
              title={t("virtualKeys.csv.buttonHint")}
            >
              <FileUp size={14} />
              {t("virtualKeys.csv.button")}
            </button>
            <button
              className="btn"
              onClick={() => setShowBatchModal(true)}
              title={t("virtualKeys.batch.buttonHint")}
            >
              <Layers size={14} />
              {t("virtualKeys.batch.button")}
            </button>
            <button
              className="btn btn-primary"
              onClick={() => setShowAddForm(!showAddForm)}
            >
              {showAddForm ? t("common.cancel") : t("virtualKeys.create")}
            </button>
          </div>
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
              invalidateKeys();
            }
          }}
          onCancel={() => setShowAddForm(false)}
        />
      )}

      {showBatchModal && (
        <BatchCreateModal
          onCreated={handleBatchCreated}
          onClose={() => setShowBatchModal(false)}
        />
      )}

      {showCsvModal && (
        <CsvImportModal
          onCreated={handleBatchCreated}
          onClose={() => setShowCsvModal(false)}
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
        {groups.length > 0 && (
          <select
            className="vk-group-filter"
            value={selectedGroup}
            onChange={(e) => setSelectedGroup(e.target.value)}
            aria-label={t("virtualKeys.groupFilterLabel")}
            title={t("virtualKeys.groupFilterLabel")}
          >
            <option value="">{t("virtualKeys.groupFilterAll")}</option>
            {groups.map((g) => (
              <option key={g} value={g}>
                {g}
              </option>
            ))}
          </select>
        )}
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

      {displayedKeys.length === 0 && !showAddForm ? (
        <EmptyState
          icon={KeyRound}
          title={t("virtualKeys.empty")}
          description={t("virtualKeys.emptyHint")}
        />
      ) : (
        <div className="vk-list">
          {displayedKeys.map((vk) => {
            if (editingId === vk.id) {
              return (
                <VirtualKeyForm
                  key={vk.id}
                  mode="edit"
                  existingKey={vk}
                  onSave={() => {
                    setEditingId(null);
                    invalidateKeys();
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
