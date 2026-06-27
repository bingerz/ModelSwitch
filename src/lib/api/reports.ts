// Reports + metrics + logs endpoints.
// Flat methods (`stats`, `costStats`, `usageHistory`, `logs`, `auditLog`,
// `metrics`) plus the nested `api.reports` object.

import { request, fetchText } from "./client";
import { API_BASE } from "../runtime";
import type {
  AuditLogEntry,
  CostStats,
  DispatchLog,
  DispatchStats,
  PaginatedEnvelope,
  UsageHistory,
  UsageReport,
  UsageReportParams,
} from "./types";

export const reportsFlatApi = {
  stats: () => request<DispatchStats>("/api/stats"),

  costStats: () => request<CostStats>("/api/stats/cost"),

  usageHistory: (hours = 24) =>
    request<UsageHistory>(`/api/stats/usage?hours=${hours}`),

  auditLog: (limit = 100) =>
    request<AuditLogEntry[]>(`/api/audit-log?limit=${limit}`),

  // Prometheus text format. The original implementation used a manual fetch
  // with bearer auth; we delegate to the shared `fetchText` helper so the
  // auth-header logic is no longer duplicated.
  metrics: () => fetchText("/metrics"),
};

/**
 * Dispatch logs. The backend returns a raw `PaginatedEnvelope<DispatchLog[]>`
 * (`{ data, total, offset, limit }`) without the ApiResponse wrapper, so
 * `request()` returns the envelope as-is and we extract `.data`.
 *
 * The mock test fixture wraps the response in `{ ok: true, data: [...] }`,
 * in which case `request()` already auto-unwraps to the array; handle both.
 */
export async function logs(offset = 0, limit = 50): Promise<DispatchLog[]> {
  const result = await request<DispatchLog[] | PaginatedEnvelope<DispatchLog[]>>(
    `/api/logs?offset=${offset}&limit=${limit}`,
  );
  if (Array.isArray(result)) return result;
  return result.data;
}

export const reportsApi = {
  usage: (params: UsageReportParams = {}) => {
    const qs = new URLSearchParams();
    if (params.key_id) qs.set("key_id", params.key_id);
    if (params.group) qs.set("group", params.group);
    if (params.from) qs.set("from", params.from);
    if (params.to) qs.set("to", params.to);
    if (params.group_by) qs.set("group_by", params.group_by);
    const query = qs.toString();
    return request<UsageReport>(
      query ? `/api/reports/usage?${query}` : "/api/reports/usage",
    );
  },

  usageCsv: (params: UsageReportParams = {}) => {
    const qs = new URLSearchParams();
    if (params.key_id) qs.set("key_id", params.key_id);
    if (params.group) qs.set("group", params.group);
    if (params.from) qs.set("from", params.from);
    if (params.to) qs.set("to", params.to);
    if (params.group_by) qs.set("group_by", params.group_by);
    window.open(`${API_BASE}/api/reports/usage/csv?${qs}`, "_blank");
  },
};
