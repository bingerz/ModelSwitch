import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, Check } from "lucide-react";
import { api } from "../lib/api";

/**
 * Model × Channel coverage matrix.
 *
 * Shows which channels can serve each model (based on `model_mapping` keys,
 * minus `excluded_models` glob patterns). Helps admins spot single-point-of
 * -failure models and coverage gaps at a glance.
 */
export function ModelAvailabilityMatrix() {
  const { t } = useTranslation();
  const [showDisabled, setShowDisabled] = useState(false);

  const { data: channels, isLoading } = useQuery({
    queryKey: ["channels-matrix"],
    queryFn: () => api.listChannels(),
    retry: false,
  });

  const { models, activeChannels, matrix, singleChannelCount } = useMemo(() => {
    const all = channels ?? [];
    const filtered = showDisabled ? all : all.filter((c) => c.enabled);

    // Collect every virtual model name accepted by at least one channel.
    const modelSet = new Set<string>();
    for (const ch of filtered) {
      for (const model of Object.keys(ch.model_mapping)) {
        modelSet.add(model);
      }
    }
    const modelList = Array.from(modelSet).sort();

    // For each model, record the set of channel IDs that can serve it.
    const serving = new Map<string, Set<string>>();
    for (const model of modelList) {
      const servers = new Set<string>();
      for (const ch of filtered) {
        if (!(model in ch.model_mapping)) continue;
        const isExcluded = ch.excluded_models.some((pattern) => simpleGlob(pattern, model));
        if (!isExcluded) servers.add(ch.id);
      }
      serving.set(model, servers);
    }

    let singleCount = 0;
    for (const m of modelList) {
      if ((serving.get(m)?.size ?? 0) === 1) singleCount += 1;
    }

    return {
      models: modelList,
      activeChannels: filtered,
      matrix: serving,
      singleChannelCount: singleCount,
    };
  }, [channels, showDisabled]);

  if (isLoading) {
    return <div className="panel-loading">{t("common.loading")}</div>;
  }

  if (!channels || channels.length === 0) {
    return <div className="registry-empty">{t("matrix.noChannels")}</div>;
  }

  return (
    <div className="matrix-container">
      <div className="matrix-controls">
        <label className="registry-group-toggle">
          <input
            type="checkbox"
            checked={showDisabled}
            onChange={(e) => setShowDisabled(e.target.checked)}
          />
          <span>{t("matrix.showDisabled")}</span>
        </label>
      </div>

      <div className="matrix-stats">
        <span className="matrix-stat">
          {t("matrix.totalModels", { count: models.length })}
        </span>
        <span className="matrix-stat">
          {t("matrix.totalChannels", { count: activeChannels.length })}
        </span>
        <span className="matrix-stat matrix-stat-warn">
          <AlertTriangle size={12} />
          {t("matrix.singleChannelModels", { count: singleChannelCount })}
        </span>
      </div>

      <div className="matrix-scroll">
        <table className="matrix-table">
          <thead>
            <tr>
              <th className="matrix-corner">{t("matrix.model")}</th>
              {activeChannels.map((ch) => (
                <th key={ch.id} className="matrix-col-header" title={ch.name}>
                  <span className={`matrix-ch-name ${!ch.enabled ? "disabled" : ""}`}>
                    {ch.name}
                  </span>
                  <span className="matrix-ch-provider">{ch.provider}</span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {models.map((model) => {
              const serving = matrix.get(model) ?? new Set<string>();
              const count = serving.size;
              return (
                <tr key={model} className={count === 1 ? "matrix-row-warn" : ""}>
                  <td className="matrix-row-header mono">{model}</td>
                  {activeChannels.map((ch) => (
                    <td key={ch.id} className="matrix-cell">
                      {serving.has(ch.id) ? (
                        <Check size={14} className="matrix-check" />
                      ) : (
                        <span className="matrix-dash">—</span>
                      )}
                    </td>
                  ))}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

/**
 * Minimal glob matcher: `*` matches any sequence, `?` matches a single char.
 * Case-insensitive. Anything else is matched literally.
 */
/** Simple glob matching: * matches any sequence, ? matches one char.
 * Iterative character-by-character matcher — no RegExp construction,
 * avoiding ReDoS risk from admin-configured excluded_models patterns.
 */
function simpleGlob(pattern: string, text: string): boolean {
  const p = pattern.toLowerCase();
  const t = text.toLowerCase();
  const memo = new Map<string, boolean>();
  function match(pi: number, ti: number): boolean {
    const key = `${pi}:${ti}`;
    if (memo.has(key)) return memo.get(key)!;
    if (pi >= p.length) { return ti >= t.length; }
    const pc = p[pi];
    let result = false;
    if (pc === "*") {
      if (pi === p.length - 1) { result = true; }
      else {
        for (let i = ti; i <= t.length && !result; i++) {
          result = match(pi + 1, i);
        }
      }
    } else if (pc === "?") {
      result = ti < t.length && match(pi + 1, ti + 1);
    } else {
      result = ti < t.length && t[ti] === pc && match(pi + 1, ti + 1);
    }
    memo.set(key, result);
    return result;
  }
  return match(0, 0);
}
