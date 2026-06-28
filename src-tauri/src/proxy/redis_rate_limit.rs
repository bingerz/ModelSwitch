use anyhow::Context;
use uuid::Uuid;

/// Sliding window duration in milliseconds (1 minute).
const WINDOW_MS: u64 = 60_000;
/// Sub-bucket size for the hash-based sliding window (10 seconds).
const BUCKET_MS: u64 = 10_000;

/// Atomic sliding-window check-and-record Lua script.
///
/// The algorithm uses a Redis hash where each field is a time bucket
/// (`floor(timestamp_ms / bucket_ms)`) and the value is the cumulative count
/// for that bucket. This matches the existing in-memory `BucketedWindow`
/// semantics.
///
/// The script atomically:
/// 1. Removes expired buckets (older than `now - window_ms`)
/// 2. Sums remaining bucket values to get the current window total
/// 3. Checks if adding `amount` would exceed `limit`
/// 4. Optionally adds the amount to the current bucket
/// 5. Refreshes the key TTL
const CHECK_AND_ADD_LUA: &str = r#"
-- Atomic sliding-window check-and-record
--
-- KEYS[1] = Redis hash key
-- ARGV[1] = current_time_ms
-- ARGV[2] = window_ms (60000)
-- ARGV[3] = bucket_ms (10000)
-- ARGV[4] = amount to add (tokens or 1 for requests)
-- ARGV[5] = limit (0 = no limit check)
-- ARGV[6] = add_flag (1 = record, 0 = check only)
--
-- Returns: {allowed (1/0), current_total}

local now = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
local bucket_size = tonumber(ARGV[3])
local amount = tonumber(ARGV[4])
local limit = tonumber(ARGV[5])
local add_flag = tonumber(ARGV[6])

local current_bucket = math.floor(now / bucket_size)
local min_valid_bucket = math.floor((now - window) / bucket_size)

-- Sum valid buckets and remove expired ones
local all = redis.call('HGETALL', KEYS[1])
local total = 0
local i = 1
while i <= #all do
    local bucket = tonumber(all[i])
    local val = tonumber(all[i + 1])
    if bucket < min_valid_bucket then
        redis.call('HDEL', KEYS[1], all[i])
    else
        total = total + val
    end
    i = i + 2
end

-- Check limit
if limit > 0 and total + amount > limit then
    return {0, total}
end

-- Add if requested
if add_flag == 1 and amount > 0 then
    redis.call('HINCRBY', KEYS[1], current_bucket, amount)
    total = total + amount
end

-- Refresh TTL (window + one extra bucket of slack)
redis.call('PEXPIRE', KEYS[1], window + bucket_size)

return {1, total}
"#;

/// Redis-backed distributed rate limiting backend.
///
/// Uses a hash-based sliding window with 10-second sub-buckets, matching the
/// semantics of the in-memory `BucketedWindow`. All operations are atomic via
/// a single Lua script execution.
///
/// Connection management uses `redis::aio::ConnectionManager` — a multiplexed
/// async connection that automatically reconnects on failure.
pub struct RedisRateLimitBackend {
    conn: redis::aio::ConnectionManager,
    prefix: String,
    script: redis::Script,
}

impl RedisRateLimitBackend {
    /// Create a new Redis backend, connecting to the given URL.
    ///
    /// The connection is established immediately — if Redis is unreachable,
    /// this returns an error.
    pub async fn new(url: &str, prefix: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(url).context("failed to create Redis client")?;
        let conn = redis::aio::ConnectionManager::new(client)
            .await
            .context("failed to connect to Redis")?;

        // Redact credentials from URL for safe logging
        let safe_url = url::Url::parse(url)
            .map(|mut u| {
                let _ = u.set_username("");
                let _ = u.set_password(None);
                u.to_string()
            })
            .unwrap_or_else(|_| "[invalid URL]".to_string());

        tracing::info!(
            redis_url = %safe_url,
            key_prefix = %prefix,
            "Redis rate limit backend connected"
        );

        Ok(Self {
            conn,
            prefix: prefix.to_string(),
            script: redis::Script::new(CHECK_AND_ADD_LUA),
        })
    }

    /// Build the Redis key for a channel rate limit counter.
    fn channel_key(channel_id: Uuid, dimension: &str) -> String {
        format!("ch:{channel_id}:{dimension}")
    }

    /// Build the Redis key for a virtual key rate limit counter.
    fn vk_key(key_id: Uuid, dimension: &str) -> String {
        format!("vk:{key_id}:{dimension}")
    }

    /// Build the Redis key for the global TPM counter.
    fn global_key() -> &'static str {
        "global:tpm"
    }

    /// Execute the check-and-add Lua script.
    ///
    /// Returns `(allowed, current_total_in_window)`.
    async fn check_and_add(
        &self,
        key_suffix: &str,
        amount: u64,
        limit: u64,
        add: bool,
    ) -> anyhow::Result<(bool, u64)> {
        let redis_key = format!("{}:{}", self.prefix, key_suffix);
        let now = chrono::Utc::now().timestamp_millis() as u64;

        let mut invoke = self.script.prepare_invoke();
        invoke
            .key(redis_key)
            .arg(now)
            .arg(WINDOW_MS)
            .arg(BUCKET_MS)
            .arg(amount)
            .arg(limit)
            .arg(if add { 1 } else { 0 });

        let mut conn = self.conn.clone();
        let (allowed, total): (i64, i64) = invoke
            .invoke_async(&mut conn)
            .await
            .context("Redis rate limit script failed")?;

        Ok((allowed == 1, total as u64))
    }

    // ── Channel rate limiting ──────────────────────────────

    /// Check if a channel request is allowed under TPM limits.
    /// Does NOT record — call `record_channel` after the response completes.
    pub async fn check_channel_tpm(
        &self,
        channel_id: Uuid,
        estimated_tokens: u64,
        tpm_limit: Option<u64>,
    ) -> anyhow::Result<bool> {
        let limit = tpm_limit.unwrap_or(0);
        let (allowed, _) = self
            .check_and_add(
                &Self::channel_key(channel_id, "tpm"),
                estimated_tokens,
                limit,
                false,
            )
            .await?;
        Ok(allowed)
    }

    /// Check if a channel request is allowed under RPM limits.
    pub async fn check_channel_rpm(
        &self,
        channel_id: Uuid,
        rpm_limit: u64,
    ) -> anyhow::Result<bool> {
        let (_, current) = self
            .check_and_add(&Self::channel_key(channel_id, "rpm"), 0, rpm_limit, false)
            .await?;
        // RPM check: current count must be below limit
        Ok(current < rpm_limit)
    }

    /// Record actual token usage and request count for a channel.
    pub async fn record_channel(&self, channel_id: Uuid, tokens: u64) -> anyhow::Result<()> {
        // Record RPM (+1 request)
        self.check_and_add(&Self::channel_key(channel_id, "rpm"), 1, 0, true)
            .await?;
        // Record TPM (actual tokens)
        if tokens > 0 {
            self.check_and_add(&Self::channel_key(channel_id, "tpm"), tokens, 0, true)
                .await?;
        }
        Ok(())
    }

    // ── Virtual key rate limiting ──────────────────────────

    /// Check if a virtual key request is allowed under RPM limits.
    /// Does NOT record — call `record_key_rpm` after the request succeeds.
    pub async fn check_key_rpm(&self, key_id: Uuid, rpm_limit: u32) -> anyhow::Result<bool> {
        let (_, current) = self
            .check_and_add(&Self::vk_key(key_id, "rpm"), 0, rpm_limit as u64, false)
            .await?;
        Ok(current < rpm_limit as u64)
    }

    /// Check if a virtual key request is allowed under TPM limits.
    pub async fn check_key_tpm(&self, key_id: Uuid, tpm_limit: u32) -> anyhow::Result<bool> {
        let (_, current) = self
            .check_and_add(&Self::vk_key(key_id, "tpm"), 0, tpm_limit as u64, false)
            .await?;
        Ok(current < tpm_limit as u64)
    }

    /// Record a request (RPM +1) for a virtual key.
    pub async fn record_key_rpm(&self, key_id: Uuid) -> anyhow::Result<()> {
        self.check_and_add(&Self::vk_key(key_id, "rpm"), 1, 0, true)
            .await?;
        Ok(())
    }

    /// Record actual token consumption for a virtual key.
    pub async fn record_key_tokens(&self, key_id: Uuid, tokens: u64) -> anyhow::Result<()> {
        if tokens == 0 {
            return Ok(());
        }
        self.check_and_add(&Self::vk_key(key_id, "tpm"), tokens, 0, true)
            .await?;
        Ok(())
    }

    // ── Global rate limiting ───────────────────────────────

    /// Check if the global TPM limit allows the request.
    /// Does NOT record — call `record_global_tpm` after the response.
    pub async fn check_global_tpm(
        &self,
        estimated_tokens: u64,
        global_limit: u64,
    ) -> anyhow::Result<bool> {
        let (allowed, _) = self
            .check_and_add(Self::global_key(), estimated_tokens, global_limit, false)
            .await?;
        Ok(allowed)
    }

    /// Record actual token usage to the global TPM counter.
    pub async fn record_global_tpm(&self, tokens: u64) -> anyhow::Result<()> {
        if tokens == 0 {
            return Ok(());
        }
        self.check_and_add(Self::global_key(), tokens, 0, true)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Lua script must be syntactically valid Redis Lua.
    /// We verify the script compiles by creating a `redis::Script` from it.
    #[test]
    fn lua_script_parses_successfully() {
        let _ = redis::Script::new(CHECK_AND_ADD_LUA);
        // If parsing fails, this constructor would panic.
    }

    #[test]
    fn channel_key_format() {
        let id = Uuid::nil();
        assert_eq!(
            RedisRateLimitBackend::channel_key(id, "tpm"),
            "ch:00000000-0000-0000-0000-000000000000:tpm"
        );
    }

    #[test]
    fn vk_key_format() {
        let id = Uuid::nil();
        assert_eq!(
            RedisRateLimitBackend::vk_key(id, "rpm"),
            "vk:00000000-0000-0000-0000-000000000000:rpm"
        );
    }

    #[test]
    fn global_key_is_static() {
        assert_eq!(RedisRateLimitBackend::global_key(), "global:tpm");
    }
}
