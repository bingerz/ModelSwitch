import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
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
  CheckCircle2,
  Radio,
  KeyRound,
  Zap,
} from "lucide-react";
import { API_BASE } from "../../lib/runtime";
import { api, buildAuthHeaders } from "../../lib/api";
import {
  PLAYGROUND_VK_STORAGE_KEY,
} from "../../lib/playground";
import { useQuota } from "../../hooks/useQuota";
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

const MIN_MAX_TOKENS = 1;
const MAX_MAX_TOKENS = 8192;
const TEMPERATURE_STEP = 0.1;
const DEFAULT_TEMPERATURE = 0.7;
const DEFAULT_MAX_TOKENS = 256;

function resolveToken(virtualKey: string): string | null {
  return virtualKey.trim() || null;
}

export interface PlaygroundProps {
  /** Navigate to another primary tab (channels / virtualKeys / …). */
  onNavigate?: (tab: string) => void;
}

export function Playground({ onNavigate }: PlaygroundProps = {}) {
  const { t } = useTranslation();
  const { channels, loading: channelsLoading } = useQuota();

  const keysQuery = useQuery({
    queryKey: ["virtual-keys", "playground-ready"],
    queryFn: () => api.virtualKeys.list({ page: 1, limit: 1 }),
    staleTime: 15_000,
  });

  const [virtualKey, setVirtualKey] = useState<string>(
    () => sessionStorage.getItem(PLAYGROUND_VK_STORAGE_KEY) ?? "",
  );
  const [showKey, setShowKey] = useState(false);
  const [model, setModel] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [userMessage, setUserMessage] = useState("");
  const [temperature, setTemperature] = useState(DEFAULT_TEMPERATURE);
  const [maxTokens, setMaxTokens] = useState(DEFAULT_MAX_TOKENS);
  const [stream, setStream] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);

  const [models, setModels] = useState<string[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);

  const [loading, setLoading] = useState(false);
  const [response, setResponse] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [metadata, setMetadata] = useState<ResponseMetadata | null>(null);
  const [justSucceeded, setJustSucceeded] = useState(false);

  const userMessageRef = useRef(userMessage);
  userMessageRef.current = userMessage;
  const modelRef = useRef(model);
  modelRef.current = model;

  useEffect(() => {
    sessionStorage.setItem(PLAYGROUND_VK_STORAGE_KEY, virtualKey);
  }, [virtualKey]);

  // Pick up key stashed from the "test in Playground" CTA after key creation.
  useEffect(() => {
    const onFocus = () => {
      const stored = sessionStorage.getItem(PLAYGROUND_VK_STORAGE_KEY) ?? "";
      setVirtualKey((prev) => (stored && stored !== prev ? stored : prev));
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

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
      setModel((prev) => prev || ids[0] || "");
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setModelsError(message);
      setModels([]);
    } finally {
      setLoadingModels(false);
    }
  }, [virtualKey]);

  useEffect(() => {
    void fetchModels();
  }, [fetchModels]);

  const channelCount = channels.length;
  const keyTotal = keysQuery.data?.total ?? 0;
  const hasChannels = channelCount > 0;
  const hasKeys = keyTotal > 0;
  const hasModels = models.length > 0;
  const setupLoading = channelsLoading || keysQuery.isLoading;
  const readyToTest = hasChannels && hasModels;

  const defaultTestMessage = useMemo(
    () => t("playground.defaultTestMessage"),
    [t],
  );

  const sendDisabled =
    loading || userMessage.trim().length === 0 || !model;

  const sendRequest = useCallback(
    async (overrideMessage?: string, overrideModel?: string) => {
      const message = (overrideMessage ?? userMessageRef.current).trim();
      const selectedModel = overrideModel ?? modelRef.current;
      if (loading || !message || !selectedModel) return;

      setLoading(true);
      setError(null);
      setResponse("");
      setMetadata(null);
      setJustSucceeded(false);

      const startTime = performance.now();
      try {
        const token = resolveToken(virtualKey);
        const messages: ChatMessage[] = [];
        const sys = systemPrompt.trim();
        if (sys) messages.push({ role: "system", content: sys });
        messages.push({ role: "user", content: message });

        const res = await fetch(`${API_BASE}/v1/chat/completions`, {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            ...buildAuthHeaders(token),
          },
          body: JSON.stringify({
            model: selectedModel,
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
              } catch {
                /* skip malformed SSE chunk */
              }
            }
          }
          setMetadata({
            latency_ms: Math.round(performance.now() - startTime),
            model: selectedModel,
          });
          setJustSucceeded(true);
        } else {
          const data = (await res.json()) as ChatCompletionResponse;
          const content =
            data.choices?.[0]?.message?.content ??
            JSON.stringify(data, null, 2);
          setResponse(content);
          setMetadata({
            latency_ms: Math.round(performance.now() - startTime),
            model: data.model ?? selectedModel,
            prompt_tokens: data.usage?.prompt_tokens,
            completion_tokens: data.usage?.completion_tokens,
            total_tokens: data.usage?.total_tokens,
          });
          setJustSucceeded(true);
        }
      } catch (e) {
        const messageText = e instanceof Error ? e.message : String(e);
        setError(messageText);
        setJustSucceeded(false);
      } finally {
        setLoading(false);
      }
    },
    [loading, virtualKey, systemPrompt, temperature, maxTokens, stream],
  );

  const handleQuickTest = useCallback(() => {
    const msg = userMessage.trim() || defaultTestMessage;
    if (!userMessage.trim()) setUserMessage(msg);
    const selected = model || models[0] || "";
    if (selected && selected !== model) setModel(selected);
    void sendRequest(msg, selected);
  }, [userMessage, defaultTestMessage, model, models, sendRequest]);

  const quickTestDisabled =
    loading || !readyToTest || (!model && models.length === 0);

  return (
    <section className="playground-section">
      <SectionHeader
        title={t("playground.title")}
        icon={FlaskConical}
        onRefresh={() => void fetchModels()}
        refreshing={loadingModels}
      />
      <p className="playground-subtitle">{t("playground.subtitle")}</p>

      <div className="playground-ready" aria-live="polite">
        <div className="playground-ready-title">
          {t("playground.readyTitle")}
        </div>
        <ol className="playground-ready-steps">
          <li
            className={`playground-ready-step ${hasChannels ? "done" : "todo"}`}
          >
            <span className="playground-ready-icon" aria-hidden="true">
              {hasChannels ? <CheckCircle2 size={14} /> : <Radio size={14} />}
            </span>
            <span>
              {setupLoading
                ? t("common.loading")
                : hasChannels
                  ? t("playground.readyChannelsOk", { count: channelCount })
                  : t("playground.readyChannelsMissing")}
            </span>
            {!hasChannels && onNavigate && (
              <button
                type="button"
                className="btn btn-sm btn-primary"
                onClick={() => onNavigate("channels")}
              >
                {t("playground.goChannels")}
              </button>
            )}
          </li>
          <li className={`playground-ready-step ${hasKeys ? "done" : "todo"}`}>
            <span className="playground-ready-icon" aria-hidden="true">
              {hasKeys ? <CheckCircle2 size={14} /> : <KeyRound size={14} />}
            </span>
            <span>
              {setupLoading
                ? t("common.loading")
                : hasKeys
                  ? t("playground.readyKeysOk", { count: keyTotal })
                  : t("playground.readyKeysMissing")}
            </span>
            {!hasKeys && onNavigate && (
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => onNavigate("virtualKeys")}
              >
                {t("playground.goKeys")}
              </button>
            )}
          </li>
          <li
            className={`playground-ready-step ${hasModels ? "done" : "todo"}`}
          >
            <span className="playground-ready-icon" aria-hidden="true">
              {hasModels ? <CheckCircle2 size={14} /> : <Zap size={14} />}
            </span>
            <span>
              {loadingModels
                ? t("playground.fetchingModels")
                : hasModels
                  ? t("playground.readyModelsOk", { count: models.length })
                  : t("playground.readyModelsMissing")}
            </span>
            {!hasModels && !loadingModels && (
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => void fetchModels()}
              >
                {t("common.refresh")}
              </button>
            )}
          </li>
        </ol>
        {readyToTest && (
          <div className="playground-ready-cta">
            <button
              type="button"
              className="btn btn-primary"
              onClick={handleQuickTest}
              disabled={quickTestDisabled}
            >
              {loading ? (
                <>
                  <RefreshCw size={14} className="ui-spin" />
                  {t("playground.sending")}
                </>
              ) : (
                <>
                  <Zap size={14} />
                  {t("playground.quickTest")}
                </>
              )}
            </button>
            <span className="playground-help">{t("playground.quickTestHint")}</span>
          </div>
        )}
      </div>

      <div className="playground-container">
        <div className="playground-panel playground-panel-input">
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
                aria-label={
                  showKey
                    ? t("playground.hideKey")
                    : t("playground.showKey")
                }
              >
                {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
            <span className="playground-help">
              {hasKeys
                ? t("playground.virtualKeyHelpWithKeys")
                : t("playground.virtualKeyHelp")}
            </span>
          </div>

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
                    {loadingModels
                      ? t("playground.fetchingModels")
                      : t("playground.noModels")}
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
                <RefreshCw
                  size={14}
                  className={loadingModels ? "ui-spin" : ""}
                />
              </button>
            </div>
            {modelsError && (
              <span className="playground-error-text">{modelsError}</span>
            )}
            {!modelsError && models.length === 0 && !loadingModels && (
              <span className="playground-help">
                {t("playground.noModels")}
                {onNavigate && (
                  <>
                    {" "}
                    <button
                      type="button"
                      className="playground-inline-link"
                      onClick={() => onNavigate("channels")}
                    >
                      {t("playground.goChannels")}
                    </button>
                  </>
                )}
              </span>
            )}
          </div>

          <div className="playground-field">
            <label className="playground-label" htmlFor="pg-user">
              {t("playground.userMessage")}
            </label>
            <textarea
              id="pg-user"
              className="playground-input playground-textarea playground-textarea-lg"
              rows={5}
              value={userMessage}
              onChange={(e) => setUserMessage(e.target.value)}
              placeholder={defaultTestMessage}
              spellCheck={false}
            />
          </div>

          <div className="form-advanced-toggle">
            <button
              type="button"
              className="btn btn-sm btn-ghost"
              onClick={() => setShowAdvanced((v) => !v)}
            >
              {showAdvanced ? "\u25BC" : "\u25B6"}{" "}
              {t("playground.advancedOptions")}
            </button>
          </div>

          {showAdvanced && (
            <>
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

              <div className="playground-controls">
                <div className="playground-control">
                  <label className="playground-label" htmlFor="pg-temp">
                    {t("playground.temperature")}:{" "}
                    <span className="mono">{temperature.toFixed(1)}</span>
                  </label>
                  <input
                    id="pg-temp"
                    type="range"
                    min={0}
                    max={2}
                    step={TEMPERATURE_STEP}
                    value={temperature}
                    onChange={(e) =>
                      setTemperature(Number.parseFloat(e.target.value))
                    }
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
                        setMaxTokens(
                          Math.min(
                            MAX_MAX_TOKENS,
                            Math.max(MIN_MAX_TOKENS, n),
                          ),
                        );
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
                  <label
                    className="playground-label-inline"
                    htmlFor="pg-stream"
                  >
                    {t("playground.stream")}
                  </label>
                </div>
              </div>
            </>
          )}

          <div className="playground-actions">
            <button
              type="button"
              className="btn"
              onClick={handleQuickTest}
              disabled={quickTestDisabled}
            >
              <Zap size={14} />
              {t("playground.quickTest")}
            </button>
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

        <div className="playground-panel playground-panel-response">
          <div className="playground-response-header">
            <Play size={16} className="playground-response-icon" />
            <h3 className="playground-response-title">
              {t("playground.response")}
            </h3>
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
              <p className="playground-help">{t("playground.errorHint")}</p>
              {onNavigate && (
                <div className="playground-error-actions">
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => onNavigate("channels")}
                  >
                    {t("playground.goChannels")}
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => onNavigate("virtualKeys")}
                  >
                    {t("playground.goKeys")}
                  </button>
                </div>
              )}
            </div>
          )}

          {!loading && !error && response && (
            <>
              {justSucceeded && metadata && (
                <div className="playground-success-banner" role="status">
                  <CheckCircle2 size={16} />
                  <span>
                    {t("playground.testOk", {
                      latency: metadata.latency_ms,
                    })}
                  </span>
                </div>
              )}
              <pre className="playground-response-body">{response}</pre>
              {metadata && (
                <div className="playground-metadata">
                  <div className="playground-meta-tile">
                    <span className="playground-meta-icon">
                      <Clock size={14} />
                    </span>
                    <div className="playground-meta-body">
                      <div className="playground-meta-value">
                        {metadata.latency_ms} ms
                      </div>
                      <div className="playground-meta-label">
                        {t("playground.latency")}
                      </div>
                    </div>
                  </div>
                  <div className="playground-meta-tile">
                    <span className="playground-meta-icon">
                      <Cpu size={14} />
                    </span>
                    <div className="playground-meta-body">
                      <div
                        className="playground-meta-value"
                        title={metadata.model}
                      >
                        {metadata.model}
                      </div>
                      <div className="playground-meta-label">
                        {t("playground.model")}
                      </div>
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
                      <div className="playground-meta-label">
                        {t("playground.totalTokens")}
                      </div>
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
                        <div className="playground-meta-label">
                          {t("playground.promptTokens")}
                        </div>
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
              title={
                readyToTest
                  ? t("playground.noResponse")
                  : t("playground.setupNeeded")
              }
              description={
                readyToTest
                  ? t("playground.quickTestHint")
                  : t("playground.setupNeededHint")
              }
              action={
                readyToTest ? (
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={handleQuickTest}
                    disabled={quickTestDisabled}
                  >
                    {t("playground.quickTest")}
                  </button>
                ) : onNavigate ? (
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={() =>
                      onNavigate(hasChannels ? "virtualKeys" : "channels")
                    }
                  >
                    {hasChannels
                      ? t("playground.goKeys")
                      : t("playground.goChannels")}
                  </button>
                ) : undefined
              }
            />
          )}
        </div>
      </div>
    </section>
  );
}
