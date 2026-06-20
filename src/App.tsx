import { useEffect, useState } from "react";
import { type GwStatus, invokeTauri } from "./lib/api";
import { ToastProvider, useToast } from "./components/Toast";
import { ChannelPanel } from "./components/ChannelPanel";
import { LogViewer } from "./components/LogViewer";
import { StatusBar } from "./components/StatusBar";
import { StatusDashboard } from "./components/StatusDashboard";
import { CostDashboard } from "./components/CostDashboard";
import { QuotaPanel } from "./components/QuotaPanel";
import { McpServersPanel } from "./components/McpServersPanel";
import { VirtualKeysPanel } from "./components/VirtualKeysPanel";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { QuotaProvider } from "./hooks/useQuota";

type TabId = "dashboard" | "channels" | "virtualKeys" | "mcp" | "logs" | "cost" | "quota" | "settings";
type Theme = "light" | "dark";

const TAB_GROUPS: { title: string; tabs: { id: TabId; label: string }[] }[] = [
  {
    title: "Overview",
    tabs: [{ id: "dashboard", label: "Dashboard" }],
  },
  {
    title: "Configuration",
    tabs: [
      { id: "channels", label: "Channels" },
      { id: "virtualKeys", label: "Virtual Keys" },
      { id: "mcp", label: "MCP" },
    ],
  },
  {
    title: "Monitoring",
    tabs: [
      { id: "logs", label: "Logs" },
      { id: "cost", label: "Cost Analytics" },
      { id: "quota", label: "Provider Quota" },
    ],
  },
  {
    title: "System",
    tabs: [{ id: "settings", label: "Settings" }],
  },
];

function getInitialTheme(): Theme {
  const stored = localStorage.getItem("theme");
  if (stored === "light" || stored === "dark") return stored;
  return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

function AppInner() {
  const toast = useToast();
  const [activeTab, setActiveTab] = useState<TabId>("dashboard");
  const [theme, setTheme] = useState<Theme>(getInitialTheme);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("theme", theme);
  }, [theme]);

  const toggleTheme = () => setTheme((t) => (t === "dark" ? "light" : "dark"));

  // On mount: check if gateway is running, auto-start if not
  useEffect(() => {
    (async () => {
      try {
        const status: GwStatus = await invokeTauri("gateway_status");
        if (!status.running) {
          await invokeTauri("gateway_start");
          toast.success("Gateway started");
        }
      } catch (e) {
        // Tauri API not available (browser dev mode) or gateway bind failed
        if (typeof e === "string" && e.includes("bind")) {
          toast.error(`Gateway failed: ${e}`);
        }
      }
    })();
  }, [toast]);

  // Listen for window close-requested event from Rust
  // Gateway lifecycle is tied to the app — closing the app always stops the gateway.
  useEffect(() => {
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
          <button className="theme-toggle" onClick={toggleTheme} title={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}>
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
          {activeTab === "settings" && <div className="panel-loading">Settings page loading...</div>}
        </main>
      </div>
      <StatusBar />
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
