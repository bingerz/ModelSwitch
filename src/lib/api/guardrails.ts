// Guardrails endpoints, exposed as flat methods `api.guardrailsConfig`
// and `api.updateGuardrails`.

import { request } from "./client";
import type { GuardrailsConfig } from "./types";

export const guardrailsApi = {
  guardrailsConfig: () => request<GuardrailsConfig>("/api/guardrails"),

  updateGuardrails: (config: Partial<GuardrailsConfig>) =>
    request<GuardrailsConfig>("/api/guardrails", {
      method: "PUT",
      body: JSON.stringify(config),
    }),
};
