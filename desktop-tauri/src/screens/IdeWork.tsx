// IdeWork — Cursor-style IDE for novel writing.
// Three-panel layout: File tree (left) | Editor (center) | AI assistant (right)
//
// P0-1: externalRevision tracks agent-written files so MarkdownEditor can
//        reload from disk without remounting.
// P0-2: insertRef lets the AI panel push text into the active editor at cursor.
// P1:   AI-toggle uses a proper icon; welcome screen shortcuts corrected.
// P3-1: Drag handles between panels; widths persisted to localStorage.

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import FileTree from "../components/ide/FileTree";
import MarkdownEditor, {
  type ExternalReloadStatus,
} from "../components/ide/MarkdownEditor";
import IdeAiPanel from "../components/ide/IdeAiPanel";
import NewDocumentDialog, {
  type NewDocumentDraft,
} from "../components/ide/NewDocumentDialog";
import ConfirmModal from "../components/ConfirmModal";
import {
  IconPlus,
  IconClose,
  IconFile,
  IconSave,
  IconAgentMode,
  IconFocus,
} from "../components/icons";
import {
  createWorkspaceFile,
  describeError,
  isWorkspaceTargetExistsError,
  invokeTool,
  requireToolSuccess,
} from "../lib/core";
import {
  parentWorkspacePath,
  selectPreviousChapterPaths,
  type ChapterDirectoryEntry,
  type PreviousChapterContext,
} from "../lib/ideContext";
import {
  acquireEditorWriteBarrier,
  flushPendingEditors,
  tryAcquireWorkspaceAction,
} from "../lib/editorPersistence";
import { useToast } from "../components/Toast";
import { EditorView } from "@codemirror/view";

const LS_SIDEBAR_W = "ide.sidebarW";
const LS_AIPANEL_W = "ide.aiPanelW";
const MIN_W = 140;
const MAX_SIDEBAR_W = 360;
const MAX_AIPANEL_W = 480;
const MAX_PREVIOUS_CHAPTERS = 8;
const MAX_PREVIOUS_FILE_CHARS = 2_800;

interface ListDirData {
  entries?: Array<{
    name?: unknown;
    kind?: unknown;
  }>;
}

function readPx(key: string, fallback: number): number {
  try {
    const v = parseInt(localStorage.getItem(key) ?? "", 10);
    return isNaN(v) ? fallback : v;
  } catch { return fallback; }
}

interface Tab { path: string; dirty: boolean; }
interface IdeWorkProps { onSettingsOpen?: () => void; }

type ResizeSide = "sidebar" | "ai";

interface ResizeDrag {
  side: ResizeSide;
  pointerId: number;
  startX: number;
  startW: number;
  nextW: number;
  frame: number | null;
}

export default function IdeWork({ onSettingsOpen }: IdeWorkProps) {
  const toast = useToast();
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [wordCount, setWordCount] = useState(0);
  const [cursor, setCursor] = useState<{ line: number; col: number } | null>(null);
  const [aiPanelOpen, setAiPanelOpen] = useState(true);
  const [focusMode, setFocusMode] = useState(false);
  const [agentRunning, setAgentRunning] = useState(false);
  const [newDocumentOpen, setNewDocumentOpen] = useState(false);
  const [creatingDocument, setCreatingDocument] = useState(false);
  const [pendingOverwrite, setPendingOverwrite] = useState<NewDocumentDraft | null>(null);
  const editorViewRef = useRef<EditorView | null>(null);
  const savedContentRef = useRef(new Map<string, string>());
  const agentRunningRef = useRef(false);
  const ideRef = useRef<HTMLDivElement>(null);
  const tabRefs = useRef(new Map<string, HTMLDivElement>());
  agentRunningRef.current = agentRunning;

  // ── P3-1: Resizable panels ─────────────────────────────────────
  const [sidebarW, setSidebarW] = useState(() => readPx(LS_SIDEBAR_W, 200));
  const [aiPanelW, setAiPanelW] = useState(() => readPx(LS_AIPANEL_W, 300));
  const dragRef = useRef<ResizeDrag | null>(null);
  const finishResizeRef = useRef<((commit: boolean) => void) | null>(null);

  useEffect(() => {
    try { localStorage.setItem(LS_SIDEBAR_W, String(sidebarW)); } catch { /* unavailable */ }
  }, [sidebarW]);
  useEffect(() => {
    try { localStorage.setItem(LS_AIPANEL_W, String(aiPanelW)); } catch { /* unavailable */ }
  }, [aiPanelW]);

  useEffect(() => {
    document.body.classList.toggle("ide-focus-mode", focusMode);
    return () => document.body.classList.remove("ide-focus-mode");
  }, [focusMode]);

  useEffect(() => {
    return () => finishResizeRef.current?.(false);
  }, []);

  const onDragStart = useCallback((side: ResizeSide) => (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || window.matchMedia("(max-width: 1160px)").matches) return;
    const root = ideRef.current;
    if (!root) return;

    event.preventDefault();
    finishResizeRef.current?.(false);

    const handle = event.currentTarget;
    const drag: ResizeDrag = {
      side,
      pointerId: event.pointerId,
      startX: event.clientX,
      startW: side === "sidebar" ? sidebarW : aiPanelW,
      nextW: side === "sidebar" ? sidebarW : aiPanelW,
      frame: null,
    };
    dragRef.current = drag;
    root.classList.add("is-resizing");
    try { handle.setPointerCapture(event.pointerId); } catch { /* unsupported */ }

    const property = side === "sidebar" ? "--ide-sidebar-w" : "--ide-ai-panel-w";
    const max = side === "sidebar" ? MAX_SIDEBAR_W : MAX_AIPANEL_W;
    let settled = false;

    const paintPreview = () => {
      drag.frame = null;
      root.style.setProperty(property, `${drag.nextW}px`);
    };

    const removeListeners = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onPointerEnd);
      window.removeEventListener("pointercancel", onPointerEnd);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("resize", onViewportResize);
      window.removeEventListener("keydown", onKeyDown);
    };

    const finish = (commit: boolean) => {
      if (settled) return;
      settled = true;
      removeListeners();
      if (drag.frame !== null) window.cancelAnimationFrame(drag.frame);
      const width = commit ? drag.nextW : drag.startW;
      root.style.setProperty(property, `${width}px`);
      root.classList.remove("is-resizing");
      try {
        if (handle.hasPointerCapture(drag.pointerId)) handle.releasePointerCapture(drag.pointerId);
      } catch { /* already released */ }
      dragRef.current = null;
      finishResizeRef.current = null;
      if (commit && width !== drag.startW) {
        if (side === "sidebar") setSidebarW(width);
        else setAiPanelW(width);
      }
    };

    function onMove(pointerEvent: PointerEvent) {
      if (pointerEvent.pointerId !== drag.pointerId) return;
      const delta = pointerEvent.clientX - drag.startX;
      const rawWidth = side === "sidebar" ? drag.startW + delta : drag.startW - delta;
      drag.nextW = Math.max(MIN_W, Math.min(max, rawWidth));
      if (drag.frame === null) drag.frame = window.requestAnimationFrame(paintPreview);
    }

    function onPointerEnd(pointerEvent: PointerEvent) {
      if (pointerEvent.pointerId === drag.pointerId) finish(true);
    }

    function onBlur() {
      finish(true);
    }

    function onViewportResize() {
      if (window.matchMedia("(max-width: 1160px)").matches) finish(true);
    }

    function onKeyDown(keyEvent: KeyboardEvent) {
      if (keyEvent.key === "Escape") {
        keyEvent.preventDefault();
        finish(false);
      }
    }

    finishResizeRef.current = finish;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onPointerEnd);
    window.addEventListener("pointercancel", onPointerEnd);
    window.addEventListener("blur", onBlur);
    window.addEventListener("resize", onViewportResize);
    window.addEventListener("keydown", onKeyDown);
  }, [sidebarW, aiPanelW]);

  const onResizeKeyDown = useCallback(
    (side: ResizeSide) => (event: React.KeyboardEvent) => {
      const decreaseKey = side === "sidebar" ? "ArrowLeft" : "ArrowRight";
      const increaseKey = side === "sidebar" ? "ArrowRight" : "ArrowLeft";
      if (![decreaseKey, increaseKey, "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      const min = MIN_W;
      const max = side === "sidebar" ? MAX_SIDEBAR_W : MAX_AIPANEL_W;
      const setWidth = side === "sidebar" ? setSidebarW : setAiPanelW;
      setWidth((current) => {
        if (event.key === "Home") return min;
        if (event.key === "End") return max;
        return Math.max(min, Math.min(max, current + (event.key === increaseKey ? 12 : -12)));
      });
    },
    [],
  );

  // Map of filePath → revision counter. Bumping a counter triggers
  // MarkdownEditor to reload that file from disk via dispatch (no remount).
  const [fileRevisions, setFileRevisions] = useState<Map<string, number>>(new Map());
  const reloadWaiterRef = useRef<{
    path: string;
    resolve: () => void;
    timer: ReturnType<typeof setTimeout>;
  } | null>(null);

  useEffect(() => {
    return () => {
      const waiter = reloadWaiterRef.current;
      if (waiter) {
        clearTimeout(waiter.timer);
        waiter.resolve();
        reloadWaiterRef.current = null;
      }
    };
  }, []);

  // Ref exposed to IdeAiPanel so it can push text into the CodeMirror view.
  // IdeAiPanel calls insertRef.current(text) when the user hits "插入".
  const insertRef = useRef<((text: string) => void) | null>(null);

  // ── File management ─────────────────────────────────────────────

  const openFile = useCallback(async (path: string): Promise<boolean> => {
    if (agentRunning && path !== activeTab) {
      toast.info("运笔完成前无法切换文件");
      return false;
    }
    if (path !== activeTab && !(await flushPendingEditors())) {
      toast.err("当前文件保存失败，已取消切换");
      return false;
    }
    setTabs((prev) => {
      if (prev.find((t) => t.path === path)) return prev;
      return [...prev, { path, dirty: false }];
    });
    setWordCount(0);
    setCursor(null);
    setActiveTab(path);
    return true;
  }, [activeTab, agentRunning, toast]);

  const closeTab = useCallback(
    async (path: string, e: React.MouseEvent) => {
      e.stopPropagation();
      if (agentRunning) {
        toast.info("运笔完成前无法关闭文件");
        return;
      }
      if (path === activeTab && !(await flushPendingEditors())) {
        toast.err("文件保存失败，已取消关闭");
        return;
      }
      setTabs((prev) => {
        const next = prev.filter((t) => t.path !== path);
        if (activeTab === path) {
          setActiveTab(next.length > 0 ? next[next.length - 1].path : null);
        }
        return next;
      });
    },
    [activeTab, agentRunning, toast],
  );

  const openNewDocument = useCallback(() => {
    if (agentRunning) {
      toast.info("运笔完成前无法新建文件");
      return;
    }
    setNewDocumentOpen(true);
  }, [agentRunning, toast]);

  const createDocument = useCallback(async (
    draft: NewDocumentDraft,
    overwrite = false,
  ) => {
    if (agentRunning || creatingDocument) return;
    const releaseWorkspaceAction = tryAcquireWorkspaceAction();
    if (!releaseWorkspaceAction) {
      toast.info("运笔或其他文件操作完成前无法新建文件");
      return;
    }
    const releaseEditorWriteBarrier = acquireEditorWriteBarrier();
    setCreatingDocument(true);
    try {
      if (!(await flushPendingEditors())) {
        toast.err("当前文件保存失败，已取消新建");
        return;
      }
      const content = `# ${draft.title}\n\n`;
      try {
        await createWorkspaceFile(draft.path, content, overwrite);
      } catch (error) {
        if (!overwrite && isWorkspaceTargetExistsError(error)) {
          setNewDocumentOpen(false);
          setPendingOverwrite(draft);
          return;
        }
        throw error;
      }
      savedContentRef.current.set(draft.path, content);
      setRefreshKey((k) => k + 1);
      if (draft.path === activeTab) {
        setFileRevisions((prev) => {
          const next = new Map(prev);
          next.set(draft.path, (next.get(draft.path) ?? 0) + 1);
          return next;
        });
      } else {
        setTabs((prev) => prev.some((tab) => tab.path === draft.path)
          ? prev
          : [...prev, { path: draft.path, dirty: false }]);
        setWordCount(0);
        setCursor(null);
        setActiveTab(draft.path);
      }
      setNewDocumentOpen(false);
      setPendingOverwrite(null);
      toast.ok(`已创建 ${draft.kind === "chapter" ? "章节" : "资料"}「${draft.title}」`);
    } catch (error) {
      toast.err(`创建文件失败：${describeError(error)}`);
    } finally {
      setCreatingDocument(false);
      releaseWorkspaceAction();
      window.setTimeout(releaseEditorWriteBarrier, 0);
    }
  }, [activeTab, agentRunning, creatingDocument, toast]);

  const handleSave = useCallback((path: string, savedContent: string) => {
    savedContentRef.current.set(path, savedContent);
    setTabs((prev) => prev.map((tab) => {
      if (tab.path !== path) return tab;
      const currentContent = activeTab === path
        ? editorViewRef.current?.state.doc.toString() ?? savedContent
        : savedContent;
      return { ...tab, dirty: currentContent !== savedContent };
    }));
  }, [activeTab]);

  const handleDirty = useCallback((path: string) => {
    setTabs((prev) => {
      const index = prev.findIndex((tab) => tab.path === path);
      if (index < 0 || prev[index].dirty) return prev;
      const next = [...prev];
      next[index] = { ...next[index], dirty: true };
      return next;
    });
  }, []);

  const handleWordCount = useCallback((n: number) => {
    setWordCount(n);
  }, []);

  const getFileContent = useCallback(
    () => editorViewRef.current?.state.doc.toString() ?? "",
    [],
  );

  /**
   * Load a bounded, chronological slice of the chapters before the active
   * file.  This is deliberately best-effort: an unavailable directory or a
   * single unreadable chapter must never prevent editing the current file.
   */
  const getPreviousChapterContext = useCallback(async (
    path: string | null,
  ): Promise<PreviousChapterContext> => {
    if (!path) return { files: [], omitted: 0, failed: [] };

    const directory = parentWorkspacePath(path);
    try {
      const listing = requireToolSuccess(
        await invokeTool<ListDirData>("list_dir", { path: directory }),
        `无法读取章节目录：${directory || "工作区"}`,
      );
      const entries: ChapterDirectoryEntry[] = Array.isArray(listing.data?.entries)
        ? listing.data.entries.flatMap((entry) => {
            if (typeof entry.name !== "string") return [];
            const kind = entry.kind;
            if (kind !== "dir" && kind !== "file" && kind !== "other") return [];
            return [{ name: entry.name, kind }];
          })
        : listing.content
            .split(/\r?\n/)
            .map((line) => line.trim())
            .filter(Boolean)
            .map((line) => ({
              name: line.endsWith("/") ? line.slice(0, -1) : line,
              kind: line.endsWith("/") ? "dir" as const : "file" as const,
            }));

      const selection = selectPreviousChapterPaths(path, entries, MAX_PREVIOUS_CHAPTERS);
      if (selection.paths.length === 0) {
        return { files: [], omitted: selection.omitted, failed: [] };
      }

      const reads = await Promise.all(selection.paths.map(async (chapterPath) => {
        try {
          const result = requireToolSuccess(
            await invokeTool("read_file", { path: chapterPath }),
            `无法读取章节：${chapterPath}`,
          );
          const content = result.content.length > MAX_PREVIOUS_FILE_CHARS
            ? `${result.content.slice(0, MAX_PREVIOUS_FILE_CHARS)}\n…（该文件已截断）`
            : result.content;
          return { path: chapterPath, content, failed: false };
        } catch {
          return { path: chapterPath, content: "", failed: true };
        }
      }));

      return {
        files: reads
          .filter((item) => !item.failed)
          .map(({ path: chapterPath, content }) => ({ path: chapterPath, content })),
        omitted: selection.omitted,
        failed: reads.filter((item) => item.failed).map((item) => item.path),
      };
    } catch (error) {
      return {
        files: [],
        omitted: 0,
        failed: [],
        error: describeError(error),
      };
    }
  }, []);

  // ── P1: File tree callback wiring ──────────────────────────────

  const handleFileDeleted = useCallback((path: string) => {
    savedContentRef.current.delete(path);
    setTabs((prev) => {
      const next = prev.filter((t) => t.path !== path);
      setActiveTab((cur) =>
        cur === path ? next[next.length - 1]?.path ?? null : cur,
      );
      return next;
    });
    setRefreshKey((k) => k + 1);
  }, []);

  const handleFileRenamed = useCallback((oldPath: string, newPath: string) => {
    const savedContent = savedContentRef.current.get(oldPath);
    savedContentRef.current.delete(oldPath);
    if (savedContent !== undefined) savedContentRef.current.set(newPath, savedContent);
    setTabs((prev) => {
      const oldIndex = prev.findIndex((tab) => tab.path === oldPath);
      if (oldIndex < 0) return prev;
      const renamed = { ...prev[oldIndex], path: newPath };
      const next = prev.filter(
        (tab) => tab.path !== oldPath && tab.path !== newPath,
      );
      next.splice(Math.min(oldIndex, next.length), 0, renamed);
      return next;
    });
    setActiveTab((cur) => (cur === oldPath ? newPath : cur));
    setFileRevisions((prev) => {
      const next = new Map(prev);
      next.delete(oldPath);
      next.set(newPath, (next.get(newPath) ?? 0) + 1);
      return next;
    });
    setRefreshKey((k) => k + 1);
  }, []);

  // ── P0-1: Called by IdeAiPanel when agent run finishes ──────────
  // writtenPaths: the set of workspace-relative paths the agent wrote to.
  // We bump each path's revision so MarkdownEditor reloads it via dispatch.
  const handleFilesModified = useCallback((writtenPaths: string[]): Promise<void> => {
    setRefreshKey((k) => k + 1); // refresh file tree too
    const currentPath = activeTab;
    const normalizedPaths = writtenPaths.map((path) =>
      path.replace(/\\/g, "/").replace(/^\.\//, ""),
    );

    let waitForReload: Promise<void> = Promise.resolve();
    if (currentPath) {
      waitForReload = new Promise<void>((resolve) => {
        const previous = reloadWaiterRef.current;
        if (previous) {
          clearTimeout(previous.timer);
          previous.resolve();
        }
        const timer = setTimeout(() => {
          const pending = reloadWaiterRef.current;
          if (pending?.path === currentPath) {
            reloadWaiterRef.current = null;
            toast.err("重新载入 AI 修改后的文件超时");
            pending.resolve();
          }
        }, 30_000);
        reloadWaiterRef.current = { path: currentPath, resolve, timer };
      });
    }

    setFileRevisions((prev) => {
      const next = new Map(prev);
      for (const p of normalizedPaths) {
        next.set(p, (next.get(p) ?? 0) + 1);
      }
      // Always verify the visible document after an agent run. This also
      // covers tools whose mutation event did not expose a path.
      if (currentPath && !normalizedPaths.includes(currentPath)) {
        next.set(currentPath, (next.get(currentPath) ?? 0) + 1);
      }
      return next;
    });
    return waitForReload;
  }, [activeTab, toast]);

  const handleExternalReloadComplete = useCallback((
    path: string,
    _revision: number,
    status: ExternalReloadStatus,
  ) => {
    if (status !== "loaded") {
      handleFileDeleted(path);
      setFileRevisions((prev) => {
        const next = new Map(prev);
        next.delete(path);
        return next;
      });
      if (status === "missing") {
        toast.info(`AI 已删除或移动 ${path}，旧标签页已关闭`);
      } else {
        toast.err(`无法验证 AI 修改后的 ${path}，标签页已关闭以防覆盖`);
      }
    }
    const waiter = reloadWaiterRef.current;
    if (!waiter || waiter.path !== path) return;
    clearTimeout(waiter.timer);
    reloadWaiterRef.current = null;
    waiter.resolve();
  }, [handleFileDeleted, toast]);

  // ── P0-2: Called by IdeAiPanel with the text to insert ─────────
  // Delegates to the EditorView via insertRef which MarkdownEditor populates.
  const handleInsert = useCallback((text: string) => {
    if (agentRunning) {
      toast.info("运笔完成前无法插入内容");
      return;
    }
    const fn = insertRef.current;
    if (!fn) {
      toast.err("请先在编辑器中打开一个文件");
      return;
    }
    fn(text);
  }, [agentRunning, toast]);

  const handleTabKeyDown = useCallback((
    event: React.KeyboardEvent<HTMLDivElement>,
    index: number,
  ) => {
    if (event.target !== event.currentTarget) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      void openFile(tabs[index].path);
      return;
    }

    let nextIndex: number | null = null;
    if (event.key === "ArrowLeft") nextIndex = (index - 1 + tabs.length) % tabs.length;
    if (event.key === "ArrowRight") nextIndex = (index + 1) % tabs.length;
    if (event.key === "Home") nextIndex = 0;
    if (event.key === "End") nextIndex = tabs.length - 1;
    if (nextIndex === null || nextIndex === index) return;

    event.preventDefault();
    const currentTarget = event.currentTarget;
    const nextTab = tabs[nextIndex];
    if (!agentRunning) tabRefs.current.get(nextTab.path)?.focus();
    void openFile(nextTab.path).then((opened) => {
      if (!opened && currentTarget.isConnected) currentTarget.focus();
    });
  }, [agentRunning, openFile, tabs]);

  const activeDirty = tabs.find((tab) => tab.path === activeTab)?.dirty ?? false;

  const ideStyle = {
    "--ide-sidebar-w": `${sidebarW}px`,
    "--ide-ai-panel-w": `${aiPanelW}px`,
    gridTemplateColumns: focusMode
      ? "minmax(0, 1fr)"
      : aiPanelOpen
        ? "var(--ide-sidebar-w) 4px minmax(0, 1fr) 4px var(--ide-ai-panel-w)"
        : "var(--ide-sidebar-w) 4px minmax(0, 1fr)",
  } as CSSProperties;

  return (
    <div
      ref={ideRef}
      className="ide"
      style={ideStyle}
    >
      {/* ── Left: File tree ── */}
      <aside className="ide__sidebar" hidden={focusMode}>
        <div className="ide__sidebar-header">
          <span>文件</span>
          <button className="ide__icon-btn" onClick={openNewDocument} title="新建文稿" aria-label="新建文稿" disabled={agentRunning}>
            <IconPlus size={14} />
          </button>
        </div>
        <FileTree
          onOpen={(path) => { void openFile(path); }}
          activeFile={activeTab}
          refreshKey={refreshKey}
          onFileDeleted={handleFileDeleted}
          onFileRenamed={handleFileRenamed}
          disabled={agentRunning}
        />
      </aside>

      {/* P3-1: 拖拽手柄 — 侧栏/编辑器之间 */}
      {!focusMode && (
        <div
          className="ide__resize-handle"
          role="separator"
          tabIndex={0}
          aria-label="调整文件区宽度"
          aria-orientation="vertical"
          aria-valuemin={MIN_W}
          aria-valuemax={MAX_SIDEBAR_W}
          aria-valuenow={sidebarW}
          onPointerDown={onDragStart("sidebar")}
          onKeyDown={onResizeKeyDown("sidebar")}
          title="调整文件区宽度"
        />
      )}

      {/* ── Center: Tabs + Editor ── */}
      <div className="ide__main">
        {/* Tab bar */}
        <div className="ide__tabs" role="tablist" aria-label="已打开文稿">
          {tabs.map((tab, index) => {
            const name = tab.path.replace(/.*\//, "");
            return (
              <div
                ref={(node) => {
                  if (node) tabRefs.current.set(tab.path, node);
                  else tabRefs.current.delete(tab.path);
                }}
                key={tab.path}
                className={`ide__tab${activeTab === tab.path ? " ide__tab--active" : ""}${tab.dirty ? " ide__tab--dirty" : ""}`}
                role="tab"
                tabIndex={activeTab === tab.path || (activeTab === null && index === 0) ? 0 : -1}
                aria-selected={activeTab === tab.path}
                aria-disabled={agentRunning}
                onClick={() => { void openFile(tab.path); }}
                onKeyDown={(event) => handleTabKeyDown(event, index)}
              >
                <IconFile size={12} />
                <span className="ide__tab-name">{name}</span>
                {tab.dirty && <span className="ide__tab-dot" />}
                <button
                  className="ide__tab-close"
                  onClick={(e) => { void closeTab(tab.path, e); }}
                  title="关闭"
                  aria-label={`关闭 ${name}`}
                  disabled={agentRunning}
                >
                  <IconClose size={10} />
                </button>
              </div>
            );
          })}
          {tabs.length === 0 && (
            <span className="ide__tabs-empty">点击左侧文件打开编辑</span>
          )}
          {activeTab && (
            <div className="ide__statusbar">
              {wordCount > 0 && <span>{wordCount} 字</span>}
              <IconSave size={12} />
              <span>自动保存</span>
            </div>
          )}
          <button
            type="button"
            className={`ide__toolbar-toggle ide__ai-panel-toggle${aiPanelOpen ? " is-active" : ""}`}
            onClick={() => setAiPanelOpen((value) => !value)}
            title={aiPanelOpen ? "收起 AI 面板" : "展开 AI 面板"}
            aria-label={aiPanelOpen ? "收起 AI 面板" : "展开 AI 面板"}
            aria-pressed={aiPanelOpen}
            disabled={agentRunning}
            hidden={focusMode}
          >
            <IconAgentMode size={14} />
            <span>AI</span>
          </button>
          <button
            type="button"
            className={`ide__toolbar-toggle ide__focus-toggle${focusMode ? " is-active" : ""}`}
            onClick={() => setFocusMode((value) => !value)}
            aria-pressed={focusMode}
            title={agentRunning ? "改稿运行中不可隐藏 AI 面板" : focusMode ? "退出专注模式" : "进入专注模式"}
            disabled={agentRunning}
          >
            <IconFocus size={14} />
            <span>{focusMode ? "退出专注" : "专注"}</span>
          </button>
        </div>

        {/* Editor area */}
        <div className="ide__editor-area">
          {activeTab ? (
            <MarkdownEditor
              key={activeTab}
              filePath={activeTab}
              onSave={handleSave}
              onWordCount={handleWordCount}
              onDirty={handleDirty}
              onCursorChange={(line, col) => setCursor({ line, col })}
              externalRevision={fileRevisions.get(activeTab) ?? 0}
              readOnly={agentRunning}
              onExternalReloadComplete={handleExternalReloadComplete}
              onViewReady={(view: EditorView) => {
                editorViewRef.current = view;
                insertRef.current = (text: string) => {
                  if (agentRunningRef.current) return;
                  const { from } = view.state.selection.main;
                  view.dispatch({
                    changes: { from, insert: text },
                    selection: { anchor: from + text.length },
                  });
                  view.focus();
                };
              }}
              onViewDestroy={() => {
                editorViewRef.current = null;
                insertRef.current = null;
              }}
            />
          ) : (
            <div className="ide__welcome">
              <div className="ide__welcome-inner">
                <div className="ide__welcome-seal">墨</div>
                <h2>尚未打开文稿</h2>
                <button type="button" className="btn btn--primary" onClick={openNewDocument}>
                  <IconPlus size={14} />
                  新建章节
                </button>
              </div>
            </div>
          )}
        </div>

        {/* P2 底部状态栏 */}
        {activeTab && (
          <div className="ide__statusbar-bottom">
            <span>{activeTab.replace(/.*\//, "")}</span>
            <div className="ide__statusbar-sep" />
            {cursor && <span>第 {cursor.line} 行，第 {cursor.col} 列</span>}
            {cursor && <div className="ide__statusbar-sep" />}
            {wordCount > 0 && <span>{wordCount} 字</span>}
            <div style={{ flex: 1 }} />
            {activeDirty
              ? <span className="ide__statusbar-saving">保存中…</span>
              : <span className="ide__statusbar-save">✓ 已保存</span>}
          </div>
        )}
      </div>

      {/* P3-1: 拖拽手柄 — 编辑器/AI 面板之间 */}
      {aiPanelOpen && !focusMode && (
        <div
          className="ide__resize-handle ide__resize-handle--ai"
          role="separator"
          tabIndex={0}
          aria-label="调整 AI 区宽度"
          aria-orientation="vertical"
          aria-valuemin={MIN_W}
          aria-valuemax={MAX_AIPANEL_W}
          aria-valuenow={aiPanelW}
          onPointerDown={onDragStart("ai")}
          onKeyDown={onResizeKeyDown("ai")}
          title="调整 AI 区宽度"
        />
      )}

      {/* ── Right: AI panel ── */}
      <aside className="ide__ai-aside" hidden={!aiPanelOpen || focusMode}>
        <IdeAiPanel
          filePath={activeTab}
          getFileContent={getFileContent}
          getPreviousChapterContext={getPreviousChapterContext}
          onSettingsOpen={onSettingsOpen}
          onFilesModified={handleFilesModified}
          onAgentRunningChange={setAgentRunning}
          onInsert={handleInsert}
        />
      </aside>

      <NewDocumentDialog
        open={newDocumentOpen}
        busy={creatingDocument}
        onCancel={() => setNewDocumentOpen(false)}
        onCreate={(draft) => { void createDocument(draft); }}
      />

      <ConfirmModal
        open={!!pendingOverwrite}
        title="覆盖已有文稿？"
        body={pendingOverwrite ? `「${pendingOverwrite.path}」已经存在，覆盖后原内容无法恢复。` : ""}
        confirmLabel="覆盖并打开"
        danger
        busy={creatingDocument}
        onCancel={() => setPendingOverwrite(null)}
        onConfirm={() => {
          if (pendingOverwrite) void createDocument(pendingOverwrite, true);
        }}
      />
    </div>
  );
}
