// FileTree — workspace directory explorer for the IDE screen.
// Lists files and directories recursively; emits onOpen when a file is clicked.

import { useCallback, useEffect, useState } from "react";
import { invokeTool } from "../../lib/core";
import { IconChevron } from "../icons";
import { Spinner } from "../Spinner";

export interface FileEntry {
  name: string;
  path: string; // relative to workspace root
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
  refreshKey?: number; // increment to force re-fetch root
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

export default function FileTree({ onOpen, activeFile, refreshKey }: FileTreeProps) {
  const [root, setRoot] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [dirs, setDirs] = useState<Record<string, DirNode>>({});

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
  }, [loadRoot, refreshKey]);

  const toggleDir = useCallback(async (entry: FileEntry) => {
    setDirs((prev) => {
      const node = prev[entry.path];
      if (node) {
        return { ...prev, [entry.path]: { ...node, open: !node.open } };
      }
      // First open: start loading
      return {
        ...prev,
        [entry.path]: { path: entry.path, entries: [], open: true, loading: true },
      };
    });

    // Fetch if not already loaded
    setDirs((prev) => {
      const node = prev[entry.path];
      if (node && node.entries.length === 0 && node.loading) {
        // Async fetch outside of setter
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

  if (loading) {
    return (
      <div className="filetree__loading">
        <Spinner size={16} />
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

      // File — only show text files
      if (!TEXT_EXT.test(entry.name)) return null;

      return (
        <button
          key={entry.path}
          className={`filetree__item filetree__item--file${activeFile === entry.path ? " filetree__item--active" : ""}`}
          style={{ paddingLeft: `${26 + depth * 14}px` }}
          onClick={() => onOpen(entry.path)}
          title={entry.path}
        >
          <span className="filetree__dot" />
          <span className="filetree__name">{entry.name}</span>
        </button>
      );
    });
  }

  return <div className="filetree">{renderEntries(root, 0)}</div>;
}
