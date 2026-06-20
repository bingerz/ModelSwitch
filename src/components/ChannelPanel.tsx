import { useEffect, useState } from "react";
import { api, type Channel, PRIORITY_TIERS, validateChannelForm, invokeTauri } from "../lib/api";
import { useQuota } from "../hooks/useQuota";
import type { QuotaInfo } from "../lib/api";
import { useToast } from "./Toast";
import {
  type ProviderPreset,
  type PresetCategory,
  type ApiFormat,
  CATEGORY_LABELS,
  groupPresetsByCategory,
} from "../lib/presets";

type ChannelStatus = "healthy" | "circuit_open" | "disabled";

/** Derive API format from the channel's provider string */
function providerToFormat(provider: string): ApiFormat {
  if (provider === "anthropic") return "anthropic";
  if (provider === "gemini") return "gemini";
  return "openai";
}

const STATUS_DOT: Record<ChannelStatus, string> = {
  healthy: "var(--color-success)",
  circuit_open: "var(--color-danger)",
  disabled: "var(--color-text-muted)",
};

/** Format ISO timestamp into a human-readable recovery countdown. */
function formatRecoveryTime(isoUntil: string): string {
  const until = new Date(isoUntil);
  const now = new Date();
  const diffMs = until.getTime() - now.getTime();
  if (diffMs <= 0) return "recovering";
  const mins = Math.floor(diffMs / 60000);
  const secs = Math.floor((diffMs % 60000) / 1000);
  if (mins > 0) return `${mins}m ${secs}s`;
  return `${secs}s`;
}

/** Compact quota badge for inline display in channel cards. */
function QuotaBadge({ quota }: { quota: QuotaInfo | undefined }) {
  if (!quota || quota.error) return null;

  const hasBalance = quota.balance != null;
  const hasRateLimit = quota.rate_limit_remaining_req != null;

  if (!hasBalance && !hasRateLimit) return null;

  // Balance badge
  if (hasBalance) {
    const pct = quota.limit != null && quota.limit > 0
      ? ((quota.limit - quota.balance!) / quota.limit) * 100
      : null;
    const color = pct != null
      ? (pct > 80 ? "var(--color-danger)" : pct > 50 ? "var(--color-warning)" : "var(--color-success)")
      : "var(--color-success)";
    return (
      <span className="meta-tag mono" style={{ color, fontSize: "0.7rem" }}>
        ${quota.balance!.toFixed(2)}
      </span>
    );
  }

  // Rate-limit badge
  if (hasRateLimit && quota.rate_limit_limit_req != null) {
    return (
      <span className="meta-tag mono" style={{ fontSize: "0.7rem" }}>
        {quota.rate_limit_remaining_req}/{quota.rate_limit_limit_req} RPM
      </span>
    );
  }

  return null;
}

const CATEGORY_ORDER: PresetCategory[] = [
  "official",
  "cn_official",
  "aggregator",
  "cloud_provider",
  "third_party",
];

export function ChannelPanel() {
  const toast = useToast();
  const [showAddForm, setShowAddForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const { quotas, channels, refresh, loading } = useQuota();

  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<"all" | "healthy" | "circuit_open" | "disabled">("all");

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
    // Status filter
    if (statusFilter !== "all") {
      if (statusFilter === "healthy" && ch.status !== "healthy") return false;
      if (statusFilter === "circuit_open" && ch.status !== "circuit_open") return false;
      if (statusFilter === "disabled" && ch.status !== "disabled") return false;
    }
    // Search filter (match name or provider, case-insensitive)
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      return ch.name.toLowerCase().includes(q) || ch.provider.toLowerCase().includes(q);
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
            <button className="channel-search-clear" onClick={() => setSearchQuery("")} title="Clear search">×</button>
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
        const meta = PRIORITY_TIERS[priority] || { label: `Priority ${priority}`, desc: "" };
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
                    <div
                      key={ch.id}
                      className={`channel-card ${ch.status === "circuit_open" ? "channel-card-warning" : ""} ${!ch.enabled ? "channel-card-disabled" : ""}`}
                      draggable
                      onDragStart={() => handleDragStart(ch.id)}
                      onDragEnd={() => setDragId(null)}
                    >
                      <div className="channel-card-header">
                        <div className="channel-card-title">
                          <span
                            className={`status-dot ${ch.status === "healthy" ? "healthy" : ""}`}
                            style={{ background: STATUS_DOT[ch.status] || STATUS_DOT.disabled }}
                          />
                          <strong>{ch.name}</strong>
                        </div>
                        <span className="channel-provider channel-provider-badge" style={providerBadgeStyle(ch.provider)}>
                          {ch.provider}
                        </span>
                        {!ch.enabled && <span className="channel-disabled-tag">Disabled</span>}
                        {ch.status === "circuit_open" && (
                          <span className="channel-circuit-tag">
                            Circuit Open{ch.circuit_open_until ? ` · ${formatRecoveryTime(ch.circuit_open_until)}` : ""}
                          </span>
                        )}
                      </div>
                      <div className="channel-card-meta">
                        <span
                          className="api-format-card-badge"
                          style={{ color: API_FORMAT_COLORS[providerToFormat(ch.provider)] }}
                        >
                          {API_FORMAT_LABELS[providerToFormat(ch.provider)]}
                        </span>
                        <span className="meta-tag">W:{ch.weight}</span>
                        {Object.keys(ch.model_mapping).length > 0 && (
                          <span className="meta-tag">{Object.keys(ch.model_mapping).length} models</span>
                        )}
                        {ch.avg_latency_ms > 0 && (
                          <span className="meta-tag">{Math.round(ch.avg_latency_ms)}ms</span>
                        )}
                        <QuotaBadge quota={quotaMap.get(ch.id)} />
                      </div>
                      <div className="channel-card-actions">
                        <button className="btn btn-sm" onClick={() => handlePing(ch.id)}>
                          Ping
                        </button>
                        <button className="btn btn-sm" onClick={() => setEditingId(ch.id)}>
                          Edit
                        </button>
                        <button className="btn btn-sm" onClick={() => handleToggle(ch)}>
                          {ch.enabled ? "Disable" : "Enable"}
                        </button>
                        <button
                          className={`btn btn-sm ${confirmDeleteId === ch.id ? "btn-danger" : ""}`}
                          style={confirmDeleteId !== ch.id ? { color: "var(--color-danger)" } : undefined}
                          onClick={() => handleDelete(ch.id)}
                          onBlur={() => setConfirmDeleteId(null)}
                        >
                          {confirmDeleteId === ch.id ? "Confirm?" : "Delete"}
                        </button>
                      </div>
                    </div>
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

// ─── Preset Selector ───

function PresetSelector({
  selected,
  onSelect,
}: {
  selected: string | null;
  onSelect: (preset: ProviderPreset) => void;
}) {
  const grouped = groupPresetsByCategory();

  return (
    <div className="preset-selector">
      <p className="preset-hint">Select a provider preset to auto-fill, or enter manually below.</p>
      {CATEGORY_ORDER.map((cat) => {
        const presets = grouped[cat];
        if (presets.length === 0) return null;
        return (
          <div key={cat} className="preset-section">
            <div className="preset-section-title">{CATEGORY_LABELS[cat]}</div>
            <div className="preset-grid">
              {presets.map((p) => (
                <button
                  key={p.name}
                  type="button"
                  className={`preset-card ${selected === p.name ? "selected" : ""}`}
                  onClick={() => onSelect(p)}
                  title={p.baseUrl}
                >
                  <span
                    className="preset-dot"
                    style={{ background: p.iconColor }}
                  />
                  <span className="preset-name">{p.name}</span>
                </button>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}

/**
 * Returns the set of API formats a preset supports.
 * Always includes the preset's apiFormat, plus any from endpoints.
 */
function getAvailableFormats(preset: ProviderPreset): ApiFormat[] {
  const formats = new Set<ApiFormat>();
  if (preset.apiFormat) formats.add(preset.apiFormat);
  if (preset.endpoints) {
    for (const fmt of Object.keys(preset.endpoints) as ApiFormat[]) {
      formats.add(fmt);
    }
  }
  if (formats.size === 0) formats.add("openai");
  return Array.from(formats);
}

const API_FORMAT_COLORS: Record<ApiFormat, string> = {
  openai: "#10A37F",
  anthropic: "#D97757",
  gemini: "#4285F4",
};

const PROVIDER_BADGES: Record<string, { bg: string; color: string }> = {
  openai: { bg: "rgba(16, 163, 127, 0.15)", color: "#10A37F" },
  anthropic: { bg: "rgba(217, 119, 87, 0.15)", color: "#D97757" },
  deepseek: { bg: "rgba(99, 102, 241, 0.15)", color: "#6366F1" },
  gemini: { bg: "rgba(66, 133, 244, 0.15)", color: "#4285F4" },
  openrouter: { bg: "rgba(168, 85, 247, 0.15)", color: "#A855F7" },
};

function providerBadgeStyle(provider: string): React.CSSProperties {
  const badge = PROVIDER_BADGES[provider];
  if (badge) return { backgroundColor: badge.bg, color: badge.color };
  return { backgroundColor: "var(--color-surface-hover)", color: "var(--color-text-secondary)" };
}

const API_FORMAT_LABELS: Record<ApiFormat, string> = {
  openai: "OpenAI Chat",
  anthropic: "Anthropic",
  gemini: "Gemini",
};

// ─── Channel Form with Preset Selector ───

function ChannelForm({ onSave }: { onSave: () => void }) {
  const toast = useToast();
  const [selectedPreset, setSelectedPreset] = useState<string | null>(null);
  const [activePreset, setActivePreset] = useState<ProviderPreset | null>(null);
  const [name, setName] = useState("");
  const [provider, setProvider] = useState("openai");
  const [priority, setPriority] = useState(1);
  const [weight, setWeight] = useState(100);
  const [costPerToken, setCostPerToken] = useState("");
  const [credentialType, setCredentialType] = useState("api_key");
  const [credentialValue, setCredentialValue] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [modelMapping, setModelMapping] = useState<Record<string, string>>({});
  const [cooldownMinutes, setCooldownMinutes] = useState("");
  const [loginInProgress, setLoginInProgress] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [inputCostPerMtok, setInputCostPerMtok] = useState("");
  const [outputCostPerMtok, setOutputCostPerMtok] = useState("");
  const [rpmLimit, setRpmLimit] = useState("");
  const [tpmLimit, setTpmLimit] = useState("");
  const [apiFormat, setApiFormat] = useState<ApiFormat>("anthropic");

  // Listen for WebView login cookies (safe lazy import)
  useEffect(() => {
    let destroyed = false;
    let unlistenFn: (() => void) | null = null;

    import("@tauri-apps/api/event").then(({ listen }) => {
      if (destroyed) return;
      listen("login-cookies-received", (event: any) => {
        const { cookies } = event.payload;
        if (cookies) {
          setCredentialValue(cookies);
        }
        setLoginInProgress(false);
      }).then((fn) => {
        unlistenFn = fn;
      });
    }).catch(() => {
      // Tauri API not available (running in browser)
    });

    return () => {
      destroyed = true;
      unlistenFn?.();
    };
  }, []);

  const handleWebViewLogin = async () => {
    setLoginInProgress(true);
    try {
      await invokeTauri("open_login_webview", { provider });
    } catch (e) {
      toast.error(`WebView login failed: ${e}`);
      setLoginInProgress(false);
    }
  };

  const handlePresetSelect = (preset: ProviderPreset) => {
    const isSelected = selectedPreset === preset.name;
    if (isSelected) {
      // Deselect
      setSelectedPreset(null);
      setActivePreset(null);
      setName("");
      setProvider("openai");
      setPriority(1);
      setBaseUrl("");
      setModelMapping({});
      setApiFormat("anthropic");
    } else {
      // Apply preset
      setSelectedPreset(preset.name);
      setActivePreset(preset);
      setName(preset.name);
      setProvider(preset.provider);
      setPriority(preset.priority);
      // Initialize API format — prefer anthropic, then preset's apiFormat, then first available
      const formats = getAvailableFormats(preset);
      let defaultFormat = formats[0];
      if (formats.includes("anthropic")) defaultFormat = "anthropic";
      else if (preset.apiFormat && formats.includes(preset.apiFormat)) defaultFormat = preset.apiFormat;
      setApiFormat(defaultFormat);
      // Set baseUrl for the selected format
      const url = preset.endpoints?.[defaultFormat] ?? preset.baseUrl;
      setBaseUrl(url);
      // Ensure defaultModel is in the mapping
      const mapping = { ...preset.modelMapping };
      if (preset.defaultModel && !mapping[preset.defaultModel]) {
        mapping[preset.defaultModel] = preset.defaultModel;
      }
      setModelMapping(mapping);
    }
  };

  const handleApiFormatChange = (format: ApiFormat) => {
    setApiFormat(format);
    if (activePreset) {
      const url = activePreset.endpoints?.[format] ?? activePreset.baseUrl;
      setBaseUrl(url);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    const validationError = validateChannelForm({ name, baseUrl });
    if (validationError) {
      setError(validationError);
      return;
    }
    if (costPerToken && parseFloat(costPerToken) <= 0) {
      setError("Cost per token must be a positive number");
      return;
    }
    if (!credentialValue.trim() && credentialType === "api_key") {
      setError("API key is required");
      return;
    }
    setSubmitting(true);
    try {
      await api.createChannel({
        name,
        provider,
        priority,
        weight,
        cost_per_token: costPerToken ? parseFloat(costPerToken) : null,
        credential_type: credentialType,
        credential_value: credentialValue,
        base_url: baseUrl,
        model_mapping: modelMapping,
      });
      toast.success("Channel created");
      onSave();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Failed to create channel";
      setError(msg);
      toast.error(msg);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form className="channel-form" onSubmit={handleSubmit}>
      <h3 className="form-title">Add Channel</h3>

      <PresetSelector selected={selectedPreset} onSelect={handlePresetSelect} />

      <div className="form-divider"><span>Or configure manually</span></div>

      <FormFields
        name={name} setName={setName}
        provider={provider} setProvider={setProvider}
        priority={priority} setPriority={setPriority}
        weight={weight} setWeight={setWeight}
        costPerToken={costPerToken} setCostPerToken={setCostPerToken}
        credentialType={credentialType} setCredentialType={setCredentialType}
        credentialValue={credentialValue} setCredentialValue={setCredentialValue}
        baseUrl={baseUrl} setBaseUrl={setBaseUrl}
        cooldownMinutes={cooldownMinutes} setCooldownMinutes={setCooldownMinutes}
        inputCostPerMtok={inputCostPerMtok} setInputCostPerMtok={setInputCostPerMtok}
        outputCostPerMtok={outputCostPerMtok} setOutputCostPerMtok={setOutputCostPerMtok}
        rpmLimit={rpmLimit} setRpmLimit={setRpmLimit}
        tpmLimit={tpmLimit} setTpmLimit={setTpmLimit}
        presetModels={activePreset?.models ?? []}
        modelMapping={modelMapping} setModelMapping={setModelMapping}
        apiKeyUrl={activePreset?.apiKeyUrl}
        defaultModel={activePreset?.defaultModel}
        showCredential={true}
        onWebViewLogin={handleWebViewLogin}
        loginInProgress={loginInProgress}
        presetLocked={selectedPreset !== null}
        apiFormat={apiFormat}
        apiFormats={activePreset ? getAvailableFormats(activePreset) : ["openai"]}
        onApiFormatChange={handleApiFormatChange}
      />
      {error && <div className="form-error">{error}</div>}
      <button type="submit" className="btn btn-primary" disabled={submitting}>
        {submitting ? "Creating..." : "Create Channel"}
      </button>
    </form>
  );
}

function EditChannelForm({
  channel,
  onSave,
  onCancel,
}: {
  channel: Channel;
  onSave: () => void;
  onCancel: () => void;
}) {
  const toast = useToast();
  const [name, setName] = useState(channel.name);
  const [provider, setProvider] = useState(channel.provider);
  const [priority, setPriority] = useState(channel.priority);
  const [weight, setWeight] = useState(channel.weight);
  const [costPerToken, setCostPerToken] = useState(
    channel.cost_per_token != null ? String(channel.cost_per_token) : ""
  );
  const [baseUrl, setBaseUrl] = useState(channel.base_url);
  const [modelMapping, setModelMapping] = useState<Record<string, string>>(channel.model_mapping);
  const [cooldownMinutes, setCooldownMinutes] = useState(
    channel.cooldown_minutes != null ? String(channel.cooldown_minutes) : ""
  );
  const [inputCostPerMtok, setInputCostPerMtok] = useState(
    channel.input_cost_per_mtok != null ? String(channel.input_cost_per_mtok) : ""
  );
  const [outputCostPerMtok, setOutputCostPerMtok] = useState(
    channel.output_cost_per_mtok != null ? String(channel.output_cost_per_mtok) : ""
  );
  const [rpmLimit, setRpmLimit] = useState(
    channel.rpm_limit != null ? String(channel.rpm_limit) : ""
  );
  const [tpmLimit, setTpmLimit] = useState(
    channel.tpm_limit != null ? String(channel.tpm_limit) : ""
  );
  const [credentialType, setCredentialType] = useState("api_key");
  const [credentialValue, setCredentialValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    const validationError = validateChannelForm({ name, baseUrl });
    if (validationError) {
      setError(validationError);
      return;
    }
    setSubmitting(true);
    try {
      await api.updateChannel(channel.id, {
        name,
        provider,
        priority,
        weight,
        cost_per_token: costPerToken ? parseFloat(costPerToken) : null,
        base_url: baseUrl,
        enabled: channel.enabled,
        model_mapping: modelMapping,
        cooldown_minutes: cooldownMinutes ? parseInt(cooldownMinutes, 10) : null,
        credential_type: credentialValue ? credentialType : undefined,
        credential_value: credentialValue || undefined,
      });
      toast.success("Channel updated");
      onSave();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Failed to update channel";
      setError(msg);
      toast.error(msg);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form className="channel-form" onSubmit={handleSubmit}>
      <h3 className="form-title">Edit Channel</h3>
      <FormFields
        name={name} setName={setName}
        provider={provider} setProvider={setProvider}
        priority={priority} setPriority={setPriority}
        weight={weight} setWeight={setWeight}
        costPerToken={costPerToken} setCostPerToken={setCostPerToken}
        credentialType={credentialType} setCredentialType={setCredentialType}
        credentialValue={credentialValue} setCredentialValue={setCredentialValue}
        baseUrl={baseUrl} setBaseUrl={setBaseUrl}
        cooldownMinutes={cooldownMinutes} setCooldownMinutes={setCooldownMinutes}
        inputCostPerMtok={inputCostPerMtok} setInputCostPerMtok={setInputCostPerMtok}
        outputCostPerMtok={outputCostPerMtok} setOutputCostPerMtok={setOutputCostPerMtok}
        rpmLimit={rpmLimit} setRpmLimit={setRpmLimit}
        tpmLimit={tpmLimit} setTpmLimit={setTpmLimit}
        presetModels={[]}
        modelMapping={modelMapping} setModelMapping={setModelMapping}
        showCredential={true}
        credentialPlaceholder="Leave empty to keep current credential"
      />
      {error && <div className="form-error">{error}</div>}
      <div style={{ display: "flex", gap: "var(--space-2)" }}>
        <button type="submit" className="btn btn-primary" disabled={submitting}>
          {submitting ? "Saving..." : "Save"}
        </button>
        <button type="button" className="btn" onClick={onCancel} disabled={submitting}>Cancel</button>
      </div>
    </form>
  );
}

function ModelSelector({
  availableModels,
  modelMapping,
  onModelMappingChange,
  defaultModel,
}: {
  availableModels: string[];
  modelMapping: Record<string, string>;
  onModelMappingChange: (mapping: Record<string, string>) => void;
  defaultModel?: string;
}) {
  const [customModel, setCustomModel] = useState("");

  const toggleModel = (model: string) => {
    const next = { ...modelMapping };
    if (next[model] !== undefined) {
      delete next[model];
    } else {
      next[model] = model;
    }
    onModelMappingChange(next);
  };

  const addCustomModel = () => {
    const trimmed = customModel.trim();
    if (!trimmed || modelMapping[trimmed] !== undefined) return;
    onModelMappingChange({ ...modelMapping, [trimmed]: trimmed });
    setCustomModel("");
  };

  const selectedModels = Object.keys(modelMapping);

  return (
    <div className="model-selector">
      {availableModels.length > 0 && (
        <div className="model-checkboxes">
          {availableModels.map((model) => (
            <label key={model} className="model-checkbox">
              <input
                type="checkbox"
                checked={modelMapping[model] !== undefined}
                onChange={() => toggleModel(model)}
              />
              <span>
                {model}
                {defaultModel && model === defaultModel && (
                  <span className="model-default-indicator">default</span>
                )}
              </span>
            </label>
          ))}
        </div>
      )}
      {availableModels.length === 0 && selectedModels.length === 0 && (
        <p className="model-hint">Select a preset above, or add custom model IDs below.</p>
      )}
      <div className="model-custom-row">
        <input
          value={customModel}
          onChange={(e) => setCustomModel(e.target.value)}
          placeholder="Add custom model ID"
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addCustomModel();
            }
          }}
        />
        <button type="button" className="btn btn-sm" onClick={addCustomModel}>
          Add
        </button>
      </div>
      {selectedModels.length > 0 && (
        <div className="model-tags">
          {selectedModels.map((model) => (
            <span key={model} className="model-tag">
              {model}
              <button
                type="button"
                className="model-tag-remove"
                onClick={() => toggleModel(model)}
                title={`Remove ${model}`}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

function FormFields({
  name, setName,
  provider, setProvider,
  priority, setPriority,
  weight, setWeight,
  costPerToken, setCostPerToken,
  credentialType, setCredentialType,
  credentialValue, setCredentialValue,
  baseUrl, setBaseUrl,
  cooldownMinutes, setCooldownMinutes,
  inputCostPerMtok, setInputCostPerMtok,
  outputCostPerMtok, setOutputCostPerMtok,
  rpmLimit, setRpmLimit,
  tpmLimit, setTpmLimit,
  presetModels,
  modelMapping, setModelMapping,
  apiKeyUrl,
  defaultModel,
  showCredential,
  onWebViewLogin,
  loginInProgress,
  presetLocked,
  credentialPlaceholder,
  apiFormat,
  apiFormats,
  onApiFormatChange,
}: {
  name: string; setName: (v: string) => void;
  provider: string; setProvider: (v: string) => void;
  priority: number; setPriority: (v: number) => void;
  weight: number; setWeight: (v: number) => void;
  costPerToken: string; setCostPerToken: (v: string) => void;
  credentialType: string; setCredentialType: (v: string) => void;
  credentialValue: string; setCredentialValue: (v: string) => void;
  baseUrl: string; setBaseUrl: (v: string) => void;
  cooldownMinutes: string; setCooldownMinutes: (v: string) => void;
  inputCostPerMtok: string; setInputCostPerMtok: (v: string) => void;
  outputCostPerMtok: string; setOutputCostPerMtok: (v: string) => void;
  rpmLimit: string; setRpmLimit: (v: string) => void;
  tpmLimit: string; setTpmLimit: (v: string) => void;
  presetModels: string[];
  modelMapping: Record<string, string>; setModelMapping: (v: Record<string, string>) => void;
  apiKeyUrl?: string;
  defaultModel?: string;
  showCredential: boolean;
  onWebViewLogin?: () => void;
  loginInProgress?: boolean;
  presetLocked?: boolean;
  credentialPlaceholder?: string;
  apiFormat?: ApiFormat;
  apiFormats?: ApiFormat[];
  onApiFormatChange?: (format: ApiFormat) => void;
}) {
  return (
    <div className="form-grid">
      <label className="form-field">
        <span>Name</span>
        <input value={name} onChange={(e) => setName(e.target.value)} required />
      </label>
      <label className="form-field">
        <span>Provider{presetLocked ? " · preset" : ""}</span>
        <select value={provider} onChange={(e) => setProvider(e.target.value)} disabled={presetLocked}>
          <option value="openai">OpenAI</option>
          <option value="anthropic">Anthropic</option>
          <option value="deepseek">DeepSeek</option>
          <option value="gemini">Gemini</option>
          <option value="openrouter">OpenRouter</option>
          <option value="custom">Custom</option>
        </select>
      </label>
      <label className="form-field">
        <span>Priority</span>
        <select value={priority} onChange={(e) => setPriority(Number(e.target.value))}>
          <option value={1}>Priority 1 · Free / Subscription</option>
          <option value={2}>Priority 2 · Economy API</option>
          <option value={3}>Priority 3 · Official API</option>
        </select>
      </label>
      <label className="form-field">
        <span>Weight</span>
        <input
          type="number"
          value={weight}
          onChange={(e) => setWeight(Number(e.target.value))}
          min={1}
          max={1000}
        />
      </label>
      <label className="form-field">
        <span>Cost per 1K tokens ($)</span>
        <input
          type="number"
          step="0.0001"
          value={costPerToken}
          onChange={(e) => setCostPerToken(e.target.value)}
          placeholder="e.g. 0.0015"
        />
      </label>
      {showCredential && (
        <>
          <label className="form-field">
            <span>Credential Type</span>
            <select value={credentialType} onChange={(e) => setCredentialType(e.target.value)}>
              <option value="api_key">API Key</option>
              <option value="web_session">Web Session</option>
            </select>
          </label>
          <label className="form-field">
            <span>{credentialType === "api_key" ? "API Key" : "Cookie"}</span>
            <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "center" }}>
              <input
                type="password"
                value={credentialValue}
                onChange={(e) => setCredentialValue(e.target.value)}
                placeholder={credentialPlaceholder || (credentialType === "api_key" ? "Enter API key" : "Enter cookie")}
                style={{ flex: 1 }}
              />
              {apiKeyUrl && (
                <a
                  href={apiKeyUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="btn btn-sm"
                  title="Get API key"
                >
                  Get Key
                </a>
              )}
              {credentialType === "web_session" && onWebViewLogin && (
                <button
                  type="button"
                  className="btn btn-sm btn-primary"
                  onClick={onWebViewLogin}
                  disabled={loginInProgress}
                  title="Open login window to extract cookies"
                >
                  {loginInProgress ? "Logging in..." : "Login"}
                </button>
              )}
            </div>
          </label>
        </>
      )}
      <label className="form-field">
        <span>Cooldown (minutes)</span>
        <input
          type="number"
          value={cooldownMinutes}
          onChange={(e) => setCooldownMinutes(e.target.value)}
          placeholder="default (30)"
          min={1}
        />
      </label>
      <label className="form-field">
        <span>Input cost per 1M tokens ($)</span>
        <input
          type="number"
          step="0.0001"
          value={inputCostPerMtok}
          onChange={(e) => setInputCostPerMtok(e.target.value)}
          placeholder="e.g. 0.0015"
        />
      </label>
      <label className="form-field">
        <span>Output cost per 1M tokens ($)</span>
        <input
          type="number"
          step="0.0001"
          value={outputCostPerMtok}
          onChange={(e) => setOutputCostPerMtok(e.target.value)}
          placeholder="e.g. 0.0075"
        />
      </label>
      <label className="form-field">
        <span>RPM Limit</span>
        <input
          type="number"
          value={rpmLimit}
          onChange={(e) => setRpmLimit(e.target.value)}
          placeholder="default (60)"
          min={1}
        />
      </label>
      <label className="form-field">
        <span>TPM Limit</span>
        <input
          type="number"
          value={tpmLimit}
          onChange={(e) => setTpmLimit(e.target.value)}
          placeholder="no limit"
          min={1}
        />
      </label>
      {apiFormats && onApiFormatChange && (
        <label className="form-field">
          <span>API Format</span>
          <select
            value={apiFormat}
            onChange={(e) => onApiFormatChange(e.target.value as ApiFormat)}
            disabled={apiFormats.length <= 1}
          >
            {apiFormats.map((fmt) => (
              <option key={fmt} value={fmt}>
                {fmt === "openai" ? "OpenAI Chat" : fmt === "anthropic" ? "Anthropic" : "Gemini"}
              </option>
            ))}
          </select>
        </label>
      )}
      <label className="form-field">
        <span>Base URL</span>
        <input
          value={baseUrl}
          onChange={(e) => setBaseUrl(e.target.value)}
          placeholder="https://api.openai.com"
          required
        />
      </label>
      <div className="form-field span-2">
        <span>Models</span>
        <ModelSelector
          availableModels={presetModels}
          modelMapping={modelMapping}
          onModelMappingChange={setModelMapping}
          defaultModel={defaultModel}
        />
      </div>
    </div>
  );
}
