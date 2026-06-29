import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, Search } from "lucide-react";
import { api } from "../../lib/api";
import type { McpServer, McpToolInfo } from "../../lib/api";
import { truncate } from "./types";

interface McpAllToolsPanelProps {
  servers: McpServer[];
}

interface AllToolsState {
  loading: boolean;
  error: string | null;
  tools: McpToolInfo[];
  loaded: boolean;
}

/**
 * Collapsible cross-server tool inventory. Fetches the aggregated
 * `GET /api/mcp/tools` endpoint lazily on first expand so collapsed
 * state incurs no requests.
 */
export function McpAllToolsPanel({ servers }: McpAllToolsPanelProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [query, setQuery] = useState("");
  const [state, setState] = useState<AllToolsState>({
    loading: false,
    error: null,
    tools: [],
    loaded: false,
  });

  const serverNameById = useMemo(() => {
    const map: Record<string, string> = {};
    for (const s of servers) map[s.id] = s.name;
    return map;
  }, [servers]);

  const loadTools = async () => {
    if (state.loaded || state.loading) return;
    setState((prev) => ({ ...prev, loading: true, error: null }));
    try {
      const tools = await api.mcp.listAllTools();
      setState({ loading: false, error: null, tools, loaded: true });
    } catch (err) {
      setState((prev) => ({
        ...prev,
        loading: false,
        error: err instanceof Error ? err.message : t("mcp.listToolsFailed"),
        loaded: true,
      }));
    }
  };

  const handleToggle = () => {
    const next = !expanded;
    setExpanded(next);
    if (next) void loadTools();
  };

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return state.tools;
    return state.tools.filter(
      (tool) =>
        tool.name.toLowerCase().includes(q) ||
        (tool.description?.toLowerCase().includes(q) ?? false),
    );
  }, [state.tools, query]);

  const grouped = useMemo(() => {
    const groups = new Map<string, McpToolInfo[]>();
    for (const tool of filtered) {
      const key = tool.server_id;
      const arr = groups.get(key);
      if (arr) arr.push(tool);
      else groups.set(key, [tool]);
    }
    return Array.from(groups.entries()).sort((a, b) => a[0].localeCompare(b[0]));
  }, [filtered]);

  const serverCount = useMemo(() => {
    const ids = new Set<string>();
    for (const tool of state.tools) ids.add(tool.server_id);
    return ids.size;
  }, [state.tools]);

  const Chevron = expanded ? ChevronDown : ChevronRight;

  return (
    <div className="mcp-all-tools-panel">
      <button
        type="button"
        className="mcp-all-tools-header"
        onClick={handleToggle}
        aria-expanded={expanded}
      >
        <Chevron size={14} className="mcp-all-tools-chevron" />
        <span className="mcp-all-tools-title">{t("mcp.allTools")}</span>
        {state.loaded && !state.error && (
          <span className="mcp-all-tools-summary">
            {t("mcp.allToolsSummary", {
              count: state.tools.length,
              servers: serverCount,
            })}
          </span>
        )}
      </button>

      {expanded && (
        <div className="mcp-all-tools-body">
          <div className="mcp-all-tools-search">
            <Search size={14} className="mcp-all-tools-search-icon" />
            <input
              type="text"
              className="mcp-all-tools-search-input"
              placeholder={t("mcp.searchTools")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label={t("mcp.searchTools")}
            />
          </div>

          {state.loading && <div className="mcp-tools-empty">{t("common.loading")}</div>}

          {state.error && (
            <div className="mcp-error-message mono">{state.error}</div>
          )}

          {!state.loading && !state.error && filtered.length === 0 && (
            <div className="mcp-tools-empty">{t("mcp.noTools")}</div>
          )}

          {!state.loading && !state.error && filtered.length > 0 && (
            <div className="mcp-all-tools-groups">
              {grouped.map(([serverId, tools]) => (
                <div key={serverId} className="mcp-all-tools-group">
                  <div className="mcp-all-tools-group-header">
                    <span className="meta-tag mono">
                      {serverNameById[serverId] ?? serverId}
                    </span>
                    <span className="mcp-all-tools-group-count">
                      {t("mcp.toolsCount", { count: tools.length })}
                    </span>
                  </div>
                  <div className="mcp-tools-list">
                    {tools.map((tool) => (
                      <div key={`${serverId}-${tool.name}`} className="mcp-tool-item">
                        <span className="mcp-tool-name mono">{tool.name}</span>
                        {tool.description && (
                          <span className="mcp-tool-desc">
                            {truncate(tool.description, 100)}
                          </span>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
