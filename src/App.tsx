import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type GwStatus, invokeTauri, isTauri } from "./lib/api";
import { probeGatewayHealth } from "./lib/gateway-probe";
import { ToastProvider, useToast } from "./components/Toast";
import { LoginPage } from "./components/LoginPage";
import { Portal } from "./components/Portal";
import { StatusBar } from "./components/StatusBar";
import ErrorBoundary from "./components/ErrorBoundary";
import { MockBadge } from "./components/MockBadge";
import { OnboardingBanner } from "./components/OnboardingBanner";
import { QuotaProvider } from "./hooks/useQuota";

// F5 — Lazy-loadable panel chunks.
// Each major panel is split into its own bundle so the initial download only
// contains the shell (sidebar / header / status bar). The shared Suspense
// fallback inside <PanelBoundary /> renders while a chunk is fetched.
const StatusDashboard = lazy(() =>
  import("./components/dashboard/StatusDashboard").then((m) => ({ default: m.StatusDashboard }))
);
const ChannelPanel = lazy(() =>
  import("./components/channel/ChannelPanel").then((m) => ({ default: m.ChannelPanel }))
);
const VirtualKeysPanel = lazy(() =>
  import("./components/virtualkeys/VirtualKeysPanel").then((m) => ({ default: m.VirtualKeysPanel }))
);
const McpServersPanel = lazy(() =>
  import("./components/mcp/McpServersPanel").then((m) => ({ default: m.McpServersPanel }))
);
const LogViewer = lazy(() =>
  import("./components/LogViewer").then((m) => ({ default: m.LogViewer }))
);
const CostDashboard = lazy(() =>
  import("./components/CostDashboard").then((m) => ({ default: m.CostDashboard }))
);
const QuotaPanel = lazy(() =>
  import("./components/quota/QuotaPanel").then((m) => ({ default: m.QuotaPanel }))
);
const SettingsPanel = lazy(() =>
  import("./components/SettingsPanel").then((m) => ({ default: m.SettingsPanel }))
);
const AuditLogPanel = lazy(() =>
  import("./components/AuditLogPanel").then((m) => ({ default: m.AuditLogPanel }))
);
const GuardrailsPanel = lazy(() =>
  import("./components/GuardrailsPanel").then((m) => ({ default: m.GuardrailsPanel }))
);
const RedemptionCodesPanel = lazy(() =>
  import("./components/RedemptionCodesPanel").then((m) => ({ default: m.RedemptionCodesPanel }))
);
const MetricsPanel = lazy(() =>
  import("./components/MetricsPanel").then((m) => ({ default: m.MetricsPanel }))
);
const ModelRegistryPanel = lazy(() =>
  import("./components/ModelRegistryPanel").then((m) => ({ default: m.ModelRegistryPanel }))
);
const NotificationPanel = lazy(() =>
  import("./components/NotificationPanel").then((m) => ({ default: m.NotificationPanel }))
);
const ReportsPanel = lazy(() =>
  import("./components/ReportsPanel").then((m) => ({ default: m.ReportsPanel }))
);
const Playground = lazy(() =>
  import("./components/playground/Playground").then((m) => ({ default: m.Playground }))
);
const ModelRoutingPanel = lazy(() =>
  import("./components/ModelRoutingPanel").then((m) => ({ default: m.ModelRoutingPanel }))
);

import { type TabId } from "./app/navigation";

const PRIMARY_TABS: TabId[] = [
  "dashboard",
  "channels",
  "virtualKeys",
  "playground",
  "logs",
  "settings",
];

const ADVANCED_TABS: TabId[] = [
  "mcp",
  "modelRouting",
  "cost",
  "quota",
  "metrics",
  "reports",
  "registry",
  "audit",
  "guardrails",
  "notifications",
  "redemption",
];

const TAB_LABEL_KEYS: Record<TabId, string> = {
  dashboard: "nav.dashboard",
  channels: "nav.channels",
  virtualKeys: "nav.virtualKeys",
  playground: "nav.playground",
  logs: "nav.logs",
  settings: "nav.settings",
  mcp: "nav.mcp",
  modelRouting: "nav.modelRouting",
  cost: "nav.cost",
  quota: "nav.quota",
  metrics: "nav.metrics",
  reports: "nav.reports",
  registry: "nav.registry",
  audit: "nav.audit",
  guardrails: "nav.guardrails",
  notifications: "nav.notifications",
  redemption: "nav.redemption",
};

// React Query client — sensible defaults for a management console.
// Created at module scope so it is stable across renders, regardless of mock mode.
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: true,
      staleTime: 5_000, // 5s before data is considered stale
    },
  },
});

// Route check: if the URL path starts with /portal, render the employee
// self-service portal instead of the admin console.
function isPortalRoute(): boolean {
  return window.location.pathname.startsWith("/portal");
}

/**
 * F6 — Per-panel error fallback. Rendered by the inner <ErrorBoundary> when a
 * single panel throws during render, so a crash in one panel does not take
 * down the whole admin console.
 */
function PanelErrorFallback({ name }: { name: string }) {
  const { t } = useTranslation();
  return (
    <div className="panel-error" role="alert">
      <h3 className="panel-error-title">{t("error.panelLoadTitle", { name })}</h3>
      <p className="panel-error-message">{t("error.panelLoadMessage")}</p>
      <div className="panel-error-actions">
        <button
          className="btn btn-primary"
          onClick={() => window.location.reload()}
        >
          {t("error.retry")}
        </button>
      </div>
    </div>
  );
}

/**
 * Wraps a panel in its own Suspense + ErrorBoundary. The ErrorBoundary sits
 * OUTSIDE the Suspense boundary so it can catch errors from lazily-loaded
 * components. Each panel is isolated — a failure or pending chunk only
 * affects this panel, not the sidebar or status bar.
 */
function PanelBoundary({ name, children }: { name: string; children: ReactNode }) {
  const { t } = useTranslation();
  return (
    <ErrorBoundary fallback={<PanelErrorFallback name={name} />}>
      <Suspense
        fallback={
          <div className="panel-loading" role="status">
            {t("common.loading")}
          </div>
        }
      >
        {children}
      </Suspense>
    </ErrorBoundary>
  );
}

function AppInner() {
  const { t } = useTranslation();
  const toast = useToast();
  const [activeTab, setActiveTab] = useState<TabId>("dashboard");
  const [menuOpen, setMenuOpen] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [confirmLogout, setConfirmLogout] = useState(false);
  const logoutTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [authed, setAuthed] = useState(() => {
    if (isTauri) return true;
    return !!localStorage.getItem("admin_token");
  });

  // Cleanup logout confirmation timer on unmount
  useEffect(() => {
    return () => {
      if (logoutTimerRef.current) clearTimeout(logoutTimerRef.current);
    };
  }, []);

  useEffect(() => {
    if (ADVANCED_TABS.includes(activeTab)) {
      setAdvancedOpen(true);
    }
  }, [activeTab]);

  useEffect(() => {
    document.title = `${t(TAB_LABEL_KEYS[activeTab])} · ModelSwitch`;
  }, [activeTab, t]);

  // Keyboard shortcuts: Cmd/Ctrl+1..6 for primary tabs
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key >= "1" && e.key <= "6") {
        e.preventDefault();
        const idx = parseInt(e.key, 10) - 1;
        if (PRIMARY_TABS[idx]) {
          setActiveTab(PRIMARY_TABS[idx]);
          setMenuOpen(false);
        }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  // Listen for custom navigation events from empty states / CTAs
  useEffect(() => {
    const handler = (e: Event) => {
      const customEvent = e as CustomEvent<TabId>;
      if (customEvent.detail) {
        setActiveTab(customEvent.detail);
        setMenuOpen(false);
      }
    };
    window.addEventListener("navigate-to-tab", handler);
    return () => window.removeEventListener("navigate-to-tab", handler);
  }, []);

  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenuOpen(false);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [menuOpen]);

  // On mount: check if gateway is running, auto-start if not
  useEffect(() => {
    if (!isTauri) return; // Web mode: gateway is already running
    let cancelled = false;
    (async () => {
      try {
        const status: GwStatus = await invokeTauri("gateway_status");
        if (status.running) return; // Already running, no toast needed

        // Another process may already own the port (zombie gateway from a prior run).
        if (await probeGatewayHealth()) {
          if (!cancelled) {
            toast.error(t("toast.gatewayPortInUse"));
          }
          return;
        }

        // Attempt to start, with one retry after 2s if bind fails
        try {
          await invokeTauri("gateway_start");
        } catch (e) {
          if (cancelled) return;
          if (typeof e === "string" && e.includes("bind")) {
            // Port still in use — wait and retry once
            await new Promise((r) => setTimeout(r, 2000));
            if (cancelled) return;
            await invokeTauri("gateway_start"); // Retry
          } else {
            throw e; // Re-throw non-bind errors
          }
        }

        if (!cancelled) {
          toast.success(t("toast.gatewayStarted"));
        }
      } catch (e) {
        if (cancelled) return;
        // Don't show error for "already running" — that's a normal race
        if (typeof e === "string" && e.includes("already running")) return;
        toast.error(
          typeof e === "string" ? e : t("toast.gatewayFailed", { error: String(e) })
        );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [toast, t]);

  // Listen for window close-requested event from Rust
  // Gateway lifecycle is tied to the app — closing the app always stops the gateway.
  useEffect(() => {
    if (!isTauri) return; // Web mode: no Tauri window events
    let unlisten: (() => void) | null = null;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlisten = await listen("close-requested", async () => {
          try {
            // Always stop gateway and quit — their lifecycles are tied
            await invokeTauri("gateway_stop");
            await invokeTauri("app_quit");
          } catch {
            // Fallback: just quit, the Drop guard will clean up
            await invokeTauri("app_quit");
          }
        });
      } catch {
        // Tauri API not available
      }
    })();

    return () => {
      unlisten?.();
    };
  }, []);

  if (!authed) {
    return <LoginPage onSuccess={() => setAuthed(true)} />;
  }

  return (
    <>
      <a href="#main-content" className="skip-link">{t("common.skipToContent")}</a>
      <div className="layout">
      <button
        className="hamburger-toggle"
        onClick={() => setMenuOpen((o) => !o)}
        aria-label={t("common.toggleMenu")}
        aria-expanded={menuOpen}
      >
        <span className={`hamburger-icon ${menuOpen ? "hamburger-icon-open" : ""}`} />
      </button>
      {menuOpen && (
        <div
          className="sidebar-backdrop"
          onClick={() => setMenuOpen(false)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") setMenuOpen(false);
          }}
          role="button"
          tabIndex={0}
          aria-label={t("common.toggleMenu")}
        />
      )}
      <div className="layout-body">
        <nav className={`sidebar ${menuOpen ? "sidebar-open" : ""}`} aria-label="Main">
          <div className="sidebar-brand">
            <div className="sidebar-brand-name">ModelSwitch</div>
            <div className="sidebar-brand-tagline">{t("nav.tagline")}</div>
          </div>
          <div className="sidebar-nav">
            <div className="sidebar-group">
              {PRIMARY_TABS.map((tab) => (
                <button
                  key={tab}
                  className={`nav-item ${activeTab === tab ? "active" : ""}`}
                  onClick={() => {
                    setActiveTab(tab);
                    setMenuOpen(false);
                  }}
                  aria-current={activeTab === tab ? "page" : undefined}
                >
                  {t(TAB_LABEL_KEYS[tab])}
                </button>
              ))}
            </div>
            <div className="sidebar-group sidebar-group-advanced">
              <button
                type="button"
                className="sidebar-advanced-toggle"
                aria-expanded={advancedOpen}
                onClick={() => setAdvancedOpen((o) => !o)}
              >
                <span>{t("nav.advanced")}</span>
                <span className="sidebar-advanced-chevron" aria-hidden="true">
                  {advancedOpen ? "▾" : "▸"}
                </span>
              </button>
              {advancedOpen &&
                ADVANCED_TABS.map((tab) => (
                  <button
                    key={tab}
                    className={`nav-item ${activeTab === tab ? "active" : ""}`}
                    onClick={() => {
                      setActiveTab(tab);
                      setMenuOpen(false);
                    }}
                    aria-current={activeTab === tab ? "page" : undefined}
                  >
                    {t(TAB_LABEL_KEYS[tab])}
                  </button>
                ))}
            </div>
          </div>
          {!isTauri && (
            <button
              className={`nav-item logout-button ${confirmLogout ? "logout-button-confirm" : ""}`}
              onClick={() => {
                if (!confirmLogout) {
                  setConfirmLogout(true);
                  if (logoutTimerRef.current) clearTimeout(logoutTimerRef.current);
                  logoutTimerRef.current = setTimeout(() => setConfirmLogout(false), 3000);
                  return;
                }
                if (logoutTimerRef.current) {
                  clearTimeout(logoutTimerRef.current);
                  logoutTimerRef.current = null;
                }
                setConfirmLogout(false);
                localStorage.removeItem("admin_token");
                window.location.reload();
              }}
            >
              {confirmLogout ? t("common.confirmQuestion") : t("common.logout")}
            </button>
          )}
        </nav>
        <main className="main" id="main-content" tabIndex={-1}>
          {activeTab === "dashboard" && (
            <OnboardingBanner onNavigate={(tab) => setActiveTab(tab as TabId)} />
          )}
          {activeTab === "dashboard" && (
            <PanelBoundary name="Dashboard">
              <StatusDashboard onNavigate={(tab) => setActiveTab(tab as TabId)} />
            </PanelBoundary>
          )}
          {activeTab === "channels" && (
            <PanelBoundary name="Channels">
              <ChannelPanel />
            </PanelBoundary>
          )}
          {activeTab === "virtualKeys" && (
            <PanelBoundary name="Virtual Keys">
              <VirtualKeysPanel onNavigate={(tab) => setActiveTab(tab as TabId)} />
            </PanelBoundary>
          )}
          {activeTab === "mcp" && (
            <PanelBoundary name="MCP Servers">
              <McpServersPanel />
            </PanelBoundary>
          )}
          {activeTab === "modelRouting" && (
            <PanelBoundary name="Model Routing">
              <ModelRoutingPanel />
            </PanelBoundary>
          )}
          {activeTab === "logs" && (
            <PanelBoundary name="Logs">
              <LogViewer />
            </PanelBoundary>
          )}
          {activeTab === "cost" && (
            <PanelBoundary name="Cost">
              <CostDashboard />
            </PanelBoundary>
          )}
          {activeTab === "quota" && (
            <PanelBoundary name="Quota">
              <QuotaPanel />
            </PanelBoundary>
          )}
          {activeTab === "settings" && (
            <PanelBoundary name="Settings">
              <SettingsPanel />
            </PanelBoundary>
          )}
          {activeTab === "audit" && (
            <PanelBoundary name="Audit Log">
              <AuditLogPanel />
            </PanelBoundary>
          )}
          {activeTab === "guardrails" && (
            <PanelBoundary name="Guardrails">
              <GuardrailsPanel />
            </PanelBoundary>
          )}
          {activeTab === "redemption" && (
            <PanelBoundary name="Redemption Codes">
              <RedemptionCodesPanel />
            </PanelBoundary>
          )}
          {activeTab === "metrics" && (
            <PanelBoundary name="Metrics">
              <MetricsPanel />
            </PanelBoundary>
          )}
          {activeTab === "notifications" && (
            <PanelBoundary name="Notifications">
              <NotificationPanel />
            </PanelBoundary>
          )}
          {activeTab === "registry" && (
            <PanelBoundary name="Model Registry">
              <ModelRegistryPanel />
            </PanelBoundary>
          )}
          {activeTab === "reports" && (
            <PanelBoundary name="Reports">
              <ReportsPanel />
            </PanelBoundary>
          )}
          {activeTab === "playground" && (
            <PanelBoundary name="Playground">
              <Playground onNavigate={(tab) => setActiveTab(tab as TabId)} />
            </PanelBoundary>
          )}
        </main>
      </div>
      <StatusBar />
      <MockBadge />
    </div>
    </>
  );
}

export default function App() {
  // Employee self-service portal — rendered when URL path starts with /portal.
  // The portal authenticates via the employee's virtual key, not admin token.
  if (isPortalRoute()) {
    return <Portal />;
  }

  return (
    <ErrorBoundary>
      <QueryClientProvider client={queryClient}>
        <ToastProvider>
          <QuotaProvider>
            <AppInner />
          </QuotaProvider>
        </ToastProvider>
      </QueryClientProvider>
    </ErrorBoundary>
  );
}
