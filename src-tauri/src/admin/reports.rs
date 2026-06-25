//! Usage report endpoints — daily aggregated token/cost breakdown.
//!
//! Provides JSON and CSV exports of dispatch-log data filtered by virtual key,
//! group, or date range.

use super::ApiResponse;
use crate::proxy::AppState;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::Json;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

// ─── Types ─────────────────────────────────────────────

/// Query parameters accepted by both report endpoints.
#[derive(Deserialize, Debug)]
pub struct UsageReportParams {
    /// Filter to a specific virtual key by its ID.
    pub key_id: Option<String>,
    /// Filter to all virtual keys belonging to a department/group label.
    pub group: Option<String>,
    /// Inclusive start date (YYYY-MM-DD). Defaults to 30 days ago.
    pub from: Option<String>,
    /// Inclusive end date (YYYY-MM-DD). Defaults to today.
    pub to: Option<String>,
}

/// One day's aggregated usage row.
#[derive(Serialize, Debug)]
pub struct DailyUsage {
    pub date: String,
    pub total_requests: usize,
    pub total_cost_cents: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

/// Summary totals across the entire selected range.
#[derive(Serialize, Debug, Default)]
pub struct UsageSummary {
    pub total_requests: usize,
    pub total_cost_cents: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

/// Full report payload returned by the JSON endpoint.
#[derive(Serialize, Debug)]
pub struct UsageReport {
    pub daily: Vec<DailyUsage>,
    pub summary: UsageSummary,
}

// ─── Helpers ───────────────────────────────────────────

/// Parse a `YYYY-MM-DD` string into a `NaiveDate`, returning a 400-style
/// message on failure.
fn parse_date(s: &Option<String>, field: &str) -> Result<Option<NaiveDate>, String> {
    match s {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
                .map(Some)
                .map_err(|_| {
                    format!("invalid `{field}` date: expected YYYY-MM-DD, got \"{trimmed}\"")
                })
        }
        None => Ok(None),
    }
}

/// Collect the string-form IDs of all virtual keys whose `group` matches.
async fn key_ids_for_group(state: &Arc<AppState>, group: &str) -> Vec<String> {
    state
        .billing
        .virtual_key_store
        .list()
        .await
        .into_iter()
        .filter(|vk| vk.group.as_deref() == Some(group))
        .map(|vk| vk.id.to_string())
        .collect()
}

/// Build the daily aggregation from a slice of dispatch logs.
fn aggregate_daily(
    logs: &[crate::log::DispatchLog],
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> UsageReport {
    let mut buckets: BTreeMap<NaiveDate, DailyUsage> = BTreeMap::new();
    let mut summary = UsageSummary::default();

    for log in logs {
        let day = log.timestamp.date_naive();

        // Date-range filter (inclusive on both ends).
        if let Some(f) = from {
            if day < f {
                continue;
            }
        }
        if let Some(t) = to {
            if day > t {
                continue;
            }
        }

        let cost_cents = log.estimated_cost.unwrap_or(0.0) * 100.0;
        let cost_cents = if cost_cents.is_finite() && cost_cents >= 0.0 {
            cost_cents.round() as u64
        } else {
            0
        };
        let input = log.input_tokens.unwrap_or(0);
        let output = log.output_tokens.unwrap_or(0);

        let entry = buckets.entry(day).or_insert_with(|| DailyUsage {
            date: day.format("%Y-%m-%d").to_string(),
            total_requests: 0,
            total_cost_cents: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
        });
        entry.total_requests += 1;
        entry.total_cost_cents = entry.total_cost_cents.saturating_add(cost_cents);
        entry.total_input_tokens += input;
        entry.total_output_tokens += output;

        summary.total_requests += 1;
        summary.total_cost_cents = summary.total_cost_cents.saturating_add(cost_cents);
        summary.total_input_tokens += input;
        summary.total_output_tokens += output;
    }

    UsageReport {
        daily: buckets.into_values().collect(),
        summary,
    }
}

// ─── Handlers ──────────────────────────────────────────

/// GET /api/reports/usage — daily usage report as JSON.
///
/// Accepts optional `key_id`, `group`, `from`, and `to` query parameters.
/// When `group` is supplied, the report covers every virtual key whose group
/// label matches. When both `group` and `key_id` are omitted, all keys are
/// included.
pub async fn get_usage_report(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UsageReportParams>,
) -> Result<Json<ApiResponse<UsageReport>>, Response> {
    let from = match parse_date(&params.from, "from") {
        Ok(d) => d,
        Err(msg) => {
            return Err(crate::proxy::stream::json_response(
                axum::http::StatusCode::BAD_REQUEST,
                serde_json::json!({ "error": { "message": msg, "code": "bad_request" } })
                    .to_string(),
            ));
        }
    };
    let to = match parse_date(&params.to, "to") {
        Ok(d) => d,
        Err(msg) => {
            return Err(crate::proxy::stream::json_response(
                axum::http::StatusCode::BAD_REQUEST,
                serde_json::json!({ "error": { "message": msg, "code": "bad_request" } })
                    .to_string(),
            ));
        }
    };

    // Fetch logs: if key_id is provided, use the key-scoped path; otherwise
    // pull the full bounded ring buffer.
    let logs: Vec<crate::log::DispatchLog> = if let Some(ref key_id) = params.key_id {
        let total = state.logger.total_by_key(key_id).await;
        state.logger.list_by_key(key_id, 0, total).await
    } else {
        let total = state.logger.total().await;
        state.logger.list(0, total).await
    };

    // Group filter: narrow to keys belonging to the group.
    let filtered: Vec<crate::log::DispatchLog> = if let Some(ref group) = params.group {
        let group_ids = key_ids_for_group(&state, group).await;
        logs.into_iter()
            .filter(|l| {
                l.virtual_key_id
                    .as_deref()
                    .map(|id| group_ids.iter().any(|gid| gid == id))
                    .unwrap_or(false)
            })
            .collect()
    } else {
        logs
    };

    let report = aggregate_daily(&filtered, from, to);
    Ok(Json(ApiResponse::ok(report)))
}

/// GET /api/reports/usage/csv — same data as JSON endpoint, as CSV download.
pub async fn get_usage_report_csv(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UsageReportParams>,
) -> Result<Response, Response> {
    // Reuse the JSON handler's logic, then serialise to CSV.
    let report = match get_usage_report(State(state), Query(params)).await {
        Ok(Json(resp)) => resp.data,
        Err(resp) => return Err(resp),
    };

    let mut csv = String::from(
        "date,total_requests,total_cost_cents,total_input_tokens,total_output_tokens\n",
    );
    for row in &report.daily {
        csv.push_str(&format!(
            "{},{},{},{},{}\n",
            row.date,
            row.total_requests,
            row.total_cost_cents,
            row.total_input_tokens,
            row.total_output_tokens,
        ));
    }
    // Summary as a trailing comment-style row.
    csv.push_str(&format!(
        "#summary,{},{},{},{}\n",
        report.summary.total_requests,
        report.summary.total_cost_cents,
        report.summary.total_input_tokens,
        report.summary.total_output_tokens,
    ));

    Ok(Response::builder()
        .header("Content-Type", "text/csv; charset=utf-8")
        .header(
            "Content-Disposition",
            "attachment; filename=\"modelswitch-usage-report.csv\"",
        )
        .body(axum::body::Body::from(csv))
        .expect("valid CSV response"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::DispatchLog;
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn make_log(cost: f64, input: u64, output: u64, days_ago: i64) -> DispatchLog {
        DispatchLog {
            id: Uuid::new_v4(),
            timestamp: Utc::now() - Duration::days(days_ago),
            request_model: "test-model".into(),
            channel_id: Uuid::new_v4(),
            channel_name: "test".into(),
            channel_priority: 1,
            retry_count: 0,
            trigger_reason: None,
            latency_ms: 100,
            success: true,
            estimated_cost: Some(cost),
            input_tokens: Some(input),
            output_tokens: Some(output),
            cache_hit_tokens: None,
            cache_miss_tokens: None,
            request_id: None,
            virtual_key_id: None,
        }
    }

    #[test]
    fn aggregate_empty() {
        let report = aggregate_daily(&[], None, None);
        assert!(report.daily.is_empty());
        assert_eq!(report.summary.total_requests, 0);
    }

    #[test]
    fn aggregate_groups_by_day() {
        let logs = vec![
            make_log(0.01, 100, 50, 0),
            make_log(0.02, 200, 100, 0),
            make_log(0.05, 500, 250, 1),
        ];
        let report = aggregate_daily(&logs, None, None);
        // Two distinct days.
        assert_eq!(report.daily.len(), 2);
        // Today's bucket has 2 requests.
        let today = report.daily.last().unwrap();
        assert_eq!(today.total_requests, 2);
        assert_eq!(today.total_input_tokens, 300);
        assert_eq!(today.total_output_tokens, 150);
        assert_eq!(today.total_cost_cents, 3); // 0.03 * 100
                                               // Summary totals.
        assert_eq!(report.summary.total_requests, 3);
        assert_eq!(report.summary.total_input_tokens, 800);
    }

    #[test]
    fn aggregate_respects_date_range() {
        let logs = vec![make_log(0.01, 100, 50, 0), make_log(0.05, 500, 250, 5)];
        // Only include logs from 3 days ago onward.
        let from = (Utc::now() - Duration::days(3)).date_naive();
        let report = aggregate_daily(&logs, Some(from), None);
        assert_eq!(report.daily.len(), 1);
        assert_eq!(report.summary.total_requests, 1);
    }

    #[test]
    fn parse_valid_date() {
        assert_eq!(
            parse_date(&Some("2025-06-01".into()), "from").unwrap(),
            Some(NaiveDate::from_ymd_opt(2025, 6, 1).unwrap())
        );
    }

    #[test]
    fn parse_empty_date() {
        assert_eq!(parse_date(&None, "from").unwrap(), None);
        assert_eq!(parse_date(&Some("".into()), "from").unwrap(), None);
    }

    #[test]
    fn parse_invalid_date() {
        assert!(parse_date(&Some("not-a-date".into()), "from").is_err());
    }
}
