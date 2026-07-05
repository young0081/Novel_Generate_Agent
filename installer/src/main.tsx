import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import UninstallApp from "./Uninstall";
import { isUninstallMode } from "./lib/installer";
import "./App.css";

function Root() {
  const [mode, setMode] = useState<"loading" | "install" | "uninstall">("loading");

  useEffect(() => {
    isUninstallMode().then((isUninstall) => {
      setMode(isUninstall ? "uninstall" : "install");
      if (isUninstall) {
        document.title = "墨·创作 — 卸载";
      }
    });
  }, []);

  if (mode === "loading") {
    return <div style={{ background: "#efe8da", width: "100vw", height: "100vh" }} />;
  }

  return mode === "uninstall" ? <UninstallApp /> : <App />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
