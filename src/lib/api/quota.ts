// Quota + redemption code endpoints.
// `quota()` is a flat method on `api`; redemption codes are nested at
// `api.redemptionCodes`.

import { request } from "./client";
import type { QuotaInfo, RedemptionCode } from "./types";

export const quota = () => request<QuotaInfo[]>("/api/quota");

export const redemptionCodesApi = {
  list: () => request<RedemptionCode[]>("/api/redemption-codes"),

  create: (data: { credits_cents: number; expires_at?: string | null }) =>
    request<RedemptionCode>("/api/redemption-codes", {
      method: "POST",
      body: JSON.stringify(data),
    }),

  redeem: (code: string, userId?: string) =>
    request<{ credits_cents: number }>("/api/redemption-codes/redeem", {
      method: "POST",
      body: JSON.stringify({ code, user_id: userId }),
    }),

  delete: (code: string) =>
    request<void>(`/api/redemption-codes/${code}`, { method: "DELETE" }),
};
