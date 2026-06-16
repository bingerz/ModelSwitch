use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

const MAX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10MB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchLog {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub request_model: String,
    pub channel_id: Uuid,
    pub channel_name: String,
    pub channel_priority: u8,
    pub retry_count: u8,
    pub trigger_reason: Option<String>,
    pub latency_ms: u64,
    pub success: bool,
    pub estimated_cost: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_hit_tokens: Option<u64>,
    pub cache_miss_tokens: Option<u64>,
    pub request_id: Option<String>,
}

pub struct DispatchLogger {
    logs: Arc<RwLock<VecDeque<DispatchLog>>>,
    max_entries: usize,
    log_file: Option<PathBuf>,
}

impl DispatchLogger {
    pub fn new(max_entries: usize) -> Self {
        Self {
            logs: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            log_file: None,
        }
    }

    pub fn with_persistence(max_entries: usize, log_file: PathBuf) -> Self {
        Self {
            logs: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            log_file: Some(log_file),
        }
    }

    pub async fn load_from_file(&self) {
        let path = match &self.log_file {
            Some(p) => p.clone(),
            None => return,
        };

        if let Err(e) = Self::ensure_parent_dir(&path).await {
            tracing::warn!("failed to create log directory: {e}");
            return;
        }

        if let Err(e) = Self::rotate_if_oversized(&path).await {
            tracing::warn!("failed to rotate log file: {e}");
        }

        let entries = match Self::read_entries(&path).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!("failed to load log file: {e}");
                return;
            }
        };

        let mut logs = self.logs.write().await;
        let start = entries.len().saturating_sub(self.max_entries);
        for entry in entries.into_iter().skip(start) {
            if logs.len() >= self.max_entries {
                break;
            }
            logs.push_back(entry);
        }

        tracing::info!(loaded = logs.len(), "dispatch logs loaded from file");
    }

    pub async fn log(&self, entry: DispatchLog) {
        if let Some(ref path) = self.log_file {
            let line = match serde_json::to_string(&entry) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("failed to serialize log entry: {e}");
                    String::new()
                }
            };
            if !line.is_empty() {
                let file_path = path.clone();
                crate::spawn_bg(async move {
                    let _ = Self::append_line(&file_path, &line).await;
                });
            }
        }

        let mut logs = self.logs.write().await;
        if logs.len() >= self.max_entries {
            logs.pop_front();
        }
        logs.push_back(entry);
    }

    /// Retroactively update token usage and cost for a specific log entry.
    pub async fn update_log_tokens(
        &self,
        id: Uuid,
        input: u64,
        output: u64,
        cache_hit: Option<u64>,
        cache_miss: Option<u64>,
        exact_cost: Option<f64>,
    ) {
        let mut logs = self.logs.write().await;
        if let Some(log) = logs.iter_mut().rev().find(|l| l.id == id) {
            log.input_tokens = Some(input);
            log.output_tokens = Some(output);
            if cache_hit.is_some() {
                log.cache_hit_tokens = cache_hit;
            }
            if cache_miss.is_some() {
                log.cache_miss_tokens = cache_miss;
            }
            if exact_cost.is_some() {
                log.estimated_cost = exact_cost;
            }
        }
    }

    pub async fn list(&self, offset: usize, limit: usize) -> Vec<DispatchLog> {
        let logs = self.logs.read().await;
        logs.iter()
            .rev()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Total number of log entries stored.
    pub async fn total(&self) -> usize {
        self.logs.read().await.len()
    }

    pub async fn stats(&self) -> DispatchStats {
        let logs = self.logs.read().await;
        let total = logs.len();
        let successes = logs.iter().filter(|l| l.success).count();
        let failures = total - successes;
        let avg_latency = if total > 0 {
            logs.iter().map(|l| l.latency_ms).sum::<u64>() / total as u64
        } else {
            0
        };

        DispatchStats {
            total_requests: total,
            successes,
            failures,
            avg_latency_ms: avg_latency,
        }
    }

    /// Aggregate token usage into hourly buckets per channel + model.
    /// Skips placeholder logs from streaming (those with no real token data).
    pub async fn usage_history(&self, hours: u64) -> UsageHistory {
        let cutoff = Utc::now() - chrono::Duration::hours(hours as i64);
        let logs = self.logs.read().await;

        // Key: (hour, channel_id, model)
        let mut buckets: BTreeMap<(DateTime<Utc>, Uuid, String), UsageBucket> = BTreeMap::new();

        for log in logs.iter().filter(|l| {
            // Skip streaming placeholder logs — they have no real token data
            l.success
                && l.timestamp >= cutoff
                && (l.input_tokens.is_some() || l.output_tokens.is_some())
        }) {
            let hour = log
                .timestamp
                .with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap();
            let key = (hour, log.channel_id, log.request_model.clone());

            let bucket = buckets.entry(key).or_insert_with(|| UsageBucket {
                timestamp: hour,
                channel_id: log.channel_id,
                channel_name: log.channel_name.clone(),
                model: log.request_model.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
                request_count: 0,
                estimated_cost: 0.0,
            });

            bucket.input_tokens += log.input_tokens.unwrap_or(0);
            bucket.output_tokens += log.output_tokens.unwrap_or(0);
            bucket.cache_hit_tokens += log.cache_hit_tokens.unwrap_or(0);
            bucket.cache_miss_tokens += log.cache_miss_tokens.unwrap_or(0);
            bucket.request_count += 1;
            bucket.estimated_cost += log.estimated_cost.unwrap_or(0.0);
        }

        let buckets: Vec<UsageBucket> = buckets.into_values().collect();
        let total_input_tokens = buckets.iter().map(|b| b.input_tokens).sum();
        let total_output_tokens = buckets.iter().map(|b| b.output_tokens).sum();
        let total_requests = buckets.iter().map(|b| b.request_count).sum();
        let total_cost = buckets.iter().map(|b| b.estimated_cost).sum();

        UsageHistory {
            buckets,
            total_input_tokens,
            total_output_tokens,
            total_requests,
            total_cost,
        }
    }

    pub async fn cost_stats(&self) -> CostStats {
        let logs = self.logs.read().await;
        let successful: Vec<&DispatchLog> = logs.iter().filter(|l| l.success).collect();

        let total_cost: f64 = successful.iter().filter_map(|l| l.estimated_cost).sum();

        let total_requests = successful.len();

        // Per-priority breakdown
        let mut priority_breakdown: BTreeMap<u8, PriorityStats> = BTreeMap::new();
        for log in &successful {
            let priority = log.channel_priority;
            let entry = priority_breakdown
                .entry(priority)
                .or_insert_with(|| PriorityStats {
                    priority,
                    requests: 0,
                    estimated_cost: 0.0,
                });
            entry.requests += 1;
            if let Some(cost) = log.estimated_cost {
                entry.estimated_cost += cost;
            }
        }

        // Per-model breakdown
        let mut model_counts: BTreeMap<String, usize> = BTreeMap::new();
        for log in &successful {
            *model_counts.entry(log.request_model.clone()).or_insert(0) += 1;
        }

        let total_input_tokens: u64 = successful.iter().filter_map(|l| l.input_tokens).sum();
        let total_output_tokens: u64 = successful.iter().filter_map(|l| l.output_tokens).sum();

        CostStats {
            total_requests,
            total_estimated_cost: total_cost,
            priority_breakdown: priority_breakdown.into_values().collect(),
            model_counts,
            total_input_tokens,
            total_output_tokens,
        }
    }
}

// Private helpers for file persistence
impl DispatchLogger {
    async fn ensure_parent_dir(path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        Ok(())
    }

    async fn rotate_if_oversized(path: &PathBuf) -> std::io::Result<()> {
        match tokio::fs::metadata(path).await {
            Ok(meta) if meta.len() > MAX_FILE_SIZE_BYTES => {
                let rotated = Self::rotated_path(path);
                let _ = tokio::fs::remove_file(&rotated).await;
                tokio::fs::rename(path, &rotated).await?;
                tracing::info!(
                    from = %path.display(),
                    to = %rotated.display(),
                    "rotated oversized log file"
                );
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(())
    }

    async fn read_entries(path: &PathBuf) -> std::io::Result<Vec<DispatchLog>> {
        let content = tokio::fs::read_to_string(path).await?;
        let mut entries = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<DispatchLog>(trimmed) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::debug!("skipping malformed log line: {e}");
                }
            }
        }
        Ok(entries)
    }

    async fn append_line(path: &PathBuf, line: &str) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(())
    }

    fn rotated_path(path: &Path) -> PathBuf {
        path.with_extension("ndjson.1")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchStats {
    pub total_requests: usize,
    pub successes: usize,
    pub failures: usize,
    pub avg_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostStats {
    pub total_requests: usize,
    pub total_estimated_cost: f64,
    pub priority_breakdown: Vec<PriorityStats>,
    pub model_counts: BTreeMap<String, usize>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityStats {
    pub priority: u8,
    pub requests: usize,
    pub estimated_cost: f64,
}

/// A single time-bucketed usage aggregate (one hour, one channel, one model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageBucket {
    /// Bucket start time (truncated to hour)
    pub timestamp: DateTime<Utc>,
    pub channel_id: Uuid,
    pub channel_name: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_hit_tokens: u64,
    pub cache_miss_tokens: u64,
    pub request_count: u32,
    pub estimated_cost: f64,
}

/// Aggregated usage history returned by `usage_history()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageHistory {
    pub buckets: Vec<UsageBucket>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_requests: u32,
    pub total_cost: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn usage_history_aggregates_by_hour_and_channel() {
        let logger = DispatchLogger::new(100);
        let ch1 = Uuid::new_v4();
        let ch2 = Uuid::new_v4();
        let now = Utc::now()
            .with_minute(5)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();
        let one_hour_ago = now - chrono::Duration::hours(1);
        let two_hours_ago = now - chrono::Duration::hours(2);

        // ch1: 2 requests in same hour
        logger
            .log(DispatchLog {
                id: Uuid::new_v4(),
                timestamp: one_hour_ago,
                request_model: "deepseek-chat".into(),
                channel_id: ch1,
                channel_name: "DeepSeek-1".into(),
                channel_priority: 1,
                retry_count: 0,
                trigger_reason: None,
                latency_ms: 100,
                success: true,
                estimated_cost: Some(0.01),
                input_tokens: Some(500),
                output_tokens: Some(200),
                cache_hit_tokens: None,
                cache_miss_tokens: None,
                request_id: None,
            })
            .await;
        logger
            .log(DispatchLog {
                id: Uuid::new_v4(),
                timestamp: one_hour_ago + chrono::Duration::minutes(10),
                request_model: "deepseek-chat".into(),
                channel_id: ch1,
                channel_name: "DeepSeek-1".into(),
                channel_priority: 1,
                retry_count: 0,
                trigger_reason: None,
                latency_ms: 150,
                success: true,
                estimated_cost: Some(0.02),
                input_tokens: Some(300),
                output_tokens: Some(100),
                cache_hit_tokens: None,
                cache_miss_tokens: None,
                request_id: None,
            })
            .await;

        // ch2: 1 request in different hour
        logger
            .log(DispatchLog {
                id: Uuid::new_v4(),
                timestamp: two_hours_ago,
                request_model: "claude-3".into(),
                channel_id: ch2,
                channel_name: "Anthropic".into(),
                channel_priority: 2,
                retry_count: 0,
                trigger_reason: None,
                latency_ms: 200,
                success: true,
                estimated_cost: Some(0.05),
                input_tokens: Some(1000),
                output_tokens: Some(500),
                cache_hit_tokens: None,
                cache_miss_tokens: None,
                request_id: None,
            })
            .await;

        // Failed request should be excluded
        logger
            .log(DispatchLog {
                id: Uuid::new_v4(),
                timestamp: now,
                request_model: "deepseek-chat".into(),
                channel_id: ch1,
                channel_name: "DeepSeek-1".into(),
                channel_priority: 1,
                retry_count: 0,
                trigger_reason: None,
                latency_ms: 50,
                success: false,
                estimated_cost: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                cache_hit_tokens: None,
                cache_miss_tokens: None,
                request_id: None,
            })
            .await;

        let history = logger.usage_history(24).await;

        // 2 buckets: ch1 in hour-1, ch2 in hour-2
        assert_eq!(history.buckets.len(), 2);

        // Totals
        assert_eq!(history.total_input_tokens, 500 + 300 + 1000);
        assert_eq!(history.total_output_tokens, 200 + 100 + 500);
        assert_eq!(history.total_requests, 3);
        assert!((history.total_cost - 0.08).abs() < 0.001);
    }

    #[tokio::test]
    async fn usage_history_respects_hours_param() {
        let logger = DispatchLogger::new(100);
        let ch = Uuid::new_v4();
        let now = Utc::now()
            .with_minute(5)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();

        // Request 3 hours ago
        logger
            .log(DispatchLog {
                id: Uuid::new_v4(),
                timestamp: now - chrono::Duration::hours(3),
                request_model: "test".into(),
                channel_id: ch,
                channel_name: "Test".into(),
                channel_priority: 1,
                retry_count: 0,
                trigger_reason: None,
                latency_ms: 100,
                success: true,
                estimated_cost: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                cache_hit_tokens: None,
                cache_miss_tokens: None,
                request_id: None,
            })
            .await;

        // 2-hour window: should exclude the 3-hour-old entry
        let h2 = logger.usage_history(2).await;
        assert_eq!(h2.buckets.len(), 0);
        assert_eq!(h2.total_requests, 0);

        // 4-hour window: should include it
        let h4 = logger.usage_history(4).await;
        assert_eq!(h4.buckets.len(), 1);
        assert_eq!(h4.total_requests, 1);
    }
}
