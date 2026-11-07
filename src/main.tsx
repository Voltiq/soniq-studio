import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "./styles/tokens.css";
import "./styles/app.css";
import App from "./App";
import { CrashGuard, keepInterfaceErrors } from "./components/CrashGuard";
import { applyAppearance } from "./state/appearance";
import { useStore } from "./state/store";

// Apply the saved theme, accent, colours and density before the first paint to avoid a flash.
document.documentElement.dataset.theme = useStore.getState().theme;
applyAppearance(useStore.getState());
keepInterfaceErrors();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <CrashGuard>
      <App />
    </CrashGuard>
  </React.StrictMode>,
);
