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

  // On mount: check if gateway is running, auto-start if not
  useEffect(() => {
    if (!isTauri) return; // Web mode: gateway is already running
    (async () => {
      try {
        const status: GwStatus = await invokeTauri("gateway_status");
        if (!status.running) {
          await invokeTauri("gateway_start");
          toast.success(t("toast.gatewayStarted"));
        }
      } catch (e) {
        // Tauri API not available (browser dev mode) or gateway bind failed
        if (typeof e === "string" && e.includes("bind")) {
          toast.error(t("toast.gatewayFailed", { error: e }));
        }
      }
    })();
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
      <div className="layout-body">
        <nav className="sidebar">
          <div className="sidebar-brand">ModelSwitch</div>
          <div className="sidebar-nav">
            {TAB_GROUPS.map((group) => (
              <div key={group.title} className="sidebar-group">
                <div className="sidebar-group-title">{group.title}</div>
                {group.tabs.map((tab) => (
                  <button
                    key={tab.id}
                    className={`nav-item ${activeTab === tab.id ? "active" : ""}`}
                    onClick={() => setActiveTab(tab.id)}
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
                localStorage.removeItem("admin_token");
                window.location.reload();
              }}
            >
              {t("common.logout")}
            </button>
          )}
          <button
            className="theme-toggle"
            onClick={toggleLang}
            title={lang === "en" ? "Switch to Chinese" : "Switch to English"}
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
