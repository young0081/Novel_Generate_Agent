// IdeWork — Cursor-style IDE for novel writing.
// Three-panel layout: File tree (left) | Editor (center) | AI assistant (right)

import { useCallback, useRef, useState } from "react";
import FileTree from "../components/ide/FileTree";
import MarkdownEditor from "../components/ide/MarkdownEditor";
import IdeAiPanel from "../components/ide/IdeAiPanel";
import { IconPlus, IconClose, IconFile, IconSave } from "../components/icons";
import { invokeTool } from "../lib/core";
import { useToast } from "../components/Toast";

interface Tab {
  path: string;
  dirty: boolean;
}

export default function IdeWork() {
  const toast = useToast();
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [wordCount, setWordCount] = useState(0);
  const [aiPanelOpen, setAiPanelOpen] = useState(true);
  const editorContentRef = useRef<string>("");

  // Open a file: add tab if not already open
  const openFile = useCallback((path: string) => {
    setTabs((prev) => {
      if (prev.find((t) => t.path === path)) return prev;
      return [...prev, { path, dirty: false }];
    });
    setActiveTab(path);
  }, []);

  // Close a tab
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

  // Create new file
  const newFile = useCallback(async () => {
    const name = window.prompt("新文件名（如 book/第一章.md）：");
    if (!name?.trim()) return;
    const path = name.trim();
    try {
      await invokeTool("write_file", { path, content: `# ${path.replace(/.*\//, "").replace(/\.\w+$/, "")}\n\n` });
      setRefreshKey((k) => k + 1);
      openFile(path);
      toast.ok(`已创建 ${path}`);
    } catch {
      toast.err("创建文件失败");
    }
  }, [openFile, toast]);

  // Mark tab as saved
  const handleSave = useCallback((path: string) => {
    setTabs((prev) => prev.map((t) => (t.path === path ? { ...t, dirty: false } : t)));
  }, []);

  // Word count update
  const handleWordCount = useCallback((n: number) => {
    setWordCount(n);
  }, []);

  // Get current editor content for AI context
  const getFileContent = useCallback(() => editorContentRef.current, []);

  return (
    <div className="ide">
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
        />
      </aside>

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
          {/* Status bar on the right */}
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
              onSave={handleSave}
              onWordCount={handleWordCount}
              onContentChange={(c) => { editorContentRef.current = c; }}
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
                    <kbd>Enter</kbd>
                    <span>发送给 AI</span>
                  </div>
                </div>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* ── Right: AI panel ── */}
      {aiPanelOpen && (
        <aside className="ide__ai-aside">
          <IdeAiPanel
            filePath={activeTab}
            getFileContent={getFileContent}
          />
        </aside>
      )}

      {/* Toggle AI panel button */}
      <button
        className={`ide__ai-toggle${aiPanelOpen ? " ide__ai-toggle--open" : ""}`}
        onClick={() => setAiPanelOpen((v) => !v)}
        title={aiPanelOpen ? "收起 AI 面板" : "展开 AI 面板"}
      >
        <IconFile size={14} />
        AI
      </button>
    </div>
  );
}
