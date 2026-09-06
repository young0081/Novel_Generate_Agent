import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/theme.css";
import "./styles/seal.css";
import "./styles/app.css";
import "./styles/work-screens.css";
import "./styles/library-knowledge.css";
import "./styles/ide.css";
import "./styles/ai-motion.css";
import "./styles/redesign.css";
import App from "./App";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

async function renderApplication() {
  const fixtureRequested =
    import.meta.env.DEV &&
    new URLSearchParams(window.location.search).get("ui-fixture") === "agent";

  if (fixtureRequested) {
    const { default: AgentStateFixture } = await import("./dev/AgentStateFixture");
    root.render(
      <React.StrictMode>
        <AgentStateFixture />
      </React.StrictMode>,
    );
    return;
  }

  root.render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void renderApplication();
