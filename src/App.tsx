import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { type GwStatus, invokeTauri, isTauri } from "./lib/api";
import { ToastProvider, useToast } from "./components/Toast";
import { LoginPage } from "./components/LoginPage";
import { ChannelPanel } from "./components/channel/ChannelPanel";
import { LogViewer } from "./components/LogViewer";
import { StatusBar } from "./components/StatusBar";
import { StatusDashboard } from "./components/dashboard/StatusDashboard";
import { CostDashboard } from "./components/CostDashboard";
import { QuotaPanel } from "./components/quota/QuotaPanel";
import { McpServersPanel } from "./components/mcp/McpServersPanel";
import { VirtualKeysPanel } from "./components/virtualkeys/VirtualKeysPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import ErrorBoundary from "./components/ErrorBoundary";
import { MockBadge } from "./components/MockBadge";
import { QuotaProvider } from "./hooks/useQuota";

type TabId = "dashboard" | "channels" | "virtualKeys" | "mcp" | "logs" | "cost" | "quota" | "settings";
type Theme = "light" | "dark";

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
      ],
    },
    {
      title: t("nav.system"),
      tabs: [{ id: "settings", label: t("nav.settings") }],
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

function AppInner() {
  const { t, i18n } = useTranslation();
  const toast = useToast();
  const [activeTab, setActiveTab] = useState<TabId>("dashboard");
  const [theme, setTheme] = useState<Theme>(getInitialTheme);
  const [lang, setLang] = useState<"en" | "zh">(getInitialLang);
  const [menuOpen, setMenuOpen] = useState(false);
  const [authed, setAuthed] = useState(() => {
    if (isTauri) return true;
    return !!localStorage.getItem("admin_token");
  });

  const TAB_GROUPS = getTabGroups(t);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("theme", theme);
  }, [theme]);

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
      if ((e.ctrlKey || e.metaKey) && e.key >= "1" && e.key <= "8") {
        e.preventDefault();
        const tabs: TabId[] = ["dashboard", "channels", "virtualKeys", "mcp", "logs", "cost", "quota", "settings"];
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
              className="nav-item logout-button"
              onClick={() => {
                if (confirm(t("common.logoutConfirm"))) {
                  localStorage.removeItem("admin_token");
                  window.location.reload();
                }
              }}
            >
              {t("common.logout")}
            </button>
          )}
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
        </nav>
        <main className="main">
          {activeTab === "dashboard" && <StatusDashboard />}
          {activeTab === "channels" && <ChannelPanel />}
          {activeTab === "virtualKeys" && <VirtualKeysPanel />}
          {activeTab === "mcp" && <McpServersPanel />}
          {activeTab === "logs" && <LogViewer />}
          {activeTab === "cost" && <CostDashboard />}
          {activeTab === "quota" && <QuotaPanel />}
          {activeTab === "settings" && <SettingsPanel />}
        </main>
      </div>
      <StatusBar />
      <MockBadge />
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <QuotaProvider>
        <ErrorBoundary>
          <AppInner />
        </ErrorBoundary>
      </QuotaProvider>
    </ToastProvider>
  );
}
