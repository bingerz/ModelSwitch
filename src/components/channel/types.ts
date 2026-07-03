import type { PresetCategory } from "../../lib/presets";

// Re-export ApiFormat so sibling modules can import from a single location.
export type { ApiFormat } from "../../lib/presets";

export type ChannelStatus = "healthy" | "circuit_open" | "disabled";

export const STATUS_DOT: Record<ChannelStatus, string> = {
  healthy: "var(--color-success)",
  circuit_open: "var(--color-danger)",
  disabled: "var(--color-text-muted)",
};

export const CATEGORY_ORDER: PresetCategory[] = [
  "official",
  "cn_official",
  "aggregator",
  "cloud_provider",
  "third_party",
];

export const STATUS_LABEL_KEY: Record<ChannelStatus, string> = {
  healthy: "common.statusHealthy",
  circuit_open: "common.statusCircuitOpen",
  disabled: "common.statusDisabled",
};
