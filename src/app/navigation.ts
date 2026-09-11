export type TabId =
  | "dashboard"
  | "channels"
  | "virtualKeys"
  | "mcp"
  | "modelRouting"
  | "logs"
  | "cost"
  | "quota"
  | "settings"
  | "audit"
  | "redemption"
  | "metrics"
  | "guardrails"
  | "notifications"
  | "registry"
  | "reports"
  | "playground";

export interface TabConfig {
  id: TabId;
  label: string;
}

export interface TabGroup {
  title: string;
  tabs: TabConfig[];
}

export function getTabGroups(t: (key: string) => string): TabGroup[] {
  return [
    {
      title: t("nav.overview"),
      tabs: [{ id: "dashboard", label: t("nav.dashboard") }],
    },
    {
      title: t("nav.configuration"),
      tabs: [
        { id: "channels", label: t("nav.channels") },
        { id: "virtualKeys", label: t("nav.virtualKeys") },
        { id: "mcp", label: t("nav.mcp") },
        { id: "modelRouting", label: t("nav.modelRouting") },
      ],
    },
    {
      title: t("nav.monitoringPrimary"),
      tabs: [
        { id: "cost", label: t("nav.cost") },
        { id: "quota", label: t("nav.quota") },
        { id: "logs", label: t("nav.logs") },
      ],
    },
    {
      title: t("nav.monitoringAdvanced"),
      tabs: [
        { id: "metrics", label: t("nav.metrics") },
        { id: "audit", label: t("nav.audit") },
        { id: "registry", label: t("nav.registry") },
        { id: "reports", label: t("nav.reports") },
      ],
    },
    {
      title: t("nav.tools"),
      tabs: [{ id: "playground", label: t("nav.playground") }],
    },
    {
      title: t("nav.system"),
      tabs: [
        { id: "settings", label: t("nav.settings") },
        { id: "guardrails", label: t("nav.guardrails") },
        { id: "notifications", label: t("nav.notifications") },
        { id: "redemption", label: t("nav.redemption") },
      ],
    },
  ];
}

// Flat list of all tab IDs for keyboard shortcuts
export const ALL_TAB_IDS: TabId[] = [
  "dashboard",
  "channels",
  "virtualKeys",
  "mcp",
  "modelRouting",
  "logs",
  "cost",
  "quota",
  "settings",
  "audit",
  "redemption",
  "metrics",
  "guardrails",
  "notifications",
  "registry",
  "reports",
  "playground",
];
