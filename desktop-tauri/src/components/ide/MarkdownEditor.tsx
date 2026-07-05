// MarkdownEditor — CodeMirror 6 editor for markdown/plain-text files.
// Water-ink theme, line wrapping, auto-save on change (debounced 800ms).

import { useCallback, useEffect, useRef, useState } from "react";
import { EditorView, keymap, placeholder } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { search, searchKeymap } from "@codemirror/search";
import { markdown } from "@codemirror/lang-markdown";
import { invokeTool } from "../../lib/core";
import { Spinner } from "../Spinner";

interface MarkdownEditorProps {
  filePath: string;
  onSave?: (path: string) => void;
  onWordCount?: (n: number) => void;
  onContentChange?: (content: string) => void;
  /** Bump to reload from disk without remounting (preserves cursor). */
  externalRevision?: number;
  /** Called once the EditorView is ready; use it to wire up insert-at-cursor. */
  onViewReady?: (view: EditorView) => void;
  /** Called when the view is about to be destroyed. */
  onViewDestroy?: () => void;
  /** Called whenever the cursor position changes. */
  onCursorChange?: (line: number, col: number) => void;
}

// Water-ink CodeMirror theme
const inkTheme = EditorView.theme({
  "&": {
    color: "#2c2c2c",
    backgroundColor: "transparent",
    height: "100%",
    fontFamily: "'Source Han Serif CN', '思源宋体', 'Songti SC', Georgia, serif",
    fontSize: "15px",
    lineHeight: "1.9",
  },
  ".cm-content": {
    caretColor: "#c0392b",
    padding: "16px 24px",
    maxWidth: "720px",
    margin: "0 auto",
  },
  "&.cm-focused .cm-cursor": {
    borderLeftColor: "#c0392b",
    borderLeftWidth: "2px",
  },
  ".cm-line": { lineHeight: "1.9" },
  ".cm-activeLine": { backgroundColor: "rgba(139, 90, 43, 0.06)" },
  ".cm-selectionBackground, ::selection": { backgroundColor: "rgba(139, 90, 43, 0.18)" },
  "&.cm-focused .cm-selectionBackground": { backgroundColor: "rgba(139, 90, 43, 0.18)" },
  // Markdown headings get ink-red emphasis
  ".cm-header-1, .cm-header-2, .cm-header-3": { color: "#8b0000", fontWeight: "700" },
  ".cm-header-4, .cm-header-5": { color: "#a0522d" },
  ".cm-strong": { color: "#2c1a0e" },
  ".cm-em": { color: "#5a3e2b", fontStyle: "italic" },
  ".cm-meta, .cm-comment": { color: "#9c8870" },
  ".cm-gutters": { display: "none" },
  ".cm-scroller": { overflow: "auto" },
  ".cm-placeholder": { color: "#b0a090" },
}, { dark: false });

export default function MarkdownEditor({
  filePath, onSave, onWordCount, onContentChange,
  externalRevision, onViewReady, onViewDestroy, onCursorChange,
}: MarkdownEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const filePathRef = useRef(filePath);
  filePathRef.current = filePath;

  const saveFile = useCallback(async (content: string, path: string) => {
    setSaving(true);
    try {
      await invokeTool("write_file", { path, content });
      onSave?.(path);
    } finally {
      setSaving(false);
    }
  }, [onSave]);

  const scheduleSave = useCallback((content: string) => {
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(() => {
      saveFile(content, filePathRef.current);
    }, 800);
  }, [saveFile]);

  // Load file and mount CodeMirror
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    // Destroy old editor first
    if (viewRef.current) {
      viewRef.current.destroy();
      viewRef.current = null;
    }

    (async () => {
      let content = "";
      try {
        const res = await invokeTool<{ content: string }>("read_file", { path: filePath });
        if (res.ok) {
          content = res.data?.content ?? (typeof res.data === "string" ? res.data : "");
        } else {
          setError(`无法读取文件：${filePath}`);
        }
      } catch {
        setError(`读取失败：${filePath}`);
      }

      if (cancelled || !containerRef.current) return;
      setLoading(false);

      // Count words/chars
      const charCount = content.replace(/\s/g, "").length;
      onWordCount?.(charCount);

      const state = EditorState.create({
        doc: content,
        extensions: [
          history(),
          markdown(),
          search({ top: false }),
          keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
          EditorView.lineWrapping,
          placeholder("在此输入章节内容…"),
          inkTheme,
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              const newContent = update.state.doc.toString();
              const nc = newContent.replace(/\s/g, "").length;
              onWordCount?.(nc);
              onContentChange?.(newContent);
              scheduleSave(newContent);
            }
            // Cursor position (line/col) reporting
            if (update.selectionSet || update.docChanged) {
              const sel = update.state.selection.main;
              const line = update.state.doc.lineAt(sel.head);
              onCursorChange?.(line.number, sel.head - line.from + 1);
            }
          }),
        ],
      });

      const view = new EditorView({ state, parent: containerRef.current });
      viewRef.current = view;
      onViewReady?.(view);
    })();

    return () => {
      cancelled = true;
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
      onViewDestroy?.();
    };
  }, [filePath, onWordCount, scheduleSave]);

  // External file change (e.g. agent wrote to disk) — reload without recreating the view.
  // We compare the incoming content against the current doc; if identical we skip the
  // dispatch so the cursor is not disturbed for no reason.
  useEffect(() => {
    if (externalRevision === undefined || externalRevision === 0) return;
    let cancelled = false;
    (async () => {
      try {
        const res = await invokeTool<{ content: string }>("read_file", { path: filePath });
        if (cancelled || !viewRef.current) return;
        const newContent =
          res.ok
            ? (res.data?.content ?? (typeof res.data === "string" ? res.data : ""))
            : "";
        const current = viewRef.current.state.doc.toString();
        if (newContent === current) return; // nothing changed
        const { from } = viewRef.current.state.selection.main;
        const safeFrom = Math.min(from, newContent.length);
        viewRef.current.dispatch({
          changes: { from: 0, to: current.length, insert: newContent },
          selection: { anchor: safeFrom },
        });
        const nc = newContent.replace(/\s/g, "").length;
        onWordCount?.(nc);
        onContentChange?.(newContent);
      } catch {
        // silent — the file may have just been deleted; do nothing
      }
    })();
    return () => { cancelled = true; };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [externalRevision]);

  // Ctrl+S manual save
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "s") {
        e.preventDefault();
        if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
        const content = viewRef.current?.state.doc.toString() ?? "";
        saveFile(content, filePath);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [filePath, saveFile]);

  if (loading) {
    return (
      <div className="mde__loading">
        <Spinner size={20} />
        <span>读取文件…</span>
      </div>
    );
  }

  if (error) {
    return <div className="mde__error">{error}</div>;
  }

  return (
    <div className="mde">
      {saving && <div className="mde__saving">保存中…</div>}
      <div ref={containerRef} className="mde__cm" />
    </div>
  );
}
