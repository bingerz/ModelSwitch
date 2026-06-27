import { useEffect, useMemo, useState } from "react";
import { useQuery, keepPreviousData } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { BarChart3, DollarSign, Coins, TrendingUp, FileText, Download } from "lucide-react";
import { api, type UsageReportParams } from "../lib/api";
import { formatNumber, formatCents, formatTokens } from "../lib/format";
import { useToast } from "./Toast";
import { StatTile } from "./ui/StatTile";
import { SectionHeader } from "./ui/SectionHeader";
import { EmptyState } from "./ui/EmptyState";
import "../styles/pages-enhanced.css";

type GroupBy = "day" | "week" | "month";

/** Returns ISO date (YYYY-MM-DD) for `daysAgo` days before today. */
function isoDaysAgo(daysAgo: number): string {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - daysAgo);
  return d.toISOString().slice(0, 10);
}

export function ReportsPanel() {
  const { t } = useTranslation();
  const toast = useToast();

  // Filter state — defaults to last 30 days
  const [from, setFrom] = useState<string>(() => isoDaysAgo(30));
  const [to, setTo] = useState<string>(() => isoDaysAgo(0));
  const [keyId, setKeyId] = useState("");
  const [group, setGroup] = useState("");
  const [groupBy, setGroupBy] = useState<GroupBy>("day");

  // Submitted params — only updated when the user clicks Generate.
  // null = never generated, so the query stays disabled until first click.
  const [submittedParams, setSubmittedParams] = useState<UsageReportParams | null>(null);

  const buildParams = (): UsageReportParams => {
    const params: UsageReportParams = {
      from,
      to,
      group_by: groupBy,
    };
    if (keyId.trim()) params.key_id = keyId.trim();
    if (group.trim()) params.group = group.trim();
    return params;
  };

  const { data: report, isFetching: loading, error, refetch } = useQuery({
    queryKey: ["reports-usage", submittedParams],
    queryFn: () => api.reports.usage(submittedParams!),
    enabled: submittedParams !== null,
    placeholderData: keepPreviousData,
    retry: false,
  });

  // Surface fetch errors via toast — matches the original UX where generate()
  // showed an error toast on failure.
  useEffect(() => {
    if (error) {
      toast.error(t("reports.loadFailed"));
    }
  }, [error, t, toast]);

  const generate = async () => {
    setSubmittedParams(buildParams());
    // Trigger an immediate refetch so subsequent clicks (without filter changes)
    // still re-request fresh data.
    await refetch();
  };

  const exportCsv = () => {
    try {
      api.reports.usageCsv(buildParams());
    } catch {
      toast.error(t("reports.exportFailed"));
    }
  };

  // Sort rows by date descending by default
  const sortedRows = useMemo(() => {
    if (!report) return [];
    return [...report.rows].sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : 0));
  }, [report]);

  return (
    <section className="vk-panel">
      <SectionHeader
        title={t("reports.title")}
        icon={FileText}
        action={
          <button
            className="btn btn-primary"
            onClick={exportCsv}
            disabled={!report || report.rows.length === 0}
            title={t("reports.exportHint")}
          >
            <Download size={14} />
            {t("reports.exportCsv")}
          </button>
        }
      />

      {/* Filter bar */}
      <div className="reports-filter-bar">
        <div className="reports-filter-group">
          <label className="reports-filter-label">{t("reports.from")}</label>
          <input
            type="date"
            className="vk-search-input reports-filter-input"
            value={from}
            onChange={(e) => setFrom(e.target.value)}
          />
        </div>
        <div className="reports-filter-group">
          <label className="reports-filter-label">{t("reports.to")}</label>
          <input
            type="date"
            className="vk-search-input reports-filter-input"
            value={to}
            onChange={(e) => setTo(e.target.value)}
          />
        </div>
        <div className="reports-filter-group">
          <label className="reports-filter-label">{t("reports.keyId")}</label>
          <input
            type="text"
            className="vk-search-input reports-filter-input"
            placeholder={t("reports.keyIdPlaceholder")}
            value={keyId}
            onChange={(e) => setKeyId(e.target.value)}
          />
        </div>
        <div className="reports-filter-group">
          <label className="reports-filter-label">{t("reports.group")}</label>
          <input
            type="text"
            className="vk-search-input reports-filter-input"
            placeholder={t("reports.groupPlaceholder")}
            value={group}
            onChange={(e) => setGroup(e.target.value)}
          />
        </div>
        <div className="reports-filter-group">
          <label className="reports-filter-label">{t("reports.groupBy")}</label>
          <select
            className="vk-search-input reports-filter-input"
            value={groupBy}
            onChange={(e) => setGroupBy(e.target.value as GroupBy)}
          >
            <option value="day">{t("reports.groupByDay")}</option>
            <option value="week">{t("reports.groupByWeek")}</option>
            <option value="month">{t("reports.groupByMonth")}</option>
          </select>
        </div>
        <div className="reports-filter-actions">
          <button
            className="btn btn-primary"
            onClick={generate}
            disabled={loading}
          >
            {loading ? t("common.loading") : t("reports.generate")}
          </button>
        </div>
      </div>

      {/* Error banner */}
      {error && (
        <div className="reports-error-banner">
          {error instanceof Error ? error.message : String(error)}
        </div>
      )}

      {/* Summary cards */}
      {report && (
        <div className="cost-summary-grid reports-summary-grid">
          <StatTile
            icon={BarChart3}
            value={formatNumber(report.summary.total_requests)}
            label={t("reports.totalRequests")}
            accent="blue"
          />
          <StatTile
            icon={Coins}
            value={formatTokens(report.summary.total_tokens)}
            label={t("reports.totalTokens")}
            accent="gray"
          />
          <StatTile
            icon={DollarSign}
            value={formatCents(report.summary.total_cost_cents)}
            label={t("reports.totalCost")}
            accent="green"
          />
          <StatTile
            icon={TrendingUp}
            value={formatCents(report.summary.avg_daily_cost_cents)}
            label={t("reports.avgDailyCost")}
            accent="amber"
          />
        </div>
      )}

      {/* Data table */}
      {!report && !loading && submittedParams === null && (
        <EmptyState
          icon={FileText}
          title={t("reports.empty")}
          description={t("reports.emptyHint")}
        />
      )}

      {submittedParams !== null && !loading && report && report.rows.length === 0 && (
        <EmptyState
          icon={FileText}
          title={t("reports.noData")}
          description={t("reports.noDataHint")}
        />
      )}

      {report && sortedRows.length > 0 && (
        <div className="settings-table-wrapper">
          <table className="settings-table">
            <thead>
              <tr>
                <th>{t("reports.colDate")}</th>
                <th>{t("reports.colKey")}</th>
                <th>{t("reports.colGroup")}</th>
                <th>{t("reports.colRequests")}</th>
                <th>{t("reports.colTokens")}</th>
                <th>{t("reports.colCost")}</th>
              </tr>
            </thead>
            <tbody>
              {sortedRows.map((row, i) => {
                const rowKey = `${row.date}-${row.key_id ?? "all"}-${i}`;
                return (
                  <tr key={rowKey}>
                    <td className="mono" style={{ whiteSpace: "nowrap" }}>
                      {row.date}
                    </td>
                    <td>{row.key_name ?? "—"}</td>
                    <td>{row.group ?? "—"}</td>
                    <td className="mono">{formatNumber(row.requests)}</td>
                    <td className="mono">
                      {formatTokens(row.input_tokens)} / {formatTokens(row.output_tokens)}
                    </td>
                    <td className="mono">{formatCents(row.estimated_cost_cents)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
