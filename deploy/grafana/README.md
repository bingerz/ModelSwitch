# ModelSwitch Grafana Dashboard

## Import

1. Open Grafana → **Dashboards** → **New** → **Import**.
2. Upload `modelswitch-dashboard.json` (or paste its contents).
3. Select your Prometheus datasource when prompted.

## Templating Variables

The dashboard ships with two multi-select variables:

- **Provider** — filters by upstream provider label (`label_values(modelswitch_requests_total, provider)`).
- **Model** — filters by model label (`label_values(modelswitch_requests_total, model)`).

Both default to "All" so the dashboard shows everything on first load.

## Panels

| Panel | Metric |
|-------|--------|
| Request Rate (QPS) | `rate(modelswitch_requests_total[5m])` grouped by provider/model |
| Latency P50 / P99 | `histogram_quantile()` over `modelswitch_request_latency_seconds_bucket` |
| Error Rate | 5xx responses divided by total, grouped by provider |
| Token Usage | `rate(modelswitch_tokens_total[5m])` split by input/output direction |
| Cost per Hour | `rate(modelswitch_cost_usd_total[1h])` grouped by provider |
| Active Requests | `sum(modelswitch_active_requests)` gauge |
| Circuit Breaker | `modelswitch_circuit_breaker_open` stat panel (CLOSED / OPEN) |
| Cache Hit Rate | hits / (hits + misses) over 5-minute windows |

## Prometheus Alerts

Load `../prometheus/alerts.yml` in your Prometheus config:

```yaml
rule_files:
  - /path/to/deploy/prometheus/alerts.yml
```

Then reload Prometheus. Three rules are defined:

- **HighErrorRate** — 5xx ratio exceeds 5 % for 5 minutes (warning).
- **HighLatencyP99** — P99 latency exceeds 2 seconds for 5 minutes (warning).
- **ChannelCircuitOpen** — any channel circuit breaker stays open for 5 minutes (critical).
