// 墨 · 创作 v3 — 书脊侧栏架构
// Grid layout: [nav-spine] [work-area]
// All navigation consolidated in the left 64px sidebar.

import { useEffect, useState } from "react";
import TitleBar, { type Health } from "./components/TitleBar";
import Navigation from "./components/Navigation";
import { ToastProvider } from "./components/Toast";
import { WorkProvider } from "./components/WorkContext";
import { ping, isDesktop } from "./lib/core";
import LibraryWork from "./screens/LibraryWork";
import PlanningWork from "./screens/PlanningWork";
import DiscussWork from "./screens/DiscussWork";
import SimulateWork from "./screens/SimulateWork";
import StudioWork from "./screens/StudioWork";
import IdeWork from "./screens/IdeWork";
import RevisionWork from "./screens/RevisionWork";
import KnowledgeWork from "./screens/KnowledgeWork";
import SettingsModal from "./components/SettingsModal";
import SessionsDrawer from "./components/SessionsDrawer";
import MemoryDrawer from "./components/MemoryDrawer";
import "./styles/app.css";
import "./styles/work-screens.css";
import "./styles/library-knowledge.css";
import "./styles/ide.css";

type WorkMode =
  | "library"
  | "planning"
  | "discuss"
  | "simulate"
  | "studio"
  | "ide"
  | "revision"
  | "knowledge";

export default function App() {
  const [mode, setMode] = useState<WorkMode>("library");
  const [showSettings, setShowSettings] = useState(false);
  const [showSessions, setShowSessions] = useState(false);
  const [showMemory, setShowMemory] = useState(false);
  const [health, setHealth] = useState<Health>("checking");
  const [resumeSessionId, setResumeSessionId] = useState<string | null>(null);

  const handleResumeSession = (kind: "discuss" | "studio", sessionId: string) => {
    setResumeSessionId(sessionId);
    setMode(kind === "discuss" ? "discuss" : "studio");
  };

  useEffect(() => {
    let alive = true;
    if (!isDesktop()) { setHealth("offline"); return; }
    (async () => {
      try {
        await ping();
        if (alive) setHealth("ok");
      } catch {
        if (alive) setHealth("offline");
      }
    })();
    return () => { alive = false; };
  }, []);

  return (
    <ToastProvider>
      <WorkProvider>
        <div className="app-shell">
          <TitleBar health={health} />

          <Navigation
            active={mode}
            onSelect={setMode}
            onSettings={() => setShowSettings(true)}
            onSessions={() => setShowSessions(true)}
            onMemory={() => setShowMemory(true)}
          />

          <main className="work-area" key={mode}>
            {mode === "library"  && <LibraryWork />}
            {mode === "planning" && <PlanningWork onOpenSettings={() => setShowSettings(true)} />}
            {mode === "discuss"  && (
              <DiscussWork
                onOpenSettings={() => setShowSettings(true)}
                initialSessionId={resumeSessionId ?? undefined}
              />
            )}
            {mode === "simulate" && <SimulateWork onOpenSettings={() => setShowSettings(true)} />}
            {mode === "studio"   && (
              <StudioWork
                onOpenSettings={() => setShowSettings(true)}
                initialSessionId={resumeSessionId ?? undefined}
              />
            )}
            {mode === "ide"      && <IdeWork />}
            {mode === "revision" && <RevisionWork />}
            {mode === "knowledge"&& <KnowledgeWork />}
          </main>
        </div>

        {showSettings && <SettingsModal onClose={() => setShowSettings(false)} />}

        <SessionsDrawer
          open={showSessions}
          onClose={() => setShowSessions(false)}
          onResume={handleResumeSession}
        />

        <MemoryDrawer open={showMemory} onClose={() => setShowMemory(false)} />
      </WorkProvider>
    </ToastProvider>
  );
}
