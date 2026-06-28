import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type GwStatus, invokeTauri, isTauri } from "./lib/api";
import { ToastProvider, useToast } from "./components/Toast";
import { LoginPage } from "./components/LoginPage";
import { Portal } from "./components/Portal";
import { StatusBar } from "./components/StatusBar";
import ErrorBoundary from "./components/ErrorBoundary";
import { MockBadge } from "./components/MockBadge";
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

type TabId = "dashboard" | "channels" | "virtualKeys" | "mcp" | "logs" | "cost" | "quota" | "settings" | "audit" | "redemption" | "metrics" | "guardrails" | "notifications" | "registry" | "reports" | "playground";
type Theme = "light" | "dark";

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

function getTabGroups(t: (key: string) => string): { title: string; tabs: { id: TabId; label: string }[] }[] {
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
      ],
    },
    {
      title: t("nav.monitoring"),
      tabs: [
        { id: "cost", label: t("nav.cost") },
        { id: "quota", label: t("nav.quota") },
        { id: "logs", label: t("nav.logs") },
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

function getInitialTheme(): Theme {
  const stored = localStorage.getItem("theme");
  if (stored === "light" || stored === "dark") return stored;
  return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

function getInitialLang(): "en" | "zh" {
  const stored = localStorage.getItem("lang");
  if (stored === "en" || stored === "zh") return stored;
  return navigator.language.startsWith("zh") ? "zh" : "en";
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
  return (
    <ErrorBoundary fallback={<PanelErrorFallback name={name} />}>
      <Suspense fallback={<div className="panel-loading">Loading…</div>}>
        {children}
      </Suspense>
    </ErrorBoundary>
  );
}

function AppInner() {
  const { t, i18n } = useTranslation();
  const toast = useToast();
  const [activeTab, setActiveTab] = useState<TabId>("dashboard");
  const [theme, setTheme] = useState<Theme>(getInitialTheme);
  const [lang, setLang] = useState<"en" | "zh">(getInitialLang);
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmLogout, setConfirmLogout] = useState(false);
  const logoutTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [authed, setAuthed] = useState(() => {
    if (isTauri) return true;
    return !!localStorage.getItem("admin_token");
  });

  const TAB_GROUPS = getTabGroups(t);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("theme", theme);
  }, [theme]);

  // Cleanup logout confirmation timer on unmount
  useEffect(() => {
    return () => {
      if (logoutTimerRef.current) clearTimeout(logoutTimerRef.current);
    };
  }, []);

  const toggleTheme = () => setTheme((th) => (th === "dark" ? "light" : "dark"));

  const toggleLang = () => {
    const next = lang === "en" ? "zh" : "en";
    setLang(next);
    i18n.changeLanguage(next);
    localStorage.setItem("lang", next);
  };

  // Web mode keyboard shortcuts: Cmd/Ctrl+1..8 to switch tabs
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key >= "1" && e.key <= "9") {
        e.preventDefault();
        const tabs: TabId[] = ["dashboard", "channels", "virtualKeys", "mcp", "logs", "cost", "quota", "settings", "audit", "redemption", "metrics", "guardrails", "notifications", "registry", "reports", "playground"];
        const idx = parseInt(e.key, 10) - 1;
        if (tabs[idx]) {
          setActiveTab(tabs[idx]);
          setMenuOpen(false);
        }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  // On mount: check if gateway is running, auto-start if not
  useEffect(() => {
    if (!isTauri) return; // Web mode: gateway is already running
    let cancelled = false;
    (async () => {
      try {
        const status: GwStatus = await invokeTauri("gateway_status");
        if (status.running) return; // Already running, no toast needed

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
        <div className="sidebar-backdrop" onClick={() => setMenuOpen(false)} />
      )}
      <div className="layout-body">
        <nav className={`sidebar ${menuOpen ? "sidebar-open" : ""}`}>
          <div className="sidebar-brand">ModelSwitch</div>
          <div className="sidebar-nav">
            {TAB_GROUPS.map((group) => (
              <div key={group.title} className="sidebar-group">
                <div className="sidebar-group-title">{group.title}</div>
                {group.tabs.map((tab) => (
                  <button
                    key={tab.id}
                    className={`nav-item ${activeTab === tab.id ? "active" : ""}`}
                    onClick={() => {
                      setActiveTab(tab.id);
                      setMenuOpen(false);
                    }}
                  >
                    {tab.label}
                  </button>
                ))}
              </div>
            ))}
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
          <div className="sidebar-footer">
            <button
              className="theme-toggle"
              onClick={toggleLang}
              title={t("common.switchLang")}
            >
              <span className="theme-toggle-icon">{lang === "en" ? "EN" : "中"}</span>
            </button>
            <button className="theme-toggle" onClick={toggleTheme} title={t("settings.toggleTheme", { mode: theme === "dark" ? t("settings.light") : t("settings.dark") })}>
              <span className="theme-toggle-icon">{theme === "dark" ? "☀" : "☾"}</span>
            </button>
          </div>
        </nav>
        <main className="main">
          {activeTab === "dashboard" && (
            <PanelBoundary name="Dashboard">
              <StatusDashboard />
            </PanelBoundary>
          )}
          {activeTab === "channels" && (
            <PanelBoundary name="Channels">
              <ChannelPanel />
            </PanelBoundary>
          )}
          {activeTab === "virtualKeys" && (
            <PanelBoundary name="Virtual Keys">
              <VirtualKeysPanel />
            </PanelBoundary>
          )}
          {activeTab === "mcp" && (
            <PanelBoundary name="MCP Servers">
              <McpServersPanel />
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
              <Playground />
            </PanelBoundary>
          )}
        </main>
      </div>
      <StatusBar />
      <MockBadge />
    </div>
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
