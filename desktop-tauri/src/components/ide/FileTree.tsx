// FileTree — workspace directory explorer for the IDE screen.
// P1: right-click on any file shows a context menu with rename / delete.

import { useCallback, useEffect, useRef, useState } from "react";
import { invokeTool } from "../../lib/core";
import { IconChevron, IconTrash, IconPencil } from "../icons";
import { Spinner } from "../Spinner";
import { useToast } from "../Toast";

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
}

interface FileTreeProps {
  onOpen: (path: string) => void;
  activeFile: string | null;
  refreshKey?: number;
  /** Called when a file is deleted or renamed so the parent can close stale tabs. */
  onFileDeleted?: (path: string) => void;
  onFileRenamed?: (oldPath: string, newPath: string) => void;
}

interface CtxMenu {
  entry: FileEntry;
  x: number;
  y: number;
}

const TEXT_EXT = /\.(md|markdown|txt|text|json|yml|yaml|toml|csv)$/i;

async function fetchDir(path: string): Promise<FileEntry[]> {
  const res = await invokeTool<{ entries: string }>("list_dir", { path: path || "" });
  if (!res.ok) return [];
  const raw: string = res.data?.entries ?? "";
  return raw
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const isDir = line.endsWith("/");
      const name = isDir ? line.slice(0, -1) : line;
      const entryPath = path ? `${path}/${name}` : name;
      return { name, path: entryPath, isDir };
    });
}

export default function FileTree({
  onOpen, activeFile, refreshKey, onFileDeleted, onFileRenamed,
}: FileTreeProps) {
  const toast = useToast();
  const [root, setRoot] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [dirs, setDirs] = useState<Record<string, DirNode>>({});
  const [ctxMenu, setCtxMenu] = useState<CtxMenu | null>(null);
  const [refreshInternal, setRefreshInternal] = useState(0);
  const menuRef = useRef<HTMLDivElement>(null);

  const loadRoot = useCallback(async () => {
    setLoading(true);
    try {
      const entries = await fetchDir("");
      setRoot(entries);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadRoot();
  }, [loadRoot, refreshKey, refreshInternal]);

  // Close context menu on outside click
  useEffect(() => {
    if (!ctxMenu) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setCtxMenu(null);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [ctxMenu]);

  const toggleDir = useCallback(async (entry: FileEntry) => {
    setDirs((prev) => {
      const node = prev[entry.path];
      if (node) {
        return { ...prev, [entry.path]: { ...node, open: !node.open } };
      }
      return {
        ...prev,
        [entry.path]: { path: entry.path, entries: [], open: true, loading: true },
      };
    });
    setDirs((prev) => {
      const node = prev[entry.path];
      if (node && node.entries.length === 0 && node.loading) {
        fetchDir(entry.path).then((entries) => {
          setDirs((p) => ({
            ...p,
            [entry.path]: { ...p[entry.path], entries, loading: false },
          }));
        });
      }
      return prev;
    });
  }, []);

  const openCtxMenu = useCallback((e: React.MouseEvent, entry: FileEntry) => {
    e.preventDefault();
    e.stopPropagation();
    setCtxMenu({ entry, x: e.clientX, y: e.clientY });
  }, []);

  const handleRename = useCallback(async () => {
    if (!ctxMenu) return;
    const { entry } = ctxMenu;
    setCtxMenu(null);
    const newName = window.prompt("重命名为：", entry.name);
    if (!newName?.trim() || newName.trim() === entry.name) return;
    const dir = entry.path.includes("/") ? entry.path.slice(0, entry.path.lastIndexOf("/")) : "";
    const newPath = dir ? `${dir}/${newName.trim()}` : newName.trim();
    try {
      // Read → Write new → Delete old
      const res = await invokeTool<{ content: string }>("read_file", { path: entry.path });
      const content = res.ok ? (res.data?.content ?? "") : "";
      await invokeTool("write_file", { path: newPath, content });
      await invokeTool("delete_file", { path: entry.path });
      onFileRenamed?.(entry.path, newPath);
      setRefreshInternal((k) => k + 1);
      toast.ok(`已重命名为 ${newName.trim()}`);
    } catch {
      toast.err("重命名失败");
    }
  }, [ctxMenu, onFileRenamed, toast]);

  const handleDelete = useCallback(async () => {
    if (!ctxMenu) return;
    const { entry } = ctxMenu;
    setCtxMenu(null);
    if (!window.confirm(`确认删除「${entry.name}」？此操作不可撤销。`)) return;
    try {
      await invokeTool("delete_file", { path: entry.path });
      onFileDeleted?.(entry.path);
      setRefreshInternal((k) => k + 1);
      toast.ok(`已删除 ${entry.name}`);
    } catch {
      toast.err("删除失败");
    }
  }, [ctxMenu, onFileDeleted, toast]);

  if (loading) {
    return <div className="filetree__loading"><Spinner size={16} /></div>;
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
              className={`filetree__item filetree__item--dir${isOpen ? " filetree__item--open" : ""}`}
              style={{ paddingLeft: `${12 + depth * 14}px` }}
              onClick={() => toggleDir(entry)}
            >
              <IconChevron size={12} className={`filetree__chevron${isOpen ? " filetree__chevron--down" : ""}`} />
              <span className="filetree__name">{entry.name}</span>
            </button>
            {isOpen && (
              <div className="filetree__children">
                {node?.loading ? (
                  <div className="filetree__subloading" style={{ paddingLeft: `${24 + depth * 14}px` }}>
                    <Spinner size={12} />
                  </div>
                ) : (
                  renderEntries(node?.entries ?? [], depth + 1)
                )}
              </div>
            )}
          </div>
        );
      }

      if (!TEXT_EXT.test(entry.name)) return null;

      return (
        <button
          key={entry.path}
          className={`filetree__item filetree__item--file${activeFile === entry.path ? " filetree__item--active" : ""}`}
          style={{ paddingLeft: `${26 + depth * 14}px` }}
          onClick={() => onOpen(entry.path)}
          onContextMenu={(e) => openCtxMenu(e, entry)}
          title={entry.path}
        >
          <span className="filetree__dot" />
          <span className="filetree__name">{entry.name}</span>
        </button>
      );
    });
  }

  return (
    <>
      <div className="filetree">{renderEntries(root, 0)}</div>

      {/* 右键上下文菜单 */}
      {ctxMenu && (
        <div
          ref={menuRef}
          className="filetree__ctx-menu"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          role="menu"
        >
          <button className="filetree__ctx-item" onClick={handleRename} role="menuitem">
            <IconPencil size={13} />
            重命名
          </button>
          <button className="filetree__ctx-item filetree__ctx-item--danger" onClick={handleDelete} role="menuitem">
            <IconTrash size={13} />
            删除文件
          </button>
        </div>
      )}
    </>
  );
}
