// 墨 · 创作 v3 — 书脊侧栏架构
// Grid layout: [nav-spine] [work-area]
// All navigation consolidated in the left 64px sidebar.

import { lazy, Suspense, useEffect, useRef, useState } from "react";
import TitleBar, { type Health } from "./components/TitleBar";
import Navigation from "./components/Navigation";
import { ToastProvider } from "./components/Toast";
import { WorkProvider } from "./components/WorkContext";
import { ping, isDesktop } from "./lib/core";
import {
  acquireEditorWriteBarrier,
  acquireWorkspaceAction,
  cancelPendingEditorRuns,
  flushPendingEditors,
} from "./lib/editorPersistence";
import AiActivity from "./components/agent/AiActivity";
import type { SessionResumeMode } from "./lib/sessionResume";

type WorkMode =
  | "library"
  | "planning"
  | "harness"
  | "discuss"
  | "simulate"
  | "studio"
  | "ide"
  | "revision"
  | "knowledge"
  | "style"
  | "collab"
  | "checkpoints";

const WORK_MODE_LABEL: Record<WorkMode, string> = {
  library: "书库",
  planning: "策划",
  harness: "Harness",
  discuss: "探讨",
  simulate: "推演",
  studio: "创作",
  ide: "编辑",
  revision: "修订",
  knowledge: "知识库",
  style: "文风",
  collab: "协作",
  checkpoints: "快照",
};

const LibraryWork = lazy(() => import("./screens/LibraryWork"));
const PlanningWork = lazy(() => import("./screens/PlanningWork"));
const HarnessWork = lazy(() => import("./screens/HarnessWork"));
const DiscussWork = lazy(() => import("./screens/DiscussWork"));
const SimulateWork = lazy(() => import("./screens/SimulateWork"));
const StudioWork = lazy(() => import("./screens/StudioWork"));
const IdeWork = lazy(() => import("./screens/IdeWork"));
const RevisionWork = lazy(() => import("./screens/RevisionWork"));
const KnowledgeWork = lazy(() => import("./screens/KnowledgeWork"));
const StyleScreen = lazy(() => import("./screens/StyleScreen"));
const CollabScreen = lazy(() => import("./screens/CollabScreen"));
const CheckpointsScreen = lazy(() => import("./screens/CheckpointsScreen"));
const SettingsModal = lazy(() => import("./components/SettingsModal"));
const SessionsDrawer = lazy(() => import("./components/SessionsDrawer"));
const MemoryDrawer = lazy(() => import("./components/MemoryDrawer"));

interface ResumeTarget {
  kind: SessionResumeMode;
  sessionId: string;
  request: number;
}

export default function App() {
  const [mode, setMode] = useState<WorkMode>("library");
  const [showSettings, setShowSettings] = useState(false);
  const [showSessions, setShowSessions] = useState(false);
  const [showMemory, setShowMemory] = useState(false);
  const [health, setHealth] = useState<Health>("checking");
  const [resumeTarget, setResumeTarget] = useState<ResumeTarget | null>(null);
  const [pendingMode, setPendingMode] = useState<WorkMode | null>(null);
  const resumeSeq = useRef(0);
  const navigationSeq = useRef(0);

  const handleResumeSession = (kind: SessionResumeMode, sessionId: string) => {
    const request = ++navigationSeq.current;
    setPendingMode(kind);
    cancelPendingEditorRuns();
    const releaseEditorWriteBarrier = acquireEditorWriteBarrier();
    void (async () => {
      const releaseWorkspaceAction = await acquireWorkspaceAction();
      if (request !== navigationSeq.current) {
        releaseWorkspaceAction();
        releaseEditorWriteBarrier();
        return;
      }
      const flushed = await flushPendingEditors();
      if (request !== navigationSeq.current) {
        releaseWorkspaceAction();
        releaseEditorWriteBarrier();
        return;
      }
      if (!flushed) {
        releaseWorkspaceAction();
        releaseEditorWriteBarrier();
        window.alert("当前编辑文件保存失败，无法切换页面。请检查错误后重试。");
        if (request === navigationSeq.current) setPendingMode(null);
        return;
      }
      resumeSeq.current += 1;
      setResumeTarget({ kind, sessionId, request: resumeSeq.current });
      setMode(kind);
      window.requestAnimationFrame(() => {
        if (request === navigationSeq.current) setPendingMode(null);
      });
      window.setTimeout(releaseWorkspaceAction, 0);
      window.setTimeout(releaseEditorWriteBarrier, 0);
    })();
  };

  const handleSelectMode = (next: WorkMode) => {
    if (next === mode || next === pendingMode) return;
    const request = ++navigationSeq.current;
    setPendingMode(next);
    cancelPendingEditorRuns();
    const releaseEditorWriteBarrier = acquireEditorWriteBarrier();
    void (async () => {
      const releaseWorkspaceAction = await acquireWorkspaceAction();
      if (request !== navigationSeq.current) {
        releaseWorkspaceAction();
        releaseEditorWriteBarrier();
        return;
      }
      const flushed = await flushPendingEditors();
      if (request !== navigationSeq.current) {
        releaseWorkspaceAction();
        releaseEditorWriteBarrier();
        return;
      }
      if (!flushed) {
        releaseWorkspaceAction();
        releaseEditorWriteBarrier();
        window.alert("当前编辑文件保存失败，无法切换页面。请检查错误后重试。");
        if (request === navigationSeq.current) setPendingMode(null);
        return;
      }
      setResumeTarget(null);
      setMode(next);
      window.requestAnimationFrame(() => {
        if (request === navigationSeq.current) setPendingMode(null);
      });
      window.setTimeout(releaseWorkspaceAction, 0);
      window.setTimeout(releaseEditorWriteBarrier, 0);
    })();
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
            pending={pendingMode}
            onSelect={handleSelectMode}
            onSettings={() => setShowSettings(true)}
            onSessions={() => setShowSessions(true)}
            onMemory={() => setShowMemory(true)}
          />

          <main
            className={`work-area work-area--${mode}`}
            key={`${mode}:${resumeTarget?.request ?? "new"}`}
            aria-busy={pendingMode !== null}
          >
            {pendingMode && pendingMode !== mode ? (
              <div className="workspace-switch" role="status" aria-live="polite">
                <AiActivity
                  kind="preparing"
                  label={`正在转往${WORK_MODE_LABEL[pendingMode]}`}
                  detail="先收好当前文稿，再展新卷"
                  compact
                  announce={false}
                />
              </div>
            ) : null}
            <Suspense
              fallback={
                <div className="work-loading" role="status" aria-label="正在载入工作区">
                  <AiActivity
                    kind="preparing"
                    label={`正在展卷 · ${WORK_MODE_LABEL[mode]}`}
                    detail="整理纸页与工作上下文"
                    announce={false}
                  />
                </div>
              }
            >
              {mode === "library"  && <LibraryWork />}
              {mode === "planning" && (
                <PlanningWork
                  onOpenSettings={() => setShowSettings(true)}
                  initialSessionId={
                    resumeTarget?.kind === "planning" ? resumeTarget.sessionId : undefined
                  }
                />
              )}
              {mode === "harness" && (
                <HarnessWork
                  onOpenSettings={() => setShowSettings(true)}
                  initialSessionId={
                    resumeTarget?.kind === "harness" ? resumeTarget.sessionId : undefined
                  }
                />
              )}
              {mode === "discuss"  && (
                <DiscussWork
                  onOpenSettings={() => setShowSettings(true)}
                  initialSessionId={
                    resumeTarget?.kind === "discuss" ? resumeTarget.sessionId : undefined
                  }
                />
              )}
              {mode === "simulate" && <SimulateWork onOpenSettings={() => setShowSettings(true)} />}
              {mode === "studio"   && (
                <StudioWork
                  onOpenSettings={() => setShowSettings(true)}
                  initialSessionId={
                    resumeTarget?.kind === "studio" ? resumeTarget.sessionId : undefined
                  }
                />
              )}
              {mode === "ide"      && <IdeWork onSettingsOpen={() => setShowSettings(true)} />}
              {mode === "revision" && <RevisionWork />}
              {mode === "knowledge"&& <KnowledgeWork />}
              {mode === "style" && <StyleScreen />}
              {mode === "collab" && <CollabScreen />}
              {mode === "checkpoints" && <CheckpointsScreen />}
            </Suspense>
          </main>
        </div>

        <Suspense fallback={null}>
          {showSettings && <SettingsModal onClose={() => setShowSettings(false)} />}

          <SessionsDrawer
            open={showSessions}
            onClose={() => setShowSessions(false)}
            onResume={handleResumeSession}
          />

          <MemoryDrawer open={showMemory} onClose={() => setShowMemory(false)} />
        </Suspense>
      </WorkProvider>
    </ToastProvider>
  );
}
