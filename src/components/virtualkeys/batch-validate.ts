import { dollarsToCents } from "./types";
import type { BatchCreateVirtualKeyData } from "../../lib/api";

export const MAX_BATCH_COUNT = 500;

export interface BatchCreateSettings {
  namePrefix: string;
  count: string;
  dailyBudget: string;
  monthlyBudget: string;
  rpmLimit: string;
  tpmLimit: string;
  group: string;
  expiresAt: string;
  allowedModels: string;
  allowedIps: string;
}

export type BatchValidationError =
  | "namePrefixRequired"
  | "countInvalid"
  | "dailyBudgetInvalid"
  | "monthlyBudgetInvalid"
  | "rpmLimitInvalid"
  | "tpmLimitInvalid";

/**
 * Validate batch-create settings. Returns a machine-readable error code if
 * validation fails, or null when the settings are valid. Does NOT perform i18n
 * — the caller maps the returned code to a translated string.
 */
export function validateBatchCreateInput(
  settings: BatchCreateSettings,
): BatchValidationError | null {
  const trimmedPrefix = settings.namePrefix.trim();
  if (!trimmedPrefix) return "namePrefixRequired";

  const count = Math.floor(Number(settings.count));
  if (!Number.isFinite(count) || count < 1 || count > MAX_BATCH_COUNT) {
    return "countInvalid";
  }

  const dailyCents = dollarsToCents(settings.dailyBudget);
  if (settings.dailyBudget.trim() !== "" && dailyCents === null) {
    return "dailyBudgetInvalid";
  }

  const monthlyCents = dollarsToCents(settings.monthlyBudget);
  if (settings.monthlyBudget.trim() !== "" && monthlyCents === null) {
    return "monthlyBudgetInvalid";
  }

  const parsedRpm =
    settings.rpmLimit.trim() === "" ? null : Number(settings.rpmLimit.trim());
  if (parsedRpm !== null && (!Number.isFinite(parsedRpm) || parsedRpm < 0)) {
    return "rpmLimitInvalid";
  }

  const parsedTpm =
    settings.tpmLimit.trim() === "" ? null : Number(settings.tpmLimit.trim());
  if (parsedTpm !== null && (!Number.isFinite(parsedTpm) || parsedTpm < 0)) {
    return "tpmLimitInvalid";
  }

  return null;
}

/**
 * Build the API payload from validated settings. Assumes the caller has
 * invoked `validateBatchCreateInput` first; behaviour for invalid input is
 * best-effort (counts/budgets coerce as they do in the validator).
 */
export function buildBatchPayload(
  settings: BatchCreateSettings,
): BatchCreateVirtualKeyData {
  return {
    count: Math.floor(Number(settings.count)),
    name_prefix: settings.namePrefix.trim(),
    daily_budget_cents: dollarsToCents(settings.dailyBudget),
    monthly_budget_cents: dollarsToCents(settings.monthlyBudget),
    allowed_models: settings.allowedModels.trim()
      ? settings.allowedModels
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean)
      : null,
    // allowed_ips always emits an array (empty when blank) to match the
    // existing API contract (`allowed_ips?: string[]`, non-nullable).
    allowed_ips: settings.allowedIps
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
    rpm_limit:
      settings.rpmLimit.trim() === "" ? null : Number(settings.rpmLimit.trim()),
    tpm_limit:
      settings.tpmLimit.trim() === "" ? null : Number(settings.tpmLimit.trim()),
    expires_at: settings.expiresAt.trim()
      ? new Date(settings.expiresAt.trim()).toISOString()
      : null,
    group: settings.group.trim() || null,
  };
}
