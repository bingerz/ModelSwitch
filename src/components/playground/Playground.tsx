import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  FlaskConical,
  Play,
  Eye,
  EyeOff,
  RefreshCw,
  AlertTriangle,
  Clock,
  Cpu,
  Coins,
  Bot,
  Send,
} from "lucide-react";
import { API_BASE } from "../../lib/runtime";
import { buildAuthHeaders } from "../../lib/api/client";
import { SectionHeader } from "../ui/SectionHeader";
import { EmptyState } from "../ui/EmptyState";
import "../../styles/playground.css";

/** Metadata captured from each request — used to render stat tiles. */
interface ResponseMetadata {
  latency_ms: number;
  model: string;
  prompt_tokens?: number;
  completion_tokens?: number;
  total_tokens?: number;
}

/** Shape of a single chat message sent to the OpenAI-compatible endpoint. */
interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

/** OpenAI /v1/models list response shape (only fields we consume). */
interface ModelsResponse {
  data?: Array<{ id: string }>;
}

/** OpenAI /v1/chat/completions response shape (only fields we consume). */
interface ChatCompletionResponse {
  model?: string;
  choices?: Array<{ message?: { content?: string } }>;
  usage?: {
    prompt_tokens?: number;
    completion_tokens?: number;
    total_tokens?: number;
  };
}

/** sessionStorage key used to persist the virtual key between sessions. */
const VK_STORAGE_KEY = "playground_vk";

/** Lower bound for max_tokens — keep requests well-formed. */
const MIN_MAX_TOKENS = 1;
/** Upper bound for max_tokens — sane ceiling aligned with the spec. */
const MAX_MAX_TOKENS = 8192;
/** Temperature step for the slider input. */
const TEMPERATURE_STEP = 0.1;

const DEFAULT_TEMPERATURE = 0.7;
const DEFAULT_MAX_TOKENS = 1024;

/**
 * Resolve the bearer token to send on a playground request.
 * Returns null when no virtual key is entered, so the Authorization
 * header is omitted entirely (open-proxy mode) or the proxy returns
 * 401 (virtual-key mode). Never falls back to the admin token.
 */
function resolveToken(virtualKey: string): string | null {
  return virtualKey.trim() || null;
}

export function Playground() {
  const { t } = useTranslation();

  const [virtualKey, setVirtualKey] = useState<string>(
    () => sessionStorage.getItem(VK_STORAGE_KEY) ?? "",
  );
  const [showKey, setShowKey] = useState(false);
  const [model, setModel] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [userMessage, setUserMessage] = useState("");
  const [temperature, setTemperature] = useState(DEFAULT_TEMPERATURE);
  const [maxTokens, setMaxTokens] = useState(DEFAULT_MAX_TOKENS);
  const [stream, setStream] = useState(false);

  const [models, setModels] = useState<string[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);

  const [loading, setLoading] = useState(false);
  const [response, setResponse] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [metadata, setMetadata] = useState<ResponseMetadata | null>(null);

  // Persist virtual key whenever it changes.
  useEffect(() => {
    sessionStorage.setItem(VK_STORAGE_KEY, virtualKey);
  }, [virtualKey]);

  const fetchModels = useCallback(async () => {
    setLoadingModels(true);
    setModelsError(null);
    try {
      const token = resolveToken(virtualKey);
      const res = await fetch(`${API_BASE}/v1/models`, {
        headers: buildAuthHeaders(token),
      });
      if (!res.ok) {
        const body = await res.text().catch(() => "");
        throw new Error(`HTTP ${res.status}${body ? `: ${body}` : ""}`);
      }
      const data = (await res.json()) as ModelsResponse;
      const ids = (data.data ?? []).map((m) => m.id).sort();
      setModels(ids);
      // Auto-select the first model if none is chosen yet.
      setModel((prev) => prev || ids[0] || "");
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setModelsError(message);
      setModels([]);
    } finally {
      setLoadingModels(false);
    }
  }, [virtualKey]);

  // Load models on mount.
  useEffect(() => {
    void fetchModels();
  }, [fetchModels]);

  const sendDisabled = loading || userMessage.trim().length === 0 || !model;

  const sendRequest = useCallback(async () => {
    if (sendDisabled) return;
    setLoading(true);
    setError(null);
    setResponse("");
    setMetadata(null);

    const startTime = performance.now();
    try {
      const token = resolveToken(virtualKey);
      const messages: ChatMessage[] = [];
      const sys = systemPrompt.trim();
      if (sys) messages.push({ role: "system", content: sys });
      messages.push({ role: "user", content: userMessage });

      const res = await fetch(`${API_BASE}/v1/chat/completions`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...buildAuthHeaders(token),
        },
        body: JSON.stringify({
          model,
          messages,
          temperature,
          max_tokens: maxTokens,
          stream,
        }),
      });

      if (!res.ok) {
        const errBody = await res.text().catch(() => "");
        throw new Error(`HTTP ${res.status}${errBody ? `: ${errBody}` : ""}`);
      }

      if (stream) {
        // SSE streaming — accumulate delta content progressively.
        const reader = res.body?.getReader();
        if (!reader) throw new Error("No response body for streaming");
        const decoder = new TextDecoder();
        let accumulated = "";
        let buffer = "";
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split("\n");
          buffer = lines.pop() ?? "";
          for (const line of lines) {
            const trimmed = line.trim();
            if (!trimmed.startsWith("data: ")) continue;
            const payload = trimmed.slice(6);
            if (payload === "[DONE]") continue;
            try {
              const chunk = JSON.parse(payload);
              const delta = chunk.choices?.[0]?.delta?.content;
              if (delta) {
                accumulated += delta;
                setResponse(accumulated);
              }
            } catch { /* skip malformed SSE chunk */ }
          }
        }
        setMetadata({ latency_ms: Math.round(performance.now() - startTime), model });
      } else {
        const data = (await res.json()) as ChatCompletionResponse;
        const content =
          data.choices?.[0]?.message?.content ?? JSON.stringify(data, null, 2);
        setResponse(content);
        setMetadata({
          latency_ms: Math.round(performance.now() - startTime),
          model: data.model ?? model,
          prompt_tokens: data.usage?.prompt_tokens,
          completion_tokens: data.usage?.completion_tokens,
          total_tokens: data.usage?.total_tokens,
        });
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
    } finally {
      setLoading(false);
    }
  }, [sendDisabled, virtualKey, systemPrompt, userMessage, model, temperature, maxTokens, stream]);

  const subtitle = useMemo(() => t("playground.subtitle"), [t]);

  return (
    <section className="playground-section">
      <SectionHeader
        title={t("playground.title")}
        icon={FlaskConical}
        onRefresh={() => void fetchModels()}
        refreshing={loadingModels}
      />
      <p className="playground-subtitle">{subtitle}</p>

      <div className="playground-container">
        {/* Left column — configuration + input */}
        <div className="playground-panel playground-panel-input">
          {/* Virtual key */}
          <div className="playground-field">
            <label className="playground-label" htmlFor="pg-vk">
              {t("playground.virtualKey")}
            </label>
            <div className="playground-input-row">
              <input
                id="pg-vk"
                type={showKey ? "text" : "password"}
                className="playground-input"
                value={virtualKey}
                onChange={(e) => setVirtualKey(e.target.value)}
                placeholder="ms-vk-..."
                autoComplete="off"
                spellCheck={false}
              />
              <button
                type="button"
                className="playground-icon-btn"
                onClick={() => setShowKey((s) => !s)}
                title={showKey ? t("common.dismiss") : t("common.moreActions")}
                aria-label={showKey ? "Hide key" : "Show key"}
              >
                {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
            <span className="playground-help">{t("playground.virtualKeyHelp")}</span>
          </div>

          {/* Model selector */}
          <div className="playground-field">
            <label className="playground-label" htmlFor="pg-model">
              {t("playground.model")}
            </label>
            <div className="playground-input-row">
              <select
                id="pg-model"
                className="playground-input playground-select"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                disabled={loadingModels || models.length === 0}
              >
                {models.length === 0 && (
                  <option value="">
                    {loadingModels ? t("playground.fetchingModels") : t("playground.noModels")}
                  </option>
                )}
                {models.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="playground-icon-btn"
                onClick={() => void fetchModels()}
                disabled={loadingModels}
                title={t("common.refresh")}
                aria-label={t("common.refresh")}
              >
                <RefreshCw size={14} className={loadingModels ? "ui-spin" : ""} />
              </button>
            </div>
            {modelsError && (
              <span className="playground-error-text">{modelsError}</span>
            )}
            {!modelsError && models.length === 0 && !loadingModels && (
              <span className="playground-help">{t("playground.noModels")}</span>
            )}
          </div>

          {/* System prompt */}
          <div className="playground-field">
            <label className="playground-label" htmlFor="pg-system">
              {t("playground.systemPrompt")}
            </label>
            <textarea
              id="pg-system"
              className="playground-input playground-textarea"
              rows={3}
              value={systemPrompt}
              onChange={(e) => setSystemPrompt(e.target.value)}
              placeholder="You are a helpful assistant..."
              spellCheck={false}
            />
          </div>

          {/* User message */}
          <div className="playground-field">
            <label className="playground-label" htmlFor="pg-user">
              {t("playground.userMessage")}
            </label>
            <textarea
              id="pg-user"
              className="playground-input playground-textarea playground-textarea-lg"
              rows={6}
              value={userMessage}
              onChange={(e) => setUserMessage(e.target.value)}
              placeholder={t("playground.userMessage")}
              spellCheck={false}
            />
          </div>

          {/* Controls row */}
          <div className="playground-controls">
            <div className="playground-control">
              <label className="playground-label" htmlFor="pg-temp">
                {t("playground.temperature")}: <span className="mono">{temperature.toFixed(1)}</span>
              </label>
              <input
                id="pg-temp"
                type="range"
                min={0}
                max={2}
                step={TEMPERATURE_STEP}
                value={temperature}
                onChange={(e) => setTemperature(Number.parseFloat(e.target.value))}
                className="playground-slider"
              />
            </div>
            <div className="playground-control">
              <label className="playground-label" htmlFor="pg-max">
                {t("playground.maxTokens")}
              </label>
              <input
                id="pg-max"
                type="number"
                min={MIN_MAX_TOKENS}
                max={MAX_MAX_TOKENS}
                value={maxTokens}
                onChange={(e) => {
                  const n = Number.parseInt(e.target.value, 10);
                  if (Number.isFinite(n)) {
                    setMaxTokens(Math.min(MAX_MAX_TOKENS, Math.max(MIN_MAX_TOKENS, n)));
                  }
                }}
                className="playground-input playground-number"
              />
            </div>
            <div className="playground-control playground-control-inline">
              <input
                id="pg-stream"
                type="checkbox"
                checked={stream}
                onChange={(e) => setStream(e.target.checked)}
                className="playground-checkbox"
              />
              <label className="playground-label-inline" htmlFor="pg-stream">
                {t("playground.stream")}
              </label>
            </div>
          </div>

          {/* Send button */}
          <div className="playground-actions">
            <button
              type="button"
              className="btn btn-primary playground-send-btn"
              onClick={() => void sendRequest()}
              disabled={sendDisabled}
            >
              {loading ? (
                <>
                  <RefreshCw size={14} className="ui-spin" />
                  {t("playground.sending")}
                </>
              ) : (
                <>
                  <Send size={14} />
                  {t("playground.send")}
                </>
              )}
            </button>
          </div>
        </div>

        {/* Right column — response */}
        <div className="playground-panel playground-panel-response">
          <div className="playground-response-header">
            <Play size={16} className="playground-response-icon" />
            <h3 className="playground-response-title">{t("playground.response")}</h3>
          </div>

          {loading && (
            <div className="panel-loading-enhanced">
              <div className="spinner" />
              <span>{t("playground.sending")}</span>
            </div>
          )}

          {!loading && error && (
            <div className="playground-error-card" role="alert">
              <div className="playground-error-card-head">
                <AlertTriangle size={16} />
                <strong>{t("playground.error")}</strong>
              </div>
              <pre className="playground-error-body">{error}</pre>
            </div>
          )}

          {!loading && !error && response && (
            <>
              <pre className="playground-response-body">{response}</pre>
              {metadata && (
                <div className="playground-metadata">
                  <div className="playground-meta-tile">
                    <span className="playground-meta-icon">
                      <Clock size={14} />
                    </span>
                    <div className="playground-meta-body">
                      <div className="playground-meta-value">{metadata.latency_ms} ms</div>
                      <div className="playground-meta-label">{t("playground.latency")}</div>
                    </div>
                  </div>
                  <div className="playground-meta-tile">
                    <span className="playground-meta-icon">
                      <Cpu size={14} />
                    </span>
                    <div className="playground-meta-body">
                      <div className="playground-meta-value" title={metadata.model}>
                        {metadata.model}
                      </div>
                      <div className="playground-meta-label">{t("playground.model")}</div>
                    </div>
                  </div>
                  <div className="playground-meta-tile">
                    <span className="playground-meta-icon">
                      <Coins size={14} />
                    </span>
                    <div className="playground-meta-body">
                      <div className="playground-meta-value">
                        {metadata.total_tokens?.toLocaleString() ?? "—"}
                      </div>
                      <div className="playground-meta-label">{t("playground.totalTokens")}</div>
                    </div>
                  </div>
                  {typeof metadata.prompt_tokens === "number" && (
                    <div className="playground-meta-tile">
                      <span className="playground-meta-icon">
                        <Bot size={14} />
                      </span>
                      <div className="playground-meta-body">
                        <div className="playground-meta-value">
                          {metadata.prompt_tokens.toLocaleString()}
                        </div>
                        <div className="playground-meta-label">{t("playground.promptTokens")}</div>
                      </div>
                    </div>
                  )}
                  {typeof metadata.completion_tokens === "number" && (
                    <div className="playground-meta-tile">
                      <span className="playground-meta-icon">
                        <FlaskConical size={14} />
                      </span>
                      <div className="playground-meta-body">
                        <div className="playground-meta-value">
                          {metadata.completion_tokens.toLocaleString()}
                        </div>
                        <div className="playground-meta-label">
                          {t("playground.completionTokens")}
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              )}
            </>
          )}

          {!loading && !error && !response && (
            <EmptyState
              icon={Play}
              title={t("playground.noResponse")}
              description={t("playground.subtitle")}
            />
          )}
        </div>
      </div>
    </section>
  );
}
