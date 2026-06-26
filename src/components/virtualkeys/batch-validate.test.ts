import { describe, it, expect } from "vitest";
import {
  validateBatchCreateInput,
  buildBatchPayload,
  MAX_BATCH_COUNT,
  type BatchCreateSettings,
} from "./batch-validate";

const VALID: BatchCreateSettings = {
  namePrefix: "team-",
  count: "10",
  dailyBudget: "5",
  monthlyBudget: "100",
  rpmLimit: "60",
  tpmLimit: "1000",
  group: "engineering",
  expiresAt: "2024-12-31T23:59",
  allowedModels: "gpt-4,claude-3",
  allowedIps: "10.0.0.1,10.0.0.2",
};

function override<K extends keyof BatchCreateSettings>(
  key: K,
  value: BatchCreateSettings[K],
): BatchCreateSettings {
  return { ...VALID, [key]: value };
}

describe("validateBatchCreateInput", () => {
  it("returns null for fully valid input", () => {
    expect(validateBatchCreateInput(VALID)).toBeNull();
  });

  // ── namePrefix ────────────────────────────────────────────
  it("returns namePrefixRequired for empty prefix", () => {
    expect(validateBatchCreateInput(override("namePrefix", ""))).toBe(
      "namePrefixRequired",
    );
  });

  it("returns namePrefixRequired for whitespace-only prefix", () => {
    expect(validateBatchCreateInput(override("namePrefix", "   "))).toBe(
      "namePrefixRequired",
    );
  });

  // ── count ─────────────────────────────────────────────────
  it("returns countInvalid for zero", () => {
    expect(validateBatchCreateInput(override("count", "0"))).toBe("countInvalid");
  });

  it("returns countInvalid for negative", () => {
    expect(validateBatchCreateInput(override("count", "-1"))).toBe("countInvalid");
  });

  it("returns countInvalid for non-numeric", () => {
    expect(validateBatchCreateInput(override("count", "abc"))).toBe("countInvalid");
  });

  it("returns countInvalid for value above MAX_BATCH_COUNT", () => {
    expect(
      validateBatchCreateInput(override("count", String(MAX_BATCH_COUNT + 1))),
    ).toBe("countInvalid");
  });

  it("returns null when count equals MAX_BATCH_COUNT", () => {
    expect(
      validateBatchCreateInput(override("count", String(MAX_BATCH_COUNT))),
    ).toBeNull();
  });

  it("returns null when count is a decimal within range (floors down)", () => {
    // Math.floor(Number("10.9")) === 10, which is valid.
    expect(validateBatchCreateInput(override("count", "10.9"))).toBeNull();
  });

  // ── dailyBudget ───────────────────────────────────────────
  it("returns dailyBudgetInvalid for malformed value", () => {
    expect(validateBatchCreateInput(override("dailyBudget", "abc"))).toBe(
      "dailyBudgetInvalid",
    );
  });

  it("returns null for empty dailyBudget (optional)", () => {
    expect(validateBatchCreateInput(override("dailyBudget", ""))).toBeNull();
  });

  it("returns dailyBudgetInvalid for negative value", () => {
    expect(validateBatchCreateInput(override("dailyBudget", "-1"))).toBe(
      "dailyBudgetInvalid",
    );
  });

  // ── monthlyBudget ─────────────────────────────────────────
  it("returns monthlyBudgetInvalid for malformed value", () => {
    expect(validateBatchCreateInput(override("monthlyBudget", "xyz"))).toBe(
      "monthlyBudgetInvalid",
    );
  });

  it("returns null for empty monthlyBudget (optional)", () => {
    expect(validateBatchCreateInput(override("monthlyBudget", ""))).toBeNull();
  });

  // ── rpmLimit ──────────────────────────────────────────────
  it("returns rpmLimitInvalid for negative value", () => {
    expect(validateBatchCreateInput(override("rpmLimit", "-10"))).toBe(
      "rpmLimitInvalid",
    );
  });

  it("returns rpmLimitInvalid for non-numeric value", () => {
    expect(validateBatchCreateInput(override("rpmLimit", "abc"))).toBe(
      "rpmLimitInvalid",
    );
  });

  it("returns null for empty rpmLimit (optional)", () => {
    expect(validateBatchCreateInput(override("rpmLimit", ""))).toBeNull();
  });

  it("accepts zero rpmLimit", () => {
    expect(validateBatchCreateInput(override("rpmLimit", "0"))).toBeNull();
  });

  // ── tpmLimit ──────────────────────────────────────────────
  it("returns tpmLimitInvalid for negative value", () => {
    expect(validateBatchCreateInput(override("tpmLimit", "-5"))).toBe(
      "tpmLimitInvalid",
    );
  });

  it("returns tpmLimitInvalid for non-numeric value", () => {
    expect(validateBatchCreateInput(override("tpmLimit", "nan!"))).toBe(
      "tpmLimitInvalid",
    );
  });

  it("returns null for empty tpmLimit (optional)", () => {
    expect(validateBatchCreateInput(override("tpmLimit", ""))).toBeNull();
  });
});

describe("buildBatchPayload", () => {
  it("builds a payload with all fields populated", () => {
    const payload = buildBatchPayload(VALID);
    expect(payload).toEqual({
      count: 10,
      name_prefix: "team-",
      daily_budget_cents: 500,
      monthly_budget_cents: 10000,
      allowed_models: ["gpt-4", "claude-3"],
      allowed_ips: ["10.0.0.1", "10.0.0.2"],
      rpm_limit: 60,
      tpm_limit: 1000,
      expires_at: new Date("2024-12-31T23:59").toISOString(),
      group: "engineering",
    });
  });

  it("parses comma-separated allowed_models and trims whitespace", () => {
    const payload = buildBatchPayload(
      override("allowedModels", " gpt-4 , claude-3 ,gpt-3.5"),
    );
    expect(payload.allowed_models).toEqual([
      "gpt-4",
      "claude-3",
      "gpt-3.5",
    ]);
  });

  it("returns null for empty allowed_models", () => {
    expect(buildBatchPayload(override("allowedModels", "")).allowed_models).toBeNull();
  });

  it("returns null for whitespace-only allowed_models", () => {
    expect(
      buildBatchPayload(override("allowedModels", "   ")).allowed_models,
    ).toBeNull();
  });

  it("parses comma-separated allowed_ips and trims whitespace", () => {
    const payload = buildBatchPayload(
      override("allowedIps", " 10.0.0.1 , 10.0.0.2 "),
    );
    expect(payload.allowed_ips).toEqual(["10.0.0.1", "10.0.0.2"]);
  });

  it("always returns an array for allowed_ips (empty when blank)", () => {
    // Existing contract: allowed_ips is always a string[] (non-nullable).
    const payload = buildBatchPayload(override("allowedIps", ""));
    expect(payload.allowed_ips).toEqual([]);
    expect(payload.allowed_ips).not.toBeNull();
  });

  it("filters out empty entries from comma-separated allowed_ips", () => {
    const payload = buildBatchPayload(override("allowedIps", "10.0.0.1,, ,10.0.0.2"));
    expect(payload.allowed_ips).toEqual(["10.0.0.1", "10.0.0.2"]);
  });

  it("returns null for empty group", () => {
    expect(buildBatchPayload(override("group", "")).group).toBeNull();
  });

  it("returns null for whitespace-only group", () => {
    expect(buildBatchPayload(override("group", "   ")).group).toBeNull();
  });

  it("returns null for empty expiry", () => {
    expect(buildBatchPayload(override("expiresAt", "")).expires_at).toBeNull();
  });

  it("converts expiresAt to ISO string when provided", () => {
    const payload = buildBatchPayload(override("expiresAt", "2025-06-01T12:00"));
    expect(payload.expires_at).toBe(new Date("2025-06-01T12:00").toISOString());
  });

  it("returns null budget cents for empty daily/monthly budgets", () => {
    const payload = buildBatchPayload({
      ...VALID,
      dailyBudget: "",
      monthlyBudget: "",
    });
    expect(payload.daily_budget_cents).toBeNull();
    expect(payload.monthly_budget_cents).toBeNull();
  });

  it("returns null rpm/tpm limits for empty values", () => {
    const payload = buildBatchPayload({
      ...VALID,
      rpmLimit: "",
      tpmLimit: "",
    });
    expect(payload.rpm_limit).toBeNull();
    expect(payload.tpm_limit).toBeNull();
  });
});
