use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

const MAX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10MB
/// Check file size for rotation every N writes to amortize stat() cost.
const ROTATION_CHECK_INTERVAL: u64 = 100;

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
    /// Virtual key ID that authenticated the request, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub virtual_key_id: Option<String>,
}

pub struct DispatchLogger {
    logs: Arc<RwLock<VecDeque<DispatchLog>>>,
    max_entries: usize,
    log_file: Option<PathBuf>,
    max_file_size_bytes: u64,
    max_file_count: usize,
    /// Monotonic write counter — rotation check runs every N writes.
    write_counter: Arc<std::sync::atomic::AtomicU64>,
}

impl DispatchLogger {
    pub fn new(max_entries: usize) -> Self {
        Self {
            logs: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            log_file: None,
            max_file_size_bytes: MAX_FILE_SIZE_BYTES,
            max_file_count: 1,
            write_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    pub fn with_persistence(
        max_entries: usize,
        log_file: PathBuf,
        max_file_size_mb: u64,
        max_file_count: usize,
    ) -> Self {
        Self {
            logs: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            max_file_size_bytes: max_file_size_mb.saturating_mul(1024 * 1024),
            max_file_count: max_file_count.max(1),
            log_file: Some(log_file),
            write_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
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

        if let Err(e) =
            Self::rotate_if_needed(&path, self.max_file_size_bytes, self.max_file_count).await
        {
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
                let max_bytes = self.max_file_size_bytes;
                let max_count = self.max_file_count;
                // Increment counter and check if rotation is due. Using
                // fetch_add + modulo to amortize stat() overhead — only every
                // ROTATION_CHECK_INTERVAL writes trigger a file size check.
                let count = self
                    .write_counter
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let check_rotation = count.is_multiple_of(ROTATION_CHECK_INTERVAL);
                crate::spawn_bg(async move {
                    if check_rotation {
                        if let Err(e) =
                            Self::rotate_if_needed(&file_path, max_bytes, max_count).await
                        {
                            tracing::warn!("failed to rotate log file: {e}");
                        }
                    }
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

    /// Return logs filtered by `virtual_key_id`, with pagination.
    /// Only entries whose `virtual_key_id` matches `key_id` are returned.
    pub async fn list_by_key(&self, key_id: &str, offset: usize, limit: usize) -> Vec<DispatchLog> {
        let logs = self.logs.read().await;
        logs.iter()
            .rev()
            .filter(|l| l.virtual_key_id.as_deref() == Some(key_id))
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Total number of log entries stored.
    pub async fn total(&self) -> usize {
        self.logs.read().await.len()
    }

    /// Total number of log entries matching the given `virtual_key_id`.
    pub async fn total_by_key(&self, key_id: &str) -> usize {
        let logs = self.logs.read().await;
        logs.iter()
            .filter(|l| l.virtual_key_id.as_deref() == Some(key_id))
            .count()
    }

    pub async fn stats(&self) -> DispatchStats {
        // Snapshot only the fields we need, then release the lock
        let (total, snapshot) = {
            let logs = self.logs.read().await;
            let total = logs.len();
            let snapshot: Vec<(bool, u64)> =
                logs.iter().map(|l| (l.success, l.latency_ms)).collect();
            (total, snapshot)
        };

        let successes = snapshot.iter().filter(|(s, _)| *s).count();
        let failures = total - successes;
        let avg_latency = if total > 0 {
            snapshot.iter().map(|(_, lat)| *lat).sum::<u64>() / total as u64
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

        // Snapshot matching entries under the read lock, then release
        let matching: Vec<DispatchLog> = {
            let logs = self.logs.read().await;
            logs.iter()
                .filter(|l| {
                    // Skip streaming placeholder logs — they have no real token data
                    l.success
                        && l.timestamp >= cutoff
                        && (l.input_tokens.is_some() || l.output_tokens.is_some())
                })
                .cloned()
                .collect()
        };

        // Key: (hour, channel_id, model)
        let mut buckets: BTreeMap<(DateTime<Utc>, Uuid, String), UsageBucket> = BTreeMap::new();

        for log in &matching {
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
        // Snapshot successful entries under the read lock, then release
        let successful: Vec<DispatchLog> = {
            let logs = self.logs.read().await;
            logs.iter().filter(|l| l.success).cloned().collect()
        };

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

    /// Rotate the log file if it exceeds `max_bytes`.
    /// Rotates: `logs.ndjson` → `logs.ndjson.1` → `logs.ndjson.2` → ...
    /// Files beyond `max_count` are deleted.
    async fn rotate_if_needed(
        path: &Path,
        max_bytes: u64,
        max_count: usize,
    ) -> std::io::Result<()> {
        let needs_rotation = match tokio::fs::metadata(path).await {
            Ok(meta) => meta.len() > max_bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        if !needs_rotation {
            return Ok(());
        }

        // Delete the oldest backup if it would exceed max_count after shift.
        // After shifting, files are numbered .1 through .max_count.
        // The oldest (max_count) is deleted to make room.
        let oldest = Self::rotated_path_with_index(path, max_count);
        let _ = tokio::fs::remove_file(&oldest).await;

        // Shift existing backups: .(n-1) → .n, for n = max_count-1 down to 1.
        // Process in descending order to avoid overwriting.
        for i in (1..max_count).rev() {
            let from = Self::rotated_path_with_index(path, i);
            let to = Self::rotated_path_with_index(path, i + 1);
            // Ignore errors — file may not exist yet
            let _ = tokio::fs::rename(&from, &to).await;
        }

        // Rotate current file to .1
        let first_backup = Self::rotated_path_with_index(path, 1);
        tokio::fs::rename(path, &first_backup).await?;
        tracing::info!(
            from = %path.display(),
            to = %first_backup.display(),
            max_bytes,
            max_count,
            "rotated log file"
        );

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

    /// Build the rotated backup path for a given index (1-based).
    /// e.g., `/path/logs.ndjson` with index 2 → `/path/logs.ndjson.2`
    fn rotated_path_with_index(path: &Path, index: usize) -> PathBuf {
        let mut new_path = path.as_os_str().to_os_string();
        new_path.push(format!(".{index}"));
        PathBuf::from(new_path)
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
                virtual_key_id: None,
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
                virtual_key_id: None,
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
                virtual_key_id: None,
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
                virtual_key_id: None,
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
                virtual_key_id: None,
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

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    /// Build a DispatchLog with sensible defaults for tests.
    fn make_log(id: Uuid, success: bool, latency_ms: u64) -> DispatchLog {
        DispatchLog {
            id,
            timestamp: Utc::now(),
            request_model: "test-model".into(),
            channel_id: Uuid::new_v4(),
            channel_name: "test-channel".into(),
            channel_priority: 1,
            retry_count: 0,
            trigger_reason: None,
            latency_ms,
            success,
            estimated_cost: None,
            input_tokens: None,
            output_tokens: None,
            cache_hit_tokens: None,
            cache_miss_tokens: None,
            request_id: None,
            virtual_key_id: None,
        }
    }

    /// Like `make_log` but also sets cost/token fields.
    fn make_cost_log(
        id: Uuid,
        success: bool,
        cost: Option<f64>,
        input: Option<u64>,
        output: Option<u64>,
    ) -> DispatchLog {
        let mut l = make_log(id, success, 100);
        l.estimated_cost = cost;
        l.input_tokens = input;
        l.output_tokens = output;
        l
    }

    /// Generate a unique file path inside the OS temp directory.
    fn unique_temp_file(prefix: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("{}_{}.ndjson", prefix, Uuid::new_v4()));
        p
    }

    /// Check whether a path currently exists (tolerates races).
    async fn path_exists(path: &Path) -> bool {
        tokio::fs::metadata(path).await.is_ok()
    }

    // ---------------------------------------------------------------------------
    // In-memory logging
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn new_logger_is_empty() {
        let logger = DispatchLogger::new(100);
        assert_eq!(logger.total().await, 0);
        assert!(logger.list(0, 10).await.is_empty());

        let stats = logger.stats().await;
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.successes, 0);
        assert_eq!(stats.failures, 0);
        assert_eq!(stats.avg_latency_ms, 0);
    }

    #[tokio::test]
    async fn log_appends_single_entry() {
        let logger = DispatchLogger::new(100);
        let id = Uuid::new_v4();
        logger.log(make_log(id, true, 42)).await;

        assert_eq!(logger.total().await, 1);
        let entries = logger.list(0, 10).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].latency_ms, 42);
        assert!(entries[0].success);
    }

    #[tokio::test]
    async fn log_evicts_oldest_when_max_exceeded() {
        let logger = DispatchLogger::new(3);
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let id4 = Uuid::new_v4();

        logger.log(make_log(id1, true, 10)).await;
        logger.log(make_log(id2, true, 20)).await;
        logger.log(make_log(id3, true, 30)).await;
        logger.log(make_log(id4, true, 40)).await;

        // FIFO eviction: id1 should be gone, the rest retained.
        assert_eq!(logger.total().await, 3);
        let entries = logger.list(0, 10).await;
        assert_eq!(entries[0].id, id4); // newest first
        assert_eq!(entries[1].id, id3);
        assert_eq!(entries[2].id, id2);
        assert!(entries.iter().all(|e| e.id != id1));
    }

    #[tokio::test]
    async fn list_returns_newest_first_with_pagination() {
        let logger = DispatchLogger::new(10);
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        logger.log(make_log(id1, true, 10)).await;
        logger.log(make_log(id2, true, 20)).await;
        logger.log(make_log(id3, true, 30)).await;

        // Newest first by default
        let all = logger.list(0, 10).await;
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, id3);
        assert_eq!(all[1].id, id2);
        assert_eq!(all[2].id, id1);

        // Page: offset 1, limit 1 → only id2
        let page = logger.list(1, 1).await;
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, id2);

        // Offset beyond available entries
        assert!(logger.list(10, 5).await.is_empty());
    }

    #[tokio::test]
    async fn stats_reports_counts_and_avg_latency() {
        let logger = DispatchLogger::new(100);
        logger.log(make_log(Uuid::new_v4(), true, 100)).await;
        logger.log(make_log(Uuid::new_v4(), true, 200)).await;
        logger.log(make_log(Uuid::new_v4(), false, 50)).await;

        let stats = logger.stats().await;
        assert_eq!(stats.total_requests, 3);
        assert_eq!(stats.successes, 2);
        assert_eq!(stats.failures, 1);
        assert_eq!(stats.avg_latency_ms, (100 + 200 + 50) / 3);
    }

    #[tokio::test]
    async fn cost_stats_aggregates_breakdowns() {
        let logger = DispatchLogger::new(100);

        let mut e1 = make_cost_log(Uuid::new_v4(), true, Some(0.1), Some(100), Some(50));
        e1.channel_priority = 1;
        e1.request_model = "gpt-4".into();
        logger.log(e1).await;

        let mut e2 = make_cost_log(Uuid::new_v4(), true, Some(0.2), Some(200), Some(100));
        e2.channel_priority = 1;
        e2.request_model = "gpt-4".into();
        logger.log(e2).await;

        let mut e3 = make_cost_log(Uuid::new_v4(), true, Some(0.3), Some(150), Some(75));
        e3.channel_priority = 2;
        e3.request_model = "claude-3".into();
        logger.log(e3).await;

        // Failed entry should be excluded from cost stats
        logger
            .log(make_cost_log(Uuid::new_v4(), false, None, None, None))
            .await;

        let cost = logger.cost_stats().await;
        assert_eq!(cost.total_requests, 3);
        assert!((cost.total_estimated_cost - 0.6).abs() < 0.001);
        assert_eq!(cost.total_input_tokens, 100 + 200 + 150);
        assert_eq!(cost.total_output_tokens, 50 + 100 + 75);
        assert_eq!(cost.model_counts.get("gpt-4").copied(), Some(2));
        assert_eq!(cost.model_counts.get("claude-3").copied(), Some(1));
        assert_eq!(cost.priority_breakdown.len(), 2);
    }

    // ---------------------------------------------------------------------------
    // Token updates
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn update_log_tokens_updates_matching_entry() {
        let logger = DispatchLogger::new(10);
        let id = Uuid::new_v4();
        logger.log(make_log(id, true, 50)).await;

        logger
            .update_log_tokens(id, 100, 50, Some(10), Some(5), Some(0.5))
            .await;

        let entries = logger.list(0, 10).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].input_tokens, Some(100));
        assert_eq!(entries[0].output_tokens, Some(50));
        assert_eq!(entries[0].cache_hit_tokens, Some(10));
        assert_eq!(entries[0].cache_miss_tokens, Some(5));
        assert_eq!(entries[0].estimated_cost, Some(0.5));
    }

    #[tokio::test]
    async fn update_log_tokens_noop_for_unknown_id() {
        let logger = DispatchLogger::new(10);
        let known = Uuid::new_v4();
        logger.log(make_log(known, true, 50)).await;

        // Update a non-existent ID — should not panic or modify anything
        let unknown = Uuid::new_v4();
        logger
            .update_log_tokens(unknown, 999, 888, Some(1), Some(2), Some(9.9))
            .await;

        let entries = logger.list(0, 10).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, known);
        assert!(entries[0].input_tokens.is_none());
        assert!(entries[0].cache_hit_tokens.is_none());
    }

    #[tokio::test]
    async fn update_log_tokens_safe_on_empty_logger() {
        let logger = DispatchLogger::new(10);
        // Must not panic
        logger
            .update_log_tokens(Uuid::new_v4(), 100, 50, None, None, None)
            .await;
        assert_eq!(logger.total().await, 0);
    }

    #[tokio::test]
    async fn update_log_tokens_partial_update_preserves_existing_fields() {
        let logger = DispatchLogger::new(10);
        let id = Uuid::new_v4();

        // Seed an entry with some cache/cost data
        let mut entry = make_log(id, true, 50);
        entry.cache_hit_tokens = Some(99);
        entry.cache_miss_tokens = Some(88);
        entry.estimated_cost = Some(1.0);
        logger.log(entry).await;

        // Partial update: only input/output set, cache/cost passed as None
        // (should preserve existing values, not overwrite with None)
        logger
            .update_log_tokens(id, 200, 100, None, None, None)
            .await;

        let entries = logger.list(0, 10).await;
        assert_eq!(entries[0].input_tokens, Some(200));
        assert_eq!(entries[0].output_tokens, Some(100));
        assert_eq!(entries[0].cache_hit_tokens, Some(99));
        assert_eq!(entries[0].cache_miss_tokens, Some(88));
        assert_eq!(entries[0].estimated_cost, Some(1.0));
    }

    // ---------------------------------------------------------------------------
    // Virtual-key filtering
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn list_by_key_filters_entries() {
        let logger = DispatchLogger::new(100);
        let key_a = "key-alice";
        let key_b = "key-bob";

        let mut e1 = make_log(Uuid::new_v4(), true, 10);
        e1.virtual_key_id = Some(key_a.into());
        let mut e2 = make_log(Uuid::new_v4(), true, 20);
        e2.virtual_key_id = Some(key_a.into());
        let mut e3 = make_log(Uuid::new_v4(), true, 30);
        e3.virtual_key_id = Some(key_b.into());

        logger.log(e1).await;
        logger.log(e2).await;
        logger.log(e3).await;

        let alice = logger.list_by_key(key_a, 0, 10).await;
        assert_eq!(alice.len(), 2);
        assert!(alice
            .iter()
            .all(|e| e.virtual_key_id.as_deref() == Some(key_a)));

        let bob = logger.list_by_key(key_b, 0, 10).await;
        assert_eq!(bob.len(), 1);

        assert!(logger.list_by_key("unknown", 0, 10).await.is_empty());
    }

    #[tokio::test]
    async fn total_by_key_counts_matching_entries_only() {
        let logger = DispatchLogger::new(100);
        for i in 0..5 {
            let mut e = make_log(Uuid::new_v4(), true, i * 10);
            e.virtual_key_id = Some("key-a".into());
            logger.log(e).await;
        }
        let mut e = make_log(Uuid::new_v4(), true, 99);
        e.virtual_key_id = Some("key-b".into());
        logger.log(e).await;

        assert_eq!(logger.total_by_key("key-a").await, 5);
        assert_eq!(logger.total_by_key("key-b").await, 1);
        assert_eq!(logger.total_by_key("nonexistent").await, 0);
    }

    // ---------------------------------------------------------------------------
    // File persistence
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn append_line_writes_and_appends_entries() {
        // append_line is the actual I/O path that log() calls via spawn_bg.
        // Testing it directly avoids background-task timing flakiness while
        // exercising the same serialization + file-append code.
        let path = unique_temp_file("append_line");
        DispatchLogger::ensure_parent_dir(&path).await.unwrap();

        let log1 = make_log(Uuid::new_v4(), true, 10);
        DispatchLogger::append_line(&path, &serde_json::to_string(&log1).unwrap())
            .await
            .unwrap();

        let log2 = make_log(Uuid::new_v4(), false, 20);
        DispatchLogger::append_line(&path, &serde_json::to_string(&log2).unwrap())
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 2, "file should contain 2 appended entries");

        // First entry should be on the first line (append order preserved).
        let parsed1: DispatchLog = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed1.id, log1.id);
        assert!(parsed1.success);

        let parsed2: DispatchLog = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(parsed2.id, log2.id);
        assert!(!parsed2.success);

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn with_persistence_logs_in_memory_and_configures_file() {
        // Verify that with_persistence creates a logger that still handles
        // in-memory logging correctly while being configured for disk writes.
        let path = unique_temp_file("persist_config");
        let logger = DispatchLogger::with_persistence(100, path.clone(), 10, 3);

        logger.log(make_log(Uuid::new_v4(), true, 10)).await;
        logger.log(make_log(Uuid::new_v4(), true, 20)).await;

        // In-memory state should reflect both entries immediately.
        assert_eq!(logger.total().await, 2);
        let entries = logger.list(0, 10).await;
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.success));

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn load_from_file_roundtrips_entries() {
        let path = unique_temp_file("load_roundtrip");
        let log1 = make_log(Uuid::new_v4(), true, 10);
        let log2 = make_log(Uuid::new_v4(), false, 99);
        let log3 = make_log(Uuid::new_v4(), true, 50);

        let contents = [
            serde_json::to_string(&log1).unwrap(),
            serde_json::to_string(&log2).unwrap(),
            serde_json::to_string(&log3).unwrap(),
        ]
        .join("\n");
        tokio::fs::write(&path, &contents).await.unwrap();

        let logger = DispatchLogger::with_persistence(100, path.clone(), 10, 3);
        logger.load_from_file().await;

        assert_eq!(logger.total().await, 3);
        let entries = logger.list(0, 10).await;
        // list() returns newest first
        assert_eq!(entries[0].id, log3.id);
        assert_eq!(entries[1].id, log2.id);
        assert_eq!(entries[2].id, log1.id);

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn load_from_file_truncates_to_max_entries() {
        let path = unique_temp_file("load_truncate");
        // Write 10 entries with distinct model names
        let logs: Vec<DispatchLog> = (0..10)
            .map(|i| {
                let mut l = make_log(Uuid::new_v4(), i % 2 == 0, (i * 10) as u64);
                l.request_model = format!("model-{i}");
                l
            })
            .collect();
        let contents: Vec<String> = logs
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect();
        tokio::fs::write(&path, contents.join("\n")).await.unwrap();

        // max_entries = 3 → only the 3 newest entries should be loaded.
        let logger = DispatchLogger::with_persistence(3, path.clone(), 10, 3);
        logger.load_from_file().await;

        assert_eq!(logger.total().await, 3);
        let entries = logger.list(0, 10).await;
        assert_eq!(entries[0].request_model, "model-9");
        assert_eq!(entries[1].request_model, "model-8");
        assert_eq!(entries[2].request_model, "model-7");

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn load_from_file_ignores_malformed_lines() {
        let path = unique_temp_file("load_malformed");
        let good = make_log(Uuid::new_v4(), true, 10);
        let good_json = serde_json::to_string(&good).unwrap();
        let contents = format!("{{not json\ngarbage line\n{good_json}\n");
        tokio::fs::write(&path, &contents).await.unwrap();

        let logger = DispatchLogger::with_persistence(100, path.clone(), 10, 3);
        logger.load_from_file().await;

        // Only the single valid line should be loaded
        assert_eq!(logger.total().await, 1);
        let entries = logger.list(0, 10).await;
        assert_eq!(entries[0].id, good.id);

        let _ = tokio::fs::remove_file(&path).await;
    }

    // ---------------------------------------------------------------------------
    // File rotation
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn rotate_if_needed_renames_oversized_file_to_backup() {
        let path = unique_temp_file("rotate_basic");
        let big_line = "x".repeat(600);
        tokio::fs::write(&path, &big_line).await.unwrap();

        DispatchLogger::rotate_if_needed(&path, 512, 3)
            .await
            .unwrap();

        // Original path should no longer exist
        assert!(!path_exists(&path).await);

        // Backup .1 should contain the original content
        let backup = DispatchLogger::rotated_path_with_index(&path, 1);
        let backup_content = tokio::fs::read_to_string(&backup).await.unwrap();
        assert_eq!(backup_content, big_line);

        let _ = tokio::fs::remove_file(&backup).await;
    }

    #[tokio::test]
    async fn rotate_if_needed_noop_for_small_file() {
        let path = unique_temp_file("rotate_noop");
        tokio::fs::write(&path, "small").await.unwrap();

        DispatchLogger::rotate_if_needed(&path, 1024, 2)
            .await
            .unwrap();

        // File should be untouched
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "small");
        // No backup should exist
        let backup = DispatchLogger::rotated_path_with_index(&path, 1);
        assert!(!path_exists(&backup).await);

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn rotate_if_needed_shifts_existing_backups() {
        let path = unique_temp_file("rotate_shift");

        // First rotation
        tokio::fs::write(&path, "y".repeat(512)).await.unwrap();
        DispatchLogger::rotate_if_needed(&path, 256, 2)
            .await
            .unwrap();
        let backup1 = DispatchLogger::rotated_path_with_index(&path, 1);
        assert!(path_exists(&backup1).await);

        // Second rotation — should shift .1 → .2
        tokio::fs::write(&path, "z".repeat(512)).await.unwrap();
        DispatchLogger::rotate_if_needed(&path, 256, 2)
            .await
            .unwrap();
        assert!(path_exists(&backup1).await);
        let backup2 = DispatchLogger::rotated_path_with_index(&path, 2);
        assert!(path_exists(&backup2).await);

        // .2 should contain the content from the first rotation
        let b2_content = tokio::fs::read_to_string(&backup2).await.unwrap();
        assert_eq!(b2_content, "y".repeat(512));

        for i in 1..=2 {
            let _ = tokio::fs::remove_file(DispatchLogger::rotated_path_with_index(&path, i)).await;
        }
    }

    // ---------------------------------------------------------------------------
    // Concurrency
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn concurrent_log_preserves_all_entries() {
        let logger = std::sync::Arc::new(DispatchLogger::new(200));
        let concurrency = 50;

        let mut handles = Vec::with_capacity(concurrency);
        for i in 0..concurrency {
            let log = std::sync::Arc::clone(&logger);
            handles.push(tokio::spawn(async move {
                log.log(make_log(Uuid::new_v4(), i % 2 == 0, i as u64))
                    .await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(logger.total().await, 50);
        let stats = logger.stats().await;
        assert_eq!(stats.total_requests, 50);
        // Even indices are successful (i % 2 == 0), odd are failures
        assert_eq!(stats.successes, 25);
        assert_eq!(stats.failures, 25);
    }
}
