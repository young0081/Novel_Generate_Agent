// IdeWork — Cursor-style IDE for novel writing.
// Three-panel layout: File tree (left) | Editor (center) | AI assistant (right)
//
// P0-1: externalRevision tracks agent-written files so MarkdownEditor can
//        reload from disk without remounting.
// P0-2: insertRef lets the AI panel push text into the active editor at cursor.
// P1:   AI-toggle uses a proper icon; welcome screen shortcuts corrected.
// P3-1: Drag handles between panels; widths persisted to localStorage.

import { useCallback, useEffect, useRef, useState } from "react";
import FileTree from "../components/ide/FileTree";
import MarkdownEditor from "../components/ide/MarkdownEditor";
import IdeAiPanel from "../components/ide/IdeAiPanel";
import { IconPlus, IconClose, IconFile, IconSave, IconAgentMode } from "../components/icons";
import { invokeTool } from "../lib/core";
import { useToast } from "../components/Toast";
import { EditorView } from "@codemirror/view";

const LS_SIDEBAR_W = "ide.sidebarW";
const LS_AIPANEL_W = "ide.aiPanelW";
const MIN_W = 140;
const MAX_SIDEBAR_W = 360;
const MAX_AIPANEL_W = 480;

function readPx(key: string, fallback: number): number {
  try {
    const v = parseInt(localStorage.getItem(key) ?? "", 10);
    return isNaN(v) ? fallback : v;
  } catch { return fallback; }
}

interface Tab { path: string; dirty: boolean; }
interface IdeWorkProps { onSettingsOpen?: () => void; }

export default function IdeWork({ onSettingsOpen }: IdeWorkProps) {
  const toast = useToast();
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [wordCount, setWordCount] = useState(0);
  const [cursor, setCursor] = useState<{ line: number; col: number } | null>(null);
  const [saving, setSaving] = useState(false);
  const [aiPanelOpen, setAiPanelOpen] = useState(true);
  const editorContentRef = useRef<string>("");

  // ── P3-1: Resizable panels ─────────────────────────────────────
  const [sidebarW, setSidebarW] = useState(() => readPx(LS_SIDEBAR_W, 200));
  const [aiPanelW, setAiPanelW] = useState(() => readPx(LS_AIPANEL_W, 300));
  const dragRef = useRef<{ side: "sidebar" | "ai"; startX: number; startW: number } | null>(null);

  useEffect(() => { localStorage.setItem(LS_SIDEBAR_W, String(sidebarW)); }, [sidebarW]);
  useEffect(() => { localStorage.setItem(LS_AIPANEL_W, String(aiPanelW)); }, [aiPanelW]);

  const onDragStart = useCallback((side: "sidebar" | "ai") => (e: React.MouseEvent) => {
    e.preventDefault();
    dragRef.current = { side, startX: e.clientX, startW: side === "sidebar" ? sidebarW : aiPanelW };
    const onMove = (ev: MouseEvent) => {
      if (!dragRef.current) return;
      const delta = ev.clientX - dragRef.current.startX;
      if (dragRef.current.side === "sidebar") {
        setSidebarW(Math.max(MIN_W, Math.min(MAX_SIDEBAR_W, dragRef.current.startW + delta)));
      } else {
        setAiPanelW(Math.max(MIN_W, Math.min(MAX_AIPANEL_W, dragRef.current.startW - delta)));
      }
    };
    const onUp = () => {
      dragRef.current = null;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, [sidebarW, aiPanelW]);

  // Map of filePath → revision counter. Bumping a counter triggers
  // MarkdownEditor to reload that file from disk via dispatch (no remount).
  const [fileRevisions, setFileRevisions] = useState<Map<string, number>>(new Map());

  // Ref exposed to IdeAiPanel so it can push text into the CodeMirror view.
  // IdeAiPanel calls insertRef.current(text) when the user hits "插入".
  const insertRef = useRef<((text: string) => void) | null>(null);

  // ── File management ─────────────────────────────────────────────

  const openFile = useCallback((path: string) => {
    setTabs((prev) => {
      if (prev.find((t) => t.path === path)) return prev;
      return [...prev, { path, dirty: false }];
    });
    setActiveTab(path);
  }, []);

  const closeTab = useCallback(
    (path: string, e: React.MouseEvent) => {
      e.stopPropagation();
      setTabs((prev) => {
        const next = prev.filter((t) => t.path !== path);
        if (activeTab === path) {
          setActiveTab(next.length > 0 ? next[next.length - 1].path : null);
        }
        return next;
      });
    },
    [activeTab],
  );

  const newFile = useCallback(async () => {
    const name = window.prompt("新文件名（如 book/第一章.md）：");
    if (!name?.trim()) return;
    const path = name.trim();
    try {
      await invokeTool("write_file", {
        path,
        content: `# ${path.replace(/.*\//, "").replace(/\.\w+$/, "")}\n\n`,
      });
      setRefreshKey((k) => k + 1);
      openFile(path);
      toast.ok(`已创建 ${path}`);
    } catch {
      toast.err("创建文件失败");
    }
  }, [openFile, toast]);

  const handleSave = useCallback((path: string) => {
    setTabs((prev) => prev.map((t) => (t.path === path ? { ...t, dirty: false } : t)));
  }, []);

  const handleWordCount = useCallback((n: number) => {
    setWordCount(n);
  }, []);

  const getFileContent = useCallback(() => editorContentRef.current, []);

  // ── P1: File tree callback wiring ──────────────────────────────

  const handleFileDeleted = useCallback((path: string) => {
    setTabs((prev) => prev.filter((t) => t.path !== path));
    setActiveTab((cur) => (cur === path ? null : cur));
    setRefreshKey((k) => k + 1);
  }, []);

  const handleFileRenamed = useCallback((oldPath: string, newPath: string) => {
    setTabs((prev) =>
      prev.map((t) => (t.path === oldPath ? { ...t, path: newPath } : t)),
    );
    setActiveTab((cur) => (cur === oldPath ? newPath : cur));
    setRefreshKey((k) => k + 1);
  }, []);

  // ── P0-1: Called by IdeAiPanel when agent run finishes ──────────
  // writtenPaths: the set of workspace-relative paths the agent wrote to.
  // We bump each path's revision so MarkdownEditor reloads it via dispatch.
  const handleFilesModified = useCallback((writtenPaths: string[]) => {
    if (writtenPaths.length === 0) return;
    setRefreshKey((k) => k + 1); // refresh file tree too
    setFileRevisions((prev) => {
      const next = new Map(prev);
      for (const p of writtenPaths) {
        next.set(p, (next.get(p) ?? 0) + 1);
      }
      return next;
    });
  }, []);

  // ── P0-2: Called by IdeAiPanel with the text to insert ─────────
  // Delegates to the EditorView via insertRef which MarkdownEditor populates.
  const handleInsert = useCallback((text: string) => {
    const fn = insertRef.current;
    if (!fn) {
      toast.err("请先在编辑器中打开一个文件");
      return;
    }
    fn(text);
  }, [toast]);

  return (
    <div
      className="ide"
      style={{
        gridTemplateColumns: aiPanelOpen
          ? `${sidebarW}px 4px 1fr 4px ${aiPanelW}px`
          : `${sidebarW}px 4px 1fr`,
      }}
    >
      {/* ── Left: File tree ── */}
      <aside className="ide__sidebar">
        <div className="ide__sidebar-header">
          <span>文件</span>
          <button className="ide__icon-btn" onClick={newFile} title="新建文件">
            <IconPlus size={14} />
          </button>
        </div>
        <FileTree
          onOpen={openFile}
          activeFile={activeTab}
          refreshKey={refreshKey}
          onFileDeleted={handleFileDeleted}
          onFileRenamed={handleFileRenamed}
        />
      </aside>

      {/* P3-1: 拖拽手柄 — 侧栏/编辑器之间 */}
      <div className="ide__resize-handle" onMouseDown={onDragStart("sidebar")} title="拖拽调整宽度" />

      {/* ── Center: Tabs + Editor ── */}
      <div className="ide__main">
        {/* Tab bar */}
        <div className="ide__tabs">
          {tabs.map((tab) => {
            const name = tab.path.replace(/.*\//, "");
            return (
              <button
                key={tab.path}
                className={`ide__tab${activeTab === tab.path ? " ide__tab--active" : ""}${tab.dirty ? " ide__tab--dirty" : ""}`}
                onClick={() => setActiveTab(tab.path)}
              >
                <IconFile size={12} />
                <span className="ide__tab-name">{name}</span>
                {tab.dirty && <span className="ide__tab-dot" />}
                <button
                  className="ide__tab-close"
                  onClick={(e) => closeTab(tab.path, e)}
                  title="关闭"
                >
                  <IconClose size={10} />
                </button>
              </button>
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
        </div>

        {/* Editor area */}
        <div className="ide__editor-area">
          {activeTab ? (
            <MarkdownEditor
              key={activeTab}
              filePath={activeTab}
              onSave={(path) => { handleSave(path); setSaving(false); }}
              onWordCount={handleWordCount}
              onContentChange={(c) => { editorContentRef.current = c; setSaving(true); }}
              onCursorChange={(line, col) => setCursor({ line, col })}
              externalRevision={fileRevisions.get(activeTab) ?? 0}
              onViewReady={(view: EditorView) => {
                insertRef.current = (text: string) => {
                  const { from } = view.state.selection.main;
                  view.dispatch({
                    changes: { from, insert: text },
                    selection: { anchor: from + text.length },
                  });
                  view.focus();
                };
              }}
              onViewDestroy={() => { insertRef.current = null; }}
            />
          ) : (
            <div className="ide__welcome">
              <div className="ide__welcome-inner">
                <div className="ide__welcome-seal">墨</div>
                <h2>欢迎使用 墨·创作 IDE</h2>
                <p>从左侧文件树中选择一个章节开始编辑</p>
                <p>或点击 <strong>+</strong> 新建文件</p>
                <div className="ide__welcome-tips">
                  <div className="ide__tip">
                    <kbd>Ctrl+S</kbd>
                    <span>保存文件</span>
                  </div>
                  <div className="ide__tip">
                    <kbd>Ctrl+F</kbd>
                    <span>搜索文本</span>
                  </div>
                  <div className="ide__tip">
                    <kbd>Enter</kbd>
                    <span>AI 面板发送</span>
                  </div>
                </div>
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
            {saving
              ? <span className="ide__statusbar-saving">保存中…</span>
              : <span className="ide__statusbar-save">✓ 已保存</span>}
          </div>
        )}
      </div>

      {/* P3-1: 拖拽手柄 — 编辑器/AI 面板之间 */}
      {aiPanelOpen && (
        <div className="ide__resize-handle ide__resize-handle--ai" onMouseDown={onDragStart("ai")} title="拖拽调整宽度" />
      )}

      {/* ── Right: AI panel ── */}
      {aiPanelOpen && (
        <aside className="ide__ai-aside">
          <IdeAiPanel
            filePath={activeTab}
            getFileContent={getFileContent}
            onSettingsOpen={onSettingsOpen}
            onFilesModified={handleFilesModified}
            onInsert={handleInsert}
          />
        </aside>
      )}

      {/* Toggle AI panel — P1: use correct icon */}
      <button
        className={`ide__ai-toggle${aiPanelOpen ? " ide__ai-toggle--open" : ""}`}
        onClick={() => setAiPanelOpen((v) => !v)}
        title={aiPanelOpen ? "收起 AI 面板" : "展开 AI 面板"}
      >
        <IconAgentMode size={14} />
        AI
      </button>
    </div>
  );
}
