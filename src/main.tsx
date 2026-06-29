import React from "react";
import ReactDOM from "react-dom/client";
import "./i18n/config";
import App from "./App";

// Apply theme synchronously before React renders to prevent a flash of the
// wrong theme. AppearanceSettings owns interactive updates after mount.
const storedTheme = localStorage.getItem("theme");
const initialTheme =
  storedTheme === "light" || storedTheme === "dark"
    ? storedTheme
    : window.matchMedia("(prefers-color-scheme: light)").matches
      ? "light"
      : "dark";
document.documentElement.setAttribute("data-theme", initialTheme);
import "./styles/global.css";
import "./styles/tokens.css";
import "./styles/components/shared.css";
import "./styles/components/channel.css";
import "./styles/components/logs.css";
import "./styles/components/tiers.css";
import "./styles/components/cost.css";
import "./styles/components/model-selector.css";
import "./styles/components/toast.css";
import "./styles/components/quota.css";
import "./styles/components/usage-chart.css";
import "./styles/components/mcp.css";
import "./styles/components/virtual-keys.css";
import "./styles/components/csv-import-modal.css";
import "./styles/components/misc.css";
import "./styles/ui.css";
import "./styles/login.css";
import "./styles/portal.css";
import "./styles/playground.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
