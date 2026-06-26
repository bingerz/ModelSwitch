import type { QuotaInfo } from "../lib/api";

/**
 * Sum balances across all channels that have no error.
 * Null balances are treated as 0.
 */
export function computeTotalBalance(quotas: QuotaInfo[]): number {
  return quotas.reduce(
    (sum, q) => sum + (q.error === null ? (q.balance ?? 0) : 0),
    0,
  );
}

/**
 * Count channels that have at least one non-null data field and no error.
 */
export function countChannelsWithData(quotas: QuotaInfo[]): number {
  return quotas.filter(
    (q) =>
      q.error === null &&
      (q.balance != null ||
        q.rate_limit_remaining_req != null ||
        q.total_input_tokens != null ||
        q.total_output_tokens != null),
  ).length;
}

/**
 * Count channels where balance/limit ratio is below the threshold.
 * Channels with null balance/limit or zero limit are excluded.
 */
export function countLowBalance(quotas: QuotaInfo[], threshold = 0.2): number {
  return quotas.filter((q) => {
    if (q.balance == null || q.limit == null || q.limit <= 0) return false;
    return q.balance / q.limit < threshold;
  }).length;
}

/**
 * Count channels that have a non-null error.
 */
export function countErrors(quotas: QuotaInfo[]): number {
  return quotas.filter((q) => q.error !== null).length;
}
