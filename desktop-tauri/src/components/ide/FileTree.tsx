// FileTree — workspace directory explorer for the IDE screen.
// P1: right-click on any file shows a context menu with rename / delete.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  describeError,
  invokeTool,
  isWorkspaceTargetExistsError,
  renameWorkspaceFile,
  requireToolSuccess,
} from "../../lib/core";
import {
  acquireEditorWriteBarrier,
  flushPendingEditors,
  tryAcquireWorkspaceAction,
} from "../../lib/editorPersistence";
import ConfirmModal from "../ConfirmModal";
import BatchActions from "../BatchActions";
import { IconChevron, IconTrash, IconPencil } from "../icons";
import { Spinner } from "../Spinner";
import { useToast } from "../Toast";
import { runBatch } from "../../lib/batch";

export interface FileEntry {
  name: string;
  path: string;
  isDir: boolean;
}

interface DirNode {
  path: string;
  entries: FileEntry[];
  open: boolean;
  loading: boolean;
  error: string | null;
}

interface FileTreeProps {
  onOpen: (path: string) => void;
  activeFile: string | null;
  refreshKey?: number;
  /** Called when a file is deleted or renamed so the parent can close stale tabs. */
  onFileDeleted?: (path: string) => void;
  onFileRenamed?: (oldPath: string, newPath: string) => void;
  disabled?: boolean;
}

interface CtxMenu {
  entry: FileEntry;
  x: number;
  y: number;
}

const TEXT_EXT = /\.(md|markdown|txt|text|json|yml|yaml|toml|csv)$/i;
const FILE_NAME_COLLATOR = new Intl.Collator("zh-CN", {
  numeric: true,
  sensitivity: "base",
});

interface ListDirData {
  entries?: Array<{ name: string; kind: "dir" | "file" | "other"; size: number }>;
}

function sortEntries(entries: FileEntry[]): FileEntry[] {
  return entries.sort((left, right) => {
    if (left.isDir !== right.isDir) return left.isDir ? -1 : 1;
    return FILE_NAME_COLLATOR.compare(left.name, right.name);
  });
}

async function fetchDir(path: string): Promise<FileEntry[]> {
  const res = await invokeTool<ListDirData>("list_dir", { path: path || "" });
  requireToolSuccess(res, `无法读取目录：${path || "工作区"}`);
  if (Array.isArray(res.data?.entries)) {
    return sortEntries(res.data.entries.map((entry) => ({
      name: entry.name,
      path: path ? `${path}/${entry.name}` : entry.name,
      isDir: entry.kind === "dir",
    })));
  }
  // Backward-compatible fallback for older cores that only returned text.
  return sortEntries(res.content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const isDir = line.endsWith("/");
      const name = isDir ? line.slice(0, -1) : line;
      const entryPath = path ? `${path}/${name}` : name;
      return { name, path: entryPath, isDir };
    }));
}

export default function FileTree({
  onOpen, activeFile, refreshKey, onFileDeleted, onFileRenamed, disabled = false,
}: FileTreeProps) {
  const toast = useToast();
  const [root, setRoot] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [dirs, setDirs] = useState<Record<string, DirNode>>({});
  const [ctxMenu, setCtxMenu] = useState<CtxMenu | null>(null);
  const [menuIndex, setMenuIndex] = useState(0);
  const [focusedPath, setFocusedPath] = useState<string | null>(null);
  const [refreshInternal, setRefreshInternal] = useState(0);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [manageMode, setManageMode] = useState(false);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [pendingBulk, setPendingBulk] = useState<FileEntry[] | null>(null);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ done: number; total: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuItemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const menuOriginRef = useRef<HTMLButtonElement | null>(null);
  const treeItemRefs = useRef(new Map<string, HTMLButtonElement>());
  const dirsRef = useRef(dirs);
  const rootRef = useRef(root);
  const refreshRequestRef = useRef(0);
  dirsRef.current = dirs;
  rootRef.current = root;

  const visibleEntries = useMemo(() => {
    const result: Array<{ entry: FileEntry; depth: number }> = [];
    const visit = (entries: FileEntry[], depth: number) => {
      entries.forEach((entry) => {
        if (!entry.isDir && !TEXT_EXT.test(entry.name)) return;
        result.push({ entry, depth });
        const node = entry.isDir ? dirs[entry.path] : null;
        if (node?.open) visit(node.entries, depth + 1);
      });
    };
    visit(root, 0);
    return result;
  }, [dirs, root]);

  const selectableFiles = useMemo(
    () => visibleEntries
      .map(({ entry }) => entry)
      .filter((entry) => !entry.isDir && TEXT_EXT.test(entry.name)),
    [visibleEntries],
  );
  const allSelected = selectableFiles.length > 0
    && selectableFiles.every((entry) => selectedPaths.has(entry.path));

  useEffect(() => {
    const valid = new Set(selectableFiles.map((entry) => entry.path));
    setSelectedPaths((current) => {
      const next = new Set([...current].filter((path) => valid.has(path)));
      return next.size === current.size ? current : next;
    });
  }, [selectableFiles]);

  useEffect(() => {
    if (visibleEntries.length === 0) {
      setFocusedPath(null);
      return;
    }
    if (focusedPath && visibleEntries.some(({ entry }) => entry.path === focusedPath)) return;
    const preferred = activeFile && visibleEntries.some(({ entry }) => entry.path === activeFile)
      ? activeFile
      : visibleEntries[0].entry.path;
    setFocusedPath(preferred);
  }, [activeFile, focusedPath, visibleEntries]);

  const loadDirectory = useCallback(async (path: string) => {
    setDirs((prev) => {
      const current = prev[path];
      return {
        ...prev,
        [path]: {
          path,
          entries: current?.entries ?? [],
          open: current?.open ?? true,
          loading: true,
          error: null,
        },
      };
    });
    try {
      const entries = await fetchDir(path);
      setDirs((prev) => {
        const current = prev[path];
        if (!current) return prev;
        return {
          ...prev,
          [path]: { ...current, entries, loading: false, error: null },
        };
      });
    } catch (error) {
      const message = describeError(error);
      setDirs((prev) => {
        const current = prev[path];
        if (!current) return prev;
        return {
          ...prev,
          [path]: { ...current, loading: false, error: message },
        };
      });
    }
  }, []);

  const refreshTree = useCallback(async () => {
    const request = ++refreshRequestRef.current;
    const openPaths = Object.values(dirsRef.current)
      .filter((node) => node.open)
      .map((node) => node.path);
    if (rootRef.current.length === 0) setLoading(true);

    const [rootResult, ...directoryResults] = await Promise.allSettled([
      fetchDir(""),
      ...openPaths.map((path) => fetchDir(path)),
    ]);
    if (request !== refreshRequestRef.current) return;

    if (rootResult.status === "fulfilled") {
      setRoot(rootResult.value);
      setLoadError(null);
    } else {
      setLoadError(describeError(rootResult.reason));
    }

    setDirs((prev) => {
      let next = prev;
      directoryResults.forEach((result, index) => {
        const path = openPaths[index];
        const current = prev[path];
        if (!current) return;
        if (next === prev) next = { ...prev };
        next[path] = result.status === "fulfilled"
          ? { ...current, entries: result.value, loading: false, error: null }
          : {
              ...current,
              loading: false,
              error: describeError(result.reason),
            };
      });
      return next;
    });
    setLoading(false);
  }, []);

  useEffect(() => {
    void refreshTree();
  }, [refreshTree, refreshKey, refreshInternal]);

  const closeContextMenu = useCallback((restoreFocus = false) => {
    setCtxMenu(null);
    if (restoreFocus) {
      window.requestAnimationFrame(() => menuOriginRef.current?.focus());
    }
  }, []);

  // Close context menu on outside click and focus its first command on open.
  useEffect(() => {
    if (!ctxMenu) return;
    const frame = window.requestAnimationFrame(() => menuItemRefs.current[0]?.focus());
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setCtxMenu(null);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("mousedown", handler);
    };
  }, [ctxMenu]);

  const toggleDir = useCallback(async (entry: FileEntry) => {
    const node = dirsRef.current[entry.path];
    if (!node) {
      setDirs((prev) => ({
        ...prev,
        [entry.path]: {
          path: entry.path,
          entries: [],
          open: true,
          loading: true,
          error: null,
        },
      }));
      await loadDirectory(entry.path);
      return;
    }

    const nextOpen = !node.open;
    setDirs((prev) => ({
      ...prev,
      [entry.path]: { ...prev[entry.path], open: nextOpen },
    }));
    if (nextOpen && (node.error || node.entries.length === 0) && !node.loading) {
      await loadDirectory(entry.path);
    }
  }, [loadDirectory]);

  const openCtxMenu = useCallback((e: React.MouseEvent, entry: FileEntry) => {
    e.preventDefault();
    e.stopPropagation();
    if (disabled) return;
    menuOriginRef.current = e.currentTarget as HTMLButtonElement;
    setMenuIndex(0);
    setCtxMenu({
      entry,
      x: Math.min(e.clientX, Math.max(8, window.innerWidth - 176)),
      y: Math.min(e.clientY, Math.max(8, window.innerHeight - 88)),
    });
  }, [disabled]);

  const openCtxMenuFromKeyboard = useCallback((
    event: React.KeyboardEvent<HTMLButtonElement>,
    entry: FileEntry,
  ) => {
    if (
      disabled ||
      (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10"))
    ) return;
    event.preventDefault();
    event.stopPropagation();
    const rect = event.currentTarget.getBoundingClientRect();
    menuOriginRef.current = event.currentTarget;
    setMenuIndex(0);
    setCtxMenu({
      entry,
      x: Math.min(rect.left + 22, Math.max(8, window.innerWidth - 176)),
      y: Math.min(rect.top + rect.height, Math.max(8, window.innerHeight - 88)),
    });
  }, [disabled]);

  const focusTreePath = useCallback((path: string) => {
    setFocusedPath(path);
    window.requestAnimationFrame(() => treeItemRefs.current.get(path)?.focus());
  }, []);

  const toggleSelected = useCallback((path: string) => {
    setSelectedPaths((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const toggleAll = useCallback(() => {
    setSelectedPaths(allSelected
      ? new Set()
      : new Set(selectableFiles.map((entry) => entry.path)));
  }, [allSelected, selectableFiles]);

  const handleTreeKeyDown = useCallback((
    event: React.KeyboardEvent<HTMLButtonElement>,
    entry: FileEntry,
  ) => {
    if (!entry.isDir) {
      openCtxMenuFromKeyboard(event, entry);
      if (event.defaultPrevented) return;
    }

    const index = visibleEntries.findIndex(({ entry: item }) => item.path === entry.path);
    if (index < 0) return;
    let targetPath: string | null = null;
    if (event.key === "ArrowDown") {
      targetPath = visibleEntries[Math.min(index + 1, visibleEntries.length - 1)].entry.path;
    } else if (event.key === "ArrowUp") {
      targetPath = visibleEntries[Math.max(index - 1, 0)].entry.path;
    } else if (event.key === "Home") {
      targetPath = visibleEntries[0].entry.path;
    } else if (event.key === "End") {
      targetPath = visibleEntries[visibleEntries.length - 1].entry.path;
    } else if (event.key === "ArrowRight" && entry.isDir) {
      const node = dirsRef.current[entry.path];
      if (!node?.open) {
        event.preventDefault();
        void toggleDir(entry);
        return;
      }
      const child = visibleEntries[index + 1];
      if (child?.entry.path.startsWith(`${entry.path}/`)) targetPath = child.entry.path;
    } else if (event.key === "ArrowLeft") {
      const node = entry.isDir ? dirsRef.current[entry.path] : null;
      if (node?.open) {
        event.preventDefault();
        void toggleDir(entry);
        return;
      }
      const parentPath = entry.path.includes("/")
        ? entry.path.slice(0, entry.path.lastIndexOf("/"))
        : null;
      if (parentPath && visibleEntries.some(({ entry: item }) => item.path === parentPath)) {
        targetPath = parentPath;
      }
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (entry.isDir) void toggleDir(entry);
      else if (manageMode) toggleSelected(entry.path);
      else onOpen(entry.path);
      return;
    } else {
      return;
    }

    if (targetPath) {
      event.preventDefault();
      focusTreePath(targetPath);
    }
  }, [focusTreePath, manageMode, onOpen, openCtxMenuFromKeyboard, toggleDir, toggleSelected, visibleEntries]);

  useEffect(() => {
    if (disabled) closeContextMenu(false);
  }, [disabled, closeContextMenu]);

  const handleRename = useCallback(async () => {
    if (!ctxMenu || disabled) return;
    const { entry } = ctxMenu;
    closeContextMenu(true);
    const newName = window.prompt("重命名为：", entry.name);
    if (!newName?.trim() || newName.trim() === entry.name) return;
    const dir = entry.path.includes("/") ? entry.path.slice(0, entry.path.lastIndexOf("/")) : "";
    const newPath = dir ? `${dir}/${newName.trim()}` : newName.trim();
    const releaseWorkspaceAction = tryAcquireWorkspaceAction();
    if (!releaseWorkspaceAction) {
      toast.info("运笔或其他文件操作完成前无法重命名");
      return;
    }
    const releaseEditorWriteBarrier = acquireEditorWriteBarrier();
    try {
      if (!(await flushPendingEditors())) {
        toast.err("文件保存失败，已取消重命名");
        return;
      }
      try {
        await renameWorkspaceFile(entry.path, newPath, false);
      } catch (error) {
        if (!isWorkspaceTargetExistsError(error)) throw error;
        if (!window.confirm(`「${newPath}」已存在，确定覆盖吗？`)) return;
        await renameWorkspaceFile(entry.path, newPath, true);
      }
      onFileRenamed?.(entry.path, newPath);
      setRefreshInternal((k) => k + 1);
      toast.ok(`已重命名为 ${newName.trim()}`);
    } catch (error) {
      toast.err(`重命名失败：${describeError(error)}`);
    } finally {
      releaseWorkspaceAction();
      window.setTimeout(releaseEditorWriteBarrier, 0);
    }
  }, [ctxMenu, disabled, activeFile, onFileRenamed, toast, closeContextMenu]);

  const handleDelete = useCallback(async () => {
    if (!ctxMenu || disabled) return;
    const { entry } = ctxMenu;
    closeContextMenu(true);
    if (!window.confirm(`确认删除「${entry.name}」？此操作不可撤销。`)) return;
    const releaseWorkspaceAction = tryAcquireWorkspaceAction();
    if (!releaseWorkspaceAction) {
      toast.info("运笔或其他文件操作完成前无法删除");
      return;
    }
    const releaseEditorWriteBarrier = acquireEditorWriteBarrier();
    try {
      if (entry.path === activeFile && !(await flushPendingEditors())) {
        toast.err("当前文件保存失败，已取消删除");
        return;
      }
      requireToolSuccess(
        await invokeTool("delete_file", { path: entry.path }),
        "删除文件失败",
      );
      onFileDeleted?.(entry.path);
      setRefreshInternal((k) => k + 1);
      toast.ok(`已删除 ${entry.name}`);
    } catch (error) {
      toast.err(`删除失败：${describeError(error)}`);
    } finally {
      releaseWorkspaceAction();
      window.setTimeout(releaseEditorWriteBarrier, 0);
    }
  }, [ctxMenu, disabled, activeFile, onFileDeleted, toast, closeContextMenu]);

  const confirmBulkDelete = useCallback(async () => {
    if (!pendingBulk) return;
    const releaseWorkspaceAction = tryAcquireWorkspaceAction();
    if (!releaseWorkspaceAction) {
      toast.info("运笔或其他文件操作完成前无法批量删除");
      return;
    }
    const releaseEditorWriteBarrier = acquireEditorWriteBarrier();
    setBulkDeleting(true);
    setBulkProgress({ done: 0, total: pendingBulk.length });
    try {
      if (!(await flushPendingEditors())) {
        toast.err("文件保存失败，已取消批量删除");
        return;
      }
      const result = await runBatch(
        pendingBulk,
        async (entry) => {
          requireToolSuccess(
            await invokeTool("delete_file", { path: entry.path }),
            `删除文件失败：${entry.name}`,
          );
        },
        (done, total) => setBulkProgress({ done, total }),
      );
      result.completed.forEach((entry) => onFileDeleted?.(entry.path));
      setSelectedPaths((current) => new Set(
        [...current].filter((path) => !result.completed.some((entry) => entry.path === path)),
      ));
      setRefreshInternal((k) => k + 1);
      if (result.failed.length === 0) {
        toast.ok(`已删除 ${result.completed.length} 个文件`);
      } else {
        toast.err(`已删除 ${result.completed.length} 个，${result.failed.length} 个删除失败`);
      }
      setPendingBulk(null);
    } finally {
      setBulkDeleting(false);
      setBulkProgress(null);
      releaseWorkspaceAction();
      window.setTimeout(releaseEditorWriteBarrier, 0);
    }
  }, [flushPendingEditors, onFileDeleted, pendingBulk, toast]);

  if (loading) {
    return <div className="filetree__loading"><Spinner size={16} /></div>;
  }

  if (loadError && root.length === 0) {
    return (
      <div className="filetree__empty">
        <span>{loadError}</span>
        <button type="button" onClick={() => { void refreshTree(); }}>重试</button>
      </div>
    );
  }

  if (root.length === 0) {
    return <div className="filetree__empty">工作区暂无文件</div>;
  }

  function renderEntries(entries: FileEntry[], depth: number): React.ReactNode {
    return entries.map((entry) => {
      if (entry.isDir) {
        const node = dirs[entry.path];
        const isOpen = node?.open ?? false;
        return (
          <div key={entry.path} className="filetree__dir-group">
            <button
              ref={(node) => {
                if (node) treeItemRefs.current.set(entry.path, node);
                else treeItemRefs.current.delete(entry.path);
              }}
              className={`filetree__item filetree__item--dir${isOpen ? " filetree__item--open" : ""}`}
              style={{ paddingLeft: `${12 + depth * 14}px` }}
              onClick={() => { void toggleDir(entry); }}
              role="treeitem"
              tabIndex={entry.path === (focusedPath ?? visibleEntries[0]?.entry.path) ? 0 : -1}
              aria-level={depth + 1}
              aria-expanded={isOpen}
              onFocus={() => setFocusedPath(entry.path)}
              onKeyDown={(event) => handleTreeKeyDown(event, entry)}
              disabled={disabled}
            >
              <IconChevron size={12} className={`filetree__chevron${isOpen ? " filetree__chevron--down" : ""}`} />
              <span className="filetree__name">{entry.name}</span>
            </button>
            {isOpen && (
              <div className="filetree__children" role="group">
                {node?.loading && (
                  <div className="filetree__subloading" style={{ paddingLeft: `${24 + depth * 14}px` }}>
                    <Spinner size={12} />
                  </div>
                )}
                {node?.error && (
                  <button
                    type="button"
                    className="filetree__item filetree__item--file"
                    style={{ paddingLeft: `${26 + (depth + 1) * 14}px` }}
                    onClick={(event) => {
                      event.stopPropagation();
                      void loadDirectory(entry.path);
                    }}
                    title={node.error}
                    disabled={disabled || node.loading}
                  >
                    <span className="filetree__dot" />
                    <span className="filetree__name">读取失败，点击重试</span>
                  </button>
                )}
                {renderEntries(node?.entries ?? [], depth + 1)}
              </div>
            )}
          </div>
        );
      }

      if (!TEXT_EXT.test(entry.name)) return null;

      const selected = selectedPaths.has(entry.path);
      return (
        <div className={`filetree__file-row${selected ? " is-selected" : ""}`} key={entry.path}>
          {manageMode && (
            <input
              type="checkbox"
              className="filetree__select"
              checked={selected}
              onChange={() => toggleSelected(entry.path)}
              onClick={(event) => event.stopPropagation()}
              aria-label={`选择文件：${entry.name}`}
              tabIndex={-1}
              disabled={disabled || bulkDeleting}
            />
          )}
          <button
            ref={(node) => {
              if (node) treeItemRefs.current.set(entry.path, node);
              else treeItemRefs.current.delete(entry.path);
            }}
            className={`filetree__item filetree__item--file${activeFile === entry.path ? " filetree__item--active" : ""}${selected ? " filetree__item--selected" : ""}`}
            style={{ paddingLeft: manageMode ? `${4 + depth * 14}px` : `${26 + depth * 14}px` }}
            onClick={() => (manageMode ? toggleSelected(entry.path) : onOpen(entry.path))}
            onContextMenu={(e) => openCtxMenu(e, entry)}
            onKeyDown={(event) => handleTreeKeyDown(event, entry)}
            role="treeitem"
            tabIndex={entry.path === (focusedPath ?? visibleEntries[0]?.entry.path) ? 0 : -1}
            aria-level={depth + 1}
            aria-selected={manageMode ? selected : activeFile === entry.path}
            aria-pressed={manageMode ? selected : undefined}
            onFocus={() => setFocusedPath(entry.path)}
            title={entry.path}
            disabled={disabled}
          >
            <span className="filetree__dot" />
            <span className="filetree__name">{entry.name}</span>
          </button>
        </div>
      );
    });
  }

  return (
    <>
      {loadError && (
        <div className="filetree__empty">
          <span>刷新失败，当前显示上次结果</span>
          <button type="button" onClick={() => { void refreshTree(); }} title={loadError}>重试</button>
        </div>
      )}
      {selectableFiles.length > 1 && (
        <div className="filetree__batch-toolbar">
          <button
            type="button"
            className={`btn btn--ghost btn--xs${manageMode ? " is-active" : ""}`}
            onClick={() => {
              setManageMode((value) => !value);
              setSelectedPaths(new Set());
            }}
            aria-pressed={manageMode}
          >
            {manageMode ? "完成" : "批量管理"}
          </button>
          {manageMode && (
            <BatchActions
              selectedCount={selectedPaths.size}
              totalCount={selectableFiles.length}
              allSelected={allSelected}
              onToggleAll={toggleAll}
              onClear={() => setSelectedPaths(new Set())}
              label="批量管理工作区文件"
            >
              <button
                type="button"
                className="btn btn--danger btn--xs"
                disabled={selectedPaths.size === 0 || bulkDeleting || disabled}
                onClick={() => setPendingBulk(selectableFiles.filter((entry) => selectedPaths.has(entry.path)))}
              >
                <IconTrash size={12} /> 删除已选
              </button>
            </BatchActions>
          )}
        </div>
      )}
      <div className={`filetree${manageMode ? " is-managing" : ""}`} role="tree" aria-label="工作区文件">
        {renderEntries(root, 0)}
      </div>

      {/* 右键上下文菜单 */}
      {ctxMenu && (
        <div
          ref={menuRef}
          className="filetree__ctx-menu"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          role="menu"
          aria-label={`「${ctxMenu.entry.name}」操作`}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              closeContextMenu(true);
              return;
            }
            if (event.key === "Tab") {
              window.setTimeout(() => closeContextMenu(false), 0);
              return;
            }
            let next = menuIndex;
            if (event.key === "ArrowDown") next = (menuIndex + 1) % 2;
            else if (event.key === "ArrowUp") next = (menuIndex + 1) % 2;
            else if (event.key === "Home") next = 0;
            else if (event.key === "End") next = 1;
            else return;
            event.preventDefault();
            setMenuIndex(next);
            menuItemRefs.current[next]?.focus();
          }}
        >
          <button
            ref={(node) => { menuItemRefs.current[0] = node; }}
            className="filetree__ctx-item"
            onClick={handleRename}
            onFocus={() => setMenuIndex(0)}
            role="menuitem"
            tabIndex={menuIndex === 0 ? 0 : -1}
            type="button"
          >
            <IconPencil size={13} />
            重命名
          </button>
          <button
            ref={(node) => { menuItemRefs.current[1] = node; }}
            className="filetree__ctx-item filetree__ctx-item--danger"
            onClick={handleDelete}
            onFocus={() => setMenuIndex(1)}
            role="menuitem"
            tabIndex={menuIndex === 1 ? 0 : -1}
            type="button"
          >
            <IconTrash size={13} />
            删除文件
          </button>
        </div>
      )}

      <ConfirmModal
        open={pendingBulk !== null}
        title={`删除选中的 ${pendingBulk?.length ?? 0} 个文件？`}
        sealChar="删"
        danger
        busy={bulkDeleting}
        confirmLabel="删除"
        body={
          <>
            将永久删除选中的工作区文件。
            {bulkProgress && <><br />正在处理：{bulkProgress.done} / {bulkProgress.total}</>}
            <br />此操作不可撤销。
          </>
        }
        onConfirm={() => void confirmBulkDelete()}
        onCancel={() => {
          if (!bulkDeleting) setPendingBulk(null);
        }}
      />
    </>
  );
}
