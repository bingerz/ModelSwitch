import { describe, it, expect } from "vitest";
import {
  computeTotalBalance,
  countChannelsWithData,
  countLowBalance,
  countErrors,
} from "./quota-utils";
import type { QuotaInfo } from "../lib/api";

/** Minimal factory for QuotaInfo — fills required fields with sane defaults. */
function makeQuota(overrides: Partial<QuotaInfo> = {}): QuotaInfo {
  return {
    channel_id: "ch-1",
    channel_name: "Test Channel",
    provider: "openai",
    balance: null,
    limit: null,
    usage: null,
    remaining_tokens: null,
    remaining_requests: null,
    plan_status: null,
    expires_at: null,
    items: [],
    groups: [],
    compact_text: null,
    rate_limit_remaining_req: null,
    rate_limit_limit_req: null,
    rate_limit_remaining_tok: null,
    rate_limit_limit_tok: null,
    rate_limit_updated_at: null,
    total_input_tokens: null,
    total_output_tokens: null,
    total_cache_hit_tokens: null,
    total_cache_miss_tokens: null,
    total_requests_counted: null,
    total_estimated_cost: null,
    source: "test",
    updated_at: "2026-01-01T00:00:00Z",
    error: null,
    ...overrides,
  };
}

describe("computeTotalBalance", () => {
  it("returns 0 for empty array", () => {
    expect(computeTotalBalance([])).toBe(0);
  });

  it("sums balances of all non-error channels", () => {
    const quotas = [
      makeQuota({ balance: 100 }),
      makeQuota({ balance: 50.5 }),
      makeQuota({ balance: 200 }),
    ];
    expect(computeTotalBalance(quotas)).toBe(350.5);
  });

  it("excludes error channels from the sum", () => {
    const quotas = [
      makeQuota({ balance: 100 }),
      makeQuota({ balance: 999, error: "API error" }),
      makeQuota({ balance: 50 }),
    ];
    expect(computeTotalBalance(quotas)).toBe(150);
  });

  it("treats null balance as 0", () => {
    const quotas = [
      makeQuota({ balance: null }),
      makeQuota({ balance: 100 }),
    ];
    expect(computeTotalBalance(quotas)).toBe(100);
  });

  it("handles all-null balances", () => {
    const quotas = [
      makeQuota({ balance: null }),
      makeQuota({ balance: null }),
    ];
    expect(computeTotalBalance(quotas)).toBe(0);
  });

  it("handles negative balances", () => {
    const quotas = [
      makeQuota({ balance: -50 }),
      makeQuota({ balance: 100 }),
    ];
    expect(computeTotalBalance(quotas)).toBe(50);
  });
});

describe("countChannelsWithData", () => {
  it("returns 0 for empty array", () => {
    expect(countChannelsWithData([])).toBe(0);
  });

  it("counts channels with balance", () => {
    const quotas = [
      makeQuota({ balance: 100 }),
      makeQuota({ balance: null }),
    ];
    expect(countChannelsWithData(quotas)).toBe(1);
  });

  it("counts channels with rate_limit_remaining_req", () => {
    const quotas = [
      makeQuota({ rate_limit_remaining_req: 500 }),
      makeQuota({ rate_limit_remaining_req: null }),
    ];
    expect(countChannelsWithData(quotas)).toBe(1);
  });

  it("counts channels with total_input_tokens", () => {
    const quotas = [makeQuota({ total_input_tokens: 1000 })];
    expect(countChannelsWithData(quotas)).toBe(1);
  });

  it("counts channels with total_output_tokens", () => {
    const quotas = [makeQuota({ total_output_tokens: 500 })];
    expect(countChannelsWithData(quotas)).toBe(1);
  });

  it("excludes error channels even with data", () => {
    const quotas = [
      makeQuota({ balance: 100, error: "timeout" }),
      makeQuota({ balance: 50 }),
    ];
    expect(countChannelsWithData(quotas)).toBe(1);
  });

  it("excludes channels with all null fields", () => {
    const quotas = [
      makeQuota({ balance: null, rate_limit_remaining_req: null }),
      makeQuota({ balance: 10 }),
    ];
    expect(countChannelsWithData(quotas)).toBe(1);
  });

  it("counts channel with any combination of fields", () => {
    const quotas = [
      makeQuota({ balance: 100 }),
      makeQuota({ rate_limit_remaining_req: 5 }),
      makeQuota({ total_input_tokens: 10 }),
      makeQuota({ total_output_tokens: 20 }),
    ];
    expect(countChannelsWithData(quotas)).toBe(4);
  });
});

describe("countLowBalance", () => {
  it("returns 0 for empty array", () => {
    expect(countLowBalance([])).toBe(0);
  });

  it("counts channels below 20% threshold", () => {
    const quotas = [
      makeQuota({ balance: 15, limit: 100 }), // 15% - low
      makeQuota({ balance: 50, limit: 100 }), // 50% - not low
    ];
    expect(countLowBalance(quotas)).toBe(1);
  });

  it("boundary: exactly 20% is NOT low", () => {
    const quotas = [makeQuota({ balance: 20, limit: 100 })];
    expect(countLowBalance(quotas)).toBe(0);
  });

  it("boundary: 19.99% IS low", () => {
    const quotas = [makeQuota({ balance: 19.99, limit: 100 })];
    expect(countLowBalance(quotas)).toBe(1);
  });

  it("excludes null balance", () => {
    const quotas = [makeQuota({ balance: null, limit: 100 })];
    expect(countLowBalance(quotas)).toBe(0);
  });

  it("excludes null limit", () => {
    const quotas = [makeQuota({ balance: 5, limit: null })];
    expect(countLowBalance(quotas)).toBe(0);
  });

  it("excludes zero limit to avoid division by zero", () => {
    const quotas = [makeQuota({ balance: 0, limit: 0 })];
    expect(countLowBalance(quotas)).toBe(0);
  });

  it("excludes negative limit", () => {
    const quotas = [makeQuota({ balance: -5, limit: -100 })];
    expect(countLowBalance(quotas)).toBe(0);
  });

  it("supports custom threshold", () => {
    const quotas = [
      makeQuota({ balance: 30, limit: 100 }), // 30% - low with threshold 0.5
      makeQuota({ balance: 60, limit: 100 }), // 60% - not low with threshold 0.5
    ];
    expect(countLowBalance(quotas, 0.5)).toBe(1);
  });

  it("counts multiple low-balance channels", () => {
    const quotas = [
      makeQuota({ balance: 5, limit: 100 }),
      makeQuota({ balance: 10, limit: 100 }),
      makeQuota({ balance: 19, limit: 100 }),
      makeQuota({ balance: 50, limit: 100 }), // not low
    ];
    expect(countLowBalance(quotas)).toBe(3);
  });
});

describe("countErrors", () => {
  it("returns 0 for empty array", () => {
    expect(countErrors([])).toBe(0);
  });

  it("returns 0 when all channels are error-free", () => {
    const quotas = [
      makeQuota({ error: null }),
      makeQuota({ error: null }),
    ];
    expect(countErrors(quotas)).toBe(0);
  });

  it("counts all error channels", () => {
    const quotas = [
      makeQuota({ error: "timeout" }),
      makeQuota({ error: null }),
      makeQuota({ error: "auth failed" }),
    ];
    expect(countErrors(quotas)).toBe(2);
  });

  it("counts all-error array", () => {
    const quotas = [
      makeQuota({ error: "err1" }),
      makeQuota({ error: "err2" }),
      makeQuota({ error: "err3" }),
    ];
    expect(countErrors(quotas)).toBe(3);
  });
});
