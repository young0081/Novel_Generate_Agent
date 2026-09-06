// MarkdownEditor — CodeMirror 6 editor for markdown/plain-text files.
// Water-ink theme, line wrapping, auto-save on change (debounced 800ms).

import { useCallback, useEffect, useRef, useState } from "react";
import { EditorView, keymap, placeholder } from "@codemirror/view";
import { Compartment, EditorState, type Text } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { search, searchKeymap } from "@codemirror/search";
import { markdown } from "@codemirror/lang-markdown";
import {
  describeError,
  invokeTool,
  requireToolSuccess,
} from "../../lib/core";
import {
  areEditorWritesBlocked,
  registerEditorFlush,
} from "../../lib/editorPersistence";
import { Spinner } from "../Spinner";

interface MarkdownEditorProps {
  filePath: string;
  /** Reports the exact revision that reached disk. */
  onSave?: (path: string, content: string) => void;
  onWordCount?: (n: number) => void;
  onDirty?: (path: string) => void;
  /** Bump to reload from disk without remounting (preserves cursor). */
  externalRevision?: number;
  /** Prevent user and keyboard-triggered writes while an agent owns the workspace. */
  readOnly?: boolean;
  /** Called once the EditorView is ready; use it to wire up insert-at-cursor. */
  onViewReady?: (view: EditorView) => void;
  /** Called when the view is about to be destroyed. */
  onViewDestroy?: () => void;
  /** Called whenever the cursor position changes. */
  onCursorChange?: (line: number, col: number) => void;
  /** Signals that an external-revision disk reload has settled. */
  onExternalReloadComplete?: (
    path: string,
    revision: number,
    status: ExternalReloadStatus,
  ) => void;
}

export type ExternalReloadStatus = "loaded" | "missing" | "error";

function isMissingFileMessage(message: string): boolean {
  return /file not found|no such file|cannot find|找不到|不存在/i.test(message);
}

function nonWhitespaceCount(value: string): number {
  return value.replace(/\s/g, "").length;
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
  filePath, onSave, onWordCount, onDirty,
  externalRevision, readOnly = false, onViewReady, onViewDestroy, onCursorChange,
  onExternalReloadComplete,
}: MarkdownEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingSaveRef = useRef<{ doc: Text; path: string } | null>(null);
  const metricsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingMetricsRef = useRef<{ count: number; line: number; col: number } | null>(null);
  const saveChainRef = useRef<Promise<boolean>>(Promise.resolve(true));
  const mountedRef = useRef(true);
  const applyingExternalRef = useRef(false);
  const readOnlyRef = useRef(readOnly);
  const barrierLockedRef = useRef(areEditorWritesBlocked());
  const pathWritableRef = useRef(false);
  const invalidPathsRef = useRef(new Set<string>());
  const readOnlyCompartmentRef = useRef(new Compartment());
  readOnlyRef.current = readOnly;
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const filePathRef = useRef(filePath);
  filePathRef.current = filePath;
  const callbacksRef = useRef({
    onSave,
    onWordCount,
    onDirty,
    onViewReady,
    onViewDestroy,
    onCursorChange,
    onExternalReloadComplete,
  });
  callbacksRef.current = {
    onSave,
    onWordCount,
    onDirty,
    onViewReady,
    onViewDestroy,
    onCursorChange,
    onExternalReloadComplete,
  };

  const flushMetrics = useCallback(() => {
    if (metricsTimerRef.current) {
      clearTimeout(metricsTimerRef.current);
      metricsTimerRef.current = null;
    }
    const pending = pendingMetricsRef.current;
    pendingMetricsRef.current = null;
    if (!pending) return;
    callbacksRef.current.onWordCount?.(pending.count);
    callbacksRef.current.onCursorChange?.(pending.line, pending.col);
  }, []);

  const scheduleMetrics = useCallback((count: number, line: number, col: number) => {
    pendingMetricsRef.current = { count, line, col };
    if (metricsTimerRef.current) return;
    metricsTimerRef.current = setTimeout(flushMetrics, 80);
  }, [flushMetrics]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const saveFile = useCallback((content: string, path: string): Promise<boolean> => {
    const run = async () => {
      if (invalidPathsRef.current.has(path)) return true;
      if (mountedRef.current) {
        setSaving(true);
        setSaveError(null);
      }
      try {
        requireToolSuccess(
          await invokeTool("write_file", { path, content }),
          `保存失败：${path}`,
        );
        if (mountedRef.current) callbacksRef.current.onSave?.(path, content);
        return true;
      } catch (saveFailure) {
        if (mountedRef.current) setSaveError(describeError(saveFailure));
        return false;
      } finally {
        if (mountedRef.current) setSaving(false);
      }
    };
    const next = saveChainRef.current.catch(() => false).then(run);
    saveChainRef.current = next;
    return next;
  }, []);

  const scheduleSave = useCallback((doc: Text) => {
    if (
      readOnlyRef.current ||
      barrierLockedRef.current ||
      !pathWritableRef.current
    ) return;
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    pendingSaveRef.current = { doc, path: filePathRef.current };
    saveTimerRef.current = setTimeout(() => {
      saveTimerRef.current = null;
      const pending = pendingSaveRef.current;
      pendingSaveRef.current = null;
      if (pending) void saveFile(pending.doc.toString(), pending.path);
    }, 800);
  }, [saveFile]);

  const flush = useCallback(async (): Promise<boolean> => {
    if (saveTimerRef.current) {
      clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
    }
    const pending = pendingSaveRef.current;
    pendingSaveRef.current = null;
    if (pending) return saveFile(pending.doc.toString(), pending.path);
    return saveChainRef.current.catch(() => false);
  }, [saveFile]);

  useEffect(() => registerEditorFlush(flush, (locked) => {
    barrierLockedRef.current = locked;
    const view = viewRef.current;
    if (!view) return;
    const editorReadOnly = readOnlyRef.current || locked || !pathWritableRef.current;
    view.dispatch({
      effects: readOnlyCompartmentRef.current.reconfigure([
        EditorState.readOnly.of(editorReadOnly),
        EditorView.editable.of(!editorReadOnly),
      ]),
    });
  }), [flush]);

  // Load file and mount CodeMirror
  useEffect(() => {
    let cancelled = false;
    pathWritableRef.current = false;
    setLoading(true);
    setError(null);

    // Destroy old editor first
    if (viewRef.current) {
      viewRef.current.destroy();
      viewRef.current = null;
    }

    (async () => {
      let content = "";
      let loadError: string | null = null;
      try {
        const res = await invokeTool("read_file", { path: filePath });
        if (res.ok) {
          content = res.content;
        } else {
          loadError = res.content || `无法读取文件：${filePath}`;
        }
      } catch (loadFailure) {
        loadError = describeError(loadFailure);
      }

      if (cancelled || !containerRef.current) return;
      setLoading(false);
      if (loadError) {
        invalidPathsRef.current.add(filePath);
        setError(loadError);
        callbacksRef.current.onExternalReloadComplete?.(
          filePath,
          externalRevision ?? 0,
          isMissingFileMessage(loadError) ? "missing" : "error",
        );
        return;
      }

      invalidPathsRef.current.delete(filePath);
      pathWritableRef.current = true;

      // Count words/chars
      let charCount = nonWhitespaceCount(content);
      callbacksRef.current.onWordCount?.(charCount);
      callbacksRef.current.onSave?.(filePath, content);

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
          readOnlyCompartmentRef.current.of([
            EditorState.readOnly.of(readOnlyRef.current || barrierLockedRef.current),
            EditorView.editable.of(!(readOnlyRef.current || barrierLockedRef.current)),
          ]),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              update.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
                charCount -= nonWhitespaceCount(update.startState.doc.sliceString(fromA, toA));
                charCount += nonWhitespaceCount(inserted.toString());
              });
              if (!applyingExternalRef.current) {
                callbacksRef.current.onDirty?.(filePath);
                scheduleSave(update.state.doc);
              }
            }
            if (update.selectionSet || update.docChanged) {
              const sel = update.state.selection.main;
              const line = update.state.doc.lineAt(sel.head);
              scheduleMetrics(charCount, line.number, sel.head - line.from + 1);
            }
          }),
        ],
      });

      const view = new EditorView({ state, parent: containerRef.current });
      viewRef.current = view;
      callbacksRef.current.onViewReady?.(view);
    })();

    return () => {
      cancelled = true;
      if (saveTimerRef.current) {
        clearTimeout(saveTimerRef.current);
        saveTimerRef.current = null;
      }
      const pending = pendingSaveRef.current;
      pendingSaveRef.current = null;
      if (pending && !invalidPathsRef.current.has(pending.path)) {
        void saveFile(pending.doc.toString(), pending.path);
      }
      if (metricsTimerRef.current) clearTimeout(metricsTimerRef.current);
      metricsTimerRef.current = null;
      pendingMetricsRef.current = null;
      callbacksRef.current.onViewDestroy?.();
      viewRef.current?.destroy();
      viewRef.current = null;
    };
  }, [filePath, scheduleMetrics, scheduleSave, saveFile]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: readOnlyCompartmentRef.current.reconfigure([
        EditorState.readOnly.of(readOnly || barrierLockedRef.current || !pathWritableRef.current),
        EditorView.editable.of(!(readOnly || barrierLockedRef.current || !pathWritableRef.current)),
      ]),
    });
  }, [readOnly]);

  // External file change (e.g. agent wrote to disk) — reload without recreating the view.
  // We compare the incoming content against the current doc; if identical we skip the
  // dispatch so the cursor is not disturbed for no reason.
  useEffect(() => {
    if (externalRevision === undefined || externalRevision === 0) return;
    // A newly-mounted tab already loads the latest disk content above. Running
    // the revision path before CodeMirror exists would falsely settle as an
    // error and make the parent close a valid tab.
    if (!viewRef.current) return;
    let cancelled = false;
    pathWritableRef.current = false;
    if (saveTimerRef.current) {
      clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
    }
    pendingSaveRef.current = null;
    (async () => {
      let status: ExternalReloadStatus = "error";
      try {
        const res = await invokeTool("read_file", { path: filePath });
        if (cancelled || !viewRef.current) return;
        if (!res.ok) {
          const message = res.content || res.summary || `重新读取失败：${filePath}`;
          status = isMissingFileMessage(message) ? "missing" : "error";
          invalidPathsRef.current.add(filePath);
          setSaveError(message);
          return;
        }
        invalidPathsRef.current.delete(filePath);
        pathWritableRef.current = true;
        setSaveError(null);
        const newContent = res.content;
        const current = viewRef.current.state.doc.toString();
        if (newContent === current) {
          status = "loaded";
          return;
        }
        const { from } = viewRef.current.state.selection.main;
        const safeFrom = Math.min(from, newContent.length);
        applyingExternalRef.current = true;
        try {
          viewRef.current.dispatch({
            changes: { from: 0, to: current.length, insert: newContent },
            selection: { anchor: safeFrom },
          });
        } finally {
          applyingExternalRef.current = false;
        }
        callbacksRef.current.onSave?.(filePath, newContent);
        status = "loaded";
      } catch (reloadFailure) {
        if (!cancelled) {
          invalidPathsRef.current.add(filePath);
          setSaveError(describeError(reloadFailure));
        }
      } finally {
        if (!cancelled) {
          callbacksRef.current.onExternalReloadComplete?.(
            filePath,
            externalRevision,
            status,
          );
        }
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
        if (
          readOnlyRef.current ||
          barrierLockedRef.current ||
          !pathWritableRef.current ||
          invalidPathsRef.current.has(filePath)
        ) return;
        if (saveTimerRef.current) {
          clearTimeout(saveTimerRef.current);
          saveTimerRef.current = null;
        }
        pendingSaveRef.current = null;
        const content = viewRef.current?.state.doc.toString() ?? "";
        void saveFile(content, filePath);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [filePath, saveFile]);

  return (
    <div className="mde">
      {loading && (
        <div className="mde__loading" role="status">
          <Spinner size={20} />
          <span>读取文件…</span>
        </div>
      )}
      {error && <div className="mde__error">{error}</div>}
      {saveError && <div className="mde__error">保存失败：{saveError}</div>}
      {saving && <div className="mde__saving">保存中…</div>}
      <div ref={containerRef} className="mde__cm" hidden={loading || !!error} />
    </div>
  );
}
