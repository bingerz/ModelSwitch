import type { DispatchLog, UsageBucket } from "../../lib/api";

export interface HourPoint {
  hour: Date;
  requests: number;
  tokens: number;
  cost: number;
  channels: number;
}

export interface EntityStat {
  id: string;
  name: string;
  requests: number;
  tokens: number;
  cost: number;
}

export interface ChannelUsage {
  requests: number;
  tokens: number;
  cost: number;
  spark: number[];
}

const HOUR_MS = 3600_000;

function startOfHour(d: Date): Date {
  return new Date(Math.floor(d.getTime() / HOUR_MS) * HOUR_MS);
}

/** Group 24h usage buckets by their timestamp hour, sum key metrics. */
export function aggregateByHour(buckets: UsageBucket[]): HourPoint[] {
  const map = new Map<number, HourPoint>();
  for (const b of buckets) {
    const hour = startOfHour(new Date(b.timestamp)).getTime();
    const existing = map.get(hour);
    if (existing) {
      existing.requests += b.request_count;
      existing.tokens += b.input_tokens + b.output_tokens;
      existing.cost += b.estimated_cost;
    } else {
      map.set(hour, {
        hour: new Date(hour),
        requests: b.request_count,
        tokens: b.input_tokens + b.output_tokens,
        cost: b.estimated_cost,
        channels: 0,
      });
    }
  }
  // Second pass: unique channel count per hour
  const channelSets = new Map<number, Set<string>>();
  for (const b of buckets) {
    const hour = startOfHour(new Date(b.timestamp)).getTime();
    let set = channelSets.get(hour);
    if (!set) {
      set = new Set();
      channelSets.set(hour, set);
    }
    set.add(b.channel_id);
  }
  const points = Array.from(map.values());
  for (const p of points) {
    p.channels = channelSets.get(p.hour.getTime())?.size ?? 0;
  }
  return points.sort((a, b) => a.hour.getTime() - b.hour.getTime());
}

/** Group usage by channel, return top 6 by request count. */
export function aggregateByChannel(buckets: UsageBucket[]): EntityStat[] {
  const map = new Map<string, EntityStat>();
  for (const b of buckets) {
    const existing = map.get(b.channel_id);
    if (existing) {
      existing.requests += b.request_count;
      existing.tokens += b.input_tokens + b.output_tokens;
      existing.cost += b.estimated_cost;
    } else {
      map.set(b.channel_id, {
        id: b.channel_id,
        name: b.channel_name,
        requests: b.request_count,
        tokens: b.input_tokens + b.output_tokens,
        cost: b.estimated_cost,
      });
    }
  }
  return Array.from(map.values()).sort((a, b) => b.requests - a.requests).slice(0, 6);
}

/** Group usage by model, return top 6 by request count. */
export function aggregateByModel(buckets: UsageBucket[]): EntityStat[] {
  const map = new Map<string, EntityStat>();
  for (const b of buckets) {
    const key = b.model || "unknown";
    const existing = map.get(key);
    if (existing) {
      existing.requests += b.request_count;
      existing.tokens += b.input_tokens + b.output_tokens;
      existing.cost += b.estimated_cost;
    } else {
      map.set(key, {
        id: key,
        name: key,
        requests: b.request_count,
        tokens: b.input_tokens + b.output_tokens,
        cost: b.estimated_cost,
      });
    }
  }
  return Array.from(map.values()).sort((a, b) => b.requests - a.requests).slice(0, 6);
}

/** Build per-channel usage map including a 24-point hourly spark series. */
export function buildUsageByChannelMap(
  buckets: UsageBucket[]
): Map<string, ChannelUsage> {
  const map = new Map<string, ChannelUsage>();
  const sparkMap = new Map<string, number[]>();
  const hours = aggregateByHour(buckets);
  const hourIndex = new Map(hours.map((h, i) => [h.hour.getTime(), i]));
  const totalHours = hours.length;

  for (const b of buckets) {
    const existing = map.get(b.channel_id);
    if (existing) {
      existing.requests += b.request_count;
      existing.tokens += b.input_tokens + b.output_tokens;
      existing.cost += b.estimated_cost;
    } else {
      map.set(b.channel_id, {
        requests: b.request_count,
        tokens: b.input_tokens + b.output_tokens,
        cost: b.estimated_cost,
        spark: [],
      });
    }
    // bucket per hour
    const hk = startOfHour(new Date(b.timestamp)).getTime();
    const idx = hourIndex.get(hk);
    if (idx != null) {
      let arr = sparkMap.get(b.channel_id);
      if (!arr) {
        arr = new Array(totalHours).fill(0);
        sparkMap.set(b.channel_id, arr);
      }
      arr[idx] += b.request_count;
    }
  }

  for (const [id, spark] of sparkMap) {
    const entry = map.get(id);
    if (entry) entry.spark = spark;
  }
  return map;
}

/** 1234 -> "1.2k", 1234567 -> "1.2M", 123 -> "123". */
export function formatNumber(n: number): string {
  const abs = Math.abs(n);
  if (abs < 1000) return Math.round(n).toString();
  if (abs < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** "$1.23", "$1.2k", "$0". */
export function formatCost(n: number): string {
  const abs = Math.abs(n);
  if (abs < 1000) return `$${n.toFixed(2)}`;
  if (abs < 1_000_000) return `$${(n / 1000).toFixed(1)}k`;
  return `$${(n / 1_000_000).toFixed(1)}M`;
}

/** Tokens use k/M notation. */
export function formatTokens(n: number): string {
  return formatNumber(n);
}

/** Color for latency values (returns CSS var or hex). */
export function latencyColor(ms: number): string {
  if (ms < 500) return "var(--color-success)";
  if (ms < 2000) return "var(--color-warning)";
  if (ms < 5000) return "#f97316";
  return "var(--color-danger)";
}

/** Build an SVG polyline path scaled to width x height. */
export function computeSparkPoints(
  values: number[],
  width: number,
  height: number
): string {
  if (values.length === 0) return "";
  if (values.length === 1) {
    const y = height / 2;
    return `M 0 ${y} L ${width} ${y}`;
  }
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const stepX = width / (values.length - 1);
  const points = values.map((v, i) => {
    const x = i * stepX;
    const y = height - ((v - min) / range) * height;
    return `${i === 0 ? "M" : "L"} ${x.toFixed(2)} ${y.toFixed(2)}`;
  });
  return points.join(" ");
}

/** Build a closed SVG area path (for gradient fill under sparkline). */
export function computeSparkArea(
  values: number[],
  width: number,
  height: number
): string {
  const line = computeSparkPoints(values, width, height);
  if (!line) return "";
  return `${line} L ${width} ${height} L 0 ${height} Z`;
}

/** Relative time label, refreshed by caller. */
export function formatRelativeTime(from: Date, now: Date = new Date()): string {
  const diff = Math.max(0, now.getTime() - from.getTime());
  if (diff < 1000) return "just now";
  const sec = Math.floor(diff / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  return `${hr}h ago`;
}

const SPARK_HOUR_MS = 3600_000;

/**
 * Build a 24-point hourly spark series from dispatch logs using a filter.
 * Index 0 = 23 hours ago, index 23 = current hour.
 */
function buildHourlyCountSpark(
  logs: DispatchLog[],
  filter: (l: DispatchLog) => boolean
): number[] {
  if (logs.length === 0) return [];
  const now = Date.now();
  const buckets = new Array(24).fill(0);
  for (const l of logs) {
    if (!filter(l)) continue;
    const diff = now - new Date(l.timestamp).getTime();
    const idx = 23 - Math.floor(diff / SPARK_HOUR_MS);
    if (idx >= 0 && idx < 24) buckets[idx] += 1;
  }
  return buckets;
}

export function buildSuccessSpark(logs: DispatchLog[]): number[] {
  return buildHourlyCountSpark(logs, (l) => l.success);
}

export function buildFailureSpark(logs: DispatchLog[]): number[] {
  return buildHourlyCountSpark(logs, (l) => !l.success);
}

export function buildLatencySpark(logs: DispatchLog[]): number[] {
  if (logs.length === 0) return [];
  const now = Date.now();
  const sums = new Array(24).fill(0);
  const counts = new Array(24).fill(0);
  for (const l of logs) {
    const diff = now - new Date(l.timestamp).getTime();
    const idx = 23 - Math.floor(diff / SPARK_HOUR_MS);
    if (idx >= 0 && idx < 24) {
      sums[idx] += l.latency_ms;
      counts[idx] += 1;
    }
  }
  return sums.map((s, i) => (counts[i] > 0 ? s / counts[i] : 0));
}
