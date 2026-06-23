use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::ApiResponse;

// ─── Types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetProviderBudgetRequest {
    #[serde(default)]
    pub daily_budget_cents: Option<u64>,
    #[serde(default)]
    pub monthly_budget_cents: Option<u64>,
}

/// Combined budget config + current spend for the list endpoint.
#[derive(Debug, Serialize)]
pub struct ProviderBudgetResponse {
    pub provider: String,
    pub daily_budget_cents: Option<u64>,
    pub monthly_budget_cents: Option<u64>,
    pub spend: crate::provider_budget::ProviderSpend,
}

/// Persist provider budgets to disk.
async fn persist_provider_budgets(state: &Arc<AppState>) {
    if let Err(e) = state.billing.provider_budgets.persist().await {
        tracing::warn!(error = %e, "Failed to persist provider budgets");
    }
}

// ─── Endpoints ─────────────────────────────────────────

/// GET /api/provider-budgets -- list all budget configs with current spend.
pub async fn list_provider_budgets(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<ProviderBudgetResponse>>> {
    let budgets = state.billing.provider_budgets.list_budgets().await;
    let spend_map = state.billing.provider_budgets.list_spend().await;

    // Build a combined view: all providers that have either a config or spend.
    use std::collections::HashMap;
    let spend_by_provider: HashMap<String, crate::provider_budget::ProviderSpend> =
        spend_map.into_iter().collect();

    let mut results: Vec<ProviderBudgetResponse> = budgets
        .iter()
        .map(|(provider, config)| {
            let spend = spend_by_provider.get(provider).cloned().unwrap_or_default();
            ProviderBudgetResponse {
                provider: provider.clone(),
                daily_budget_cents: config.daily_budget_cents,
                monthly_budget_cents: config.monthly_budget_cents,
                spend,
            }
        })
        .collect();

    // Include providers that have spend but no budget config.
    for (provider, spend) in &spend_by_provider {
        if !budgets.iter().any(|(p, _)| p == provider) {
            results.push(ProviderBudgetResponse {
                provider: provider.clone(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                spend: spend.clone(),
            });
        }
    }

    results.sort_by(|a, b| a.provider.cmp(&b.provider));
    Json(ApiResponse::ok(results))
}

/// PUT /api/provider-budgets/:provider -- set or update budget config for a provider.
pub async fn set_provider_budget(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Json(req): Json<SetProviderBudgetRequest>,
) -> Result<Json<ApiResponse<ProviderBudgetResponse>>, axum::response::Response> {
    if provider.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Provider name is required",
        ));
    }
    if let Some(d) = req.daily_budget_cents {
        if let Some(m) = req.monthly_budget_cents {
            if m < d {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Monthly budget must be >= daily budget",
                ));
            }
        }
    }

    let config = crate::provider_budget::ProviderBudgetConfig {
        daily_budget_cents: req.daily_budget_cents,
        monthly_budget_cents: req.monthly_budget_cents,
    };
    state
        .billing
        .provider_budgets
        .set_budget(&provider, config.clone())
        .await;
    persist_provider_budgets(&state).await;

    let spend = state
        .billing
        .provider_budgets
        .get_spend(&provider)
        .await
        .unwrap_or_default();

    Ok(Json(ApiResponse::ok(ProviderBudgetResponse {
        provider,
        daily_budget_cents: config.daily_budget_cents,
        monthly_budget_cents: config.monthly_budget_cents,
        spend,
    })))
}

/// DELETE /api/provider-budgets/:provider -- remove budget config for a provider.
pub async fn delete_provider_budget(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
) -> axum::response::Response {
    if state
        .billing
        .provider_budgets
        .delete_budget(&provider)
        .await
    {
        persist_provider_budgets(&state).await;
        StatusCode::NO_CONTENT.into_response()
    } else {
        ApiError::new(StatusCode::NOT_FOUND, "Provider budget not found")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ChannelConfig;
    use crate::test_helpers::build_test_state;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::Json;

    #[tokio::test]
    async fn list_provider_budgets_returns_empty() {
        let state = build_test_state(vec![]);
        let result = list_provider_budgets(State(state)).await;
        assert!(result.data.is_empty());
    }

    #[tokio::test]
    async fn set_provider_budget_creates_entry() {
        let state = build_test_state(vec![]);
        let req = SetProviderBudgetRequest {
            daily_budget_cents: Some(500),
            monthly_budget_cents: Some(15000),
        };
        let result =
            set_provider_budget(State(state.clone()), Path("openai".to_string()), Json(req)).await;
        assert!(result.is_ok());
        let response = result.unwrap().0;
        assert_eq!(response.data.provider, "openai");

        let list_result = list_provider_budgets(State(state)).await;
        assert_eq!(list_result.data.len(), 1);
        assert_eq!(list_result.data[0].provider, "openai");
    }

    #[tokio::test]
    async fn set_then_delete_provider_budget() {
        let state = build_test_state(vec![]);
        let req = SetProviderBudgetRequest {
            daily_budget_cents: Some(200),
            monthly_budget_cents: Some(6000),
        };
        let _ = set_provider_budget(
            State(state.clone()),
            Path("anthropic".to_string()),
            Json(req),
        )
        .await;

        let delete_response =
            delete_provider_budget(State(state.clone()), Path("anthropic".to_string())).await;
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let list_result = list_provider_budgets(State(state)).await;
        assert!(list_result.data.is_empty());
    }
}
