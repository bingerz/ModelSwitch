import React from "react";
import ReactDOM from "react-dom/client";
import "./i18n/config";
import App from "./App";
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
import "./styles/components/misc.css";
import "./styles/ui.css";
import "./styles/login.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
