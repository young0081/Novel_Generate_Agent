"use client";

import { type CSSProperties, useCallback, useEffect, useRef, useState } from "react";

import BatchActions from "@/components/BatchActions";
import Button from "@/components/Button";
import { useConnection } from "@/components/Connection";
import EmptyState from "@/components/EmptyState";
import { useToast } from "@/components/Toast";
import { runBatch } from "@/lib/batch";
import { invokeTool } from "@/lib/rpcClient";

type Job = "list" | "open" | "save" | null;

interface ListedEntry {
  name: string;
  path: string;
  kind: "dir" | "file" | "other";
  size: number;
}

interface ListDirData {
  entries?: Array<{ name?: string; kind?: string; size?: number }>;
}

function parseEntries(content: string, base: string): ListedEntry[] {
  return content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const kind: ListedEntry["kind"] = line.endsWith("/") ? "dir" : "file";
      const name = line.endsWith("/") ? line.slice(0, -1) : line;
      return { name, path: base ? `${base}/${name}` : name, kind, size: 0 };
    });
}

function formatBytes(size: number): string {
  if (!size) return "";
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

export default function FileWorkbench() {
  const toast = useToast();
  const { reportError } = useConnection();

  const [dirPath, setDirPath] = useState("");
  const [listing, setListing] = useState("");
  const [listed, setListed] = useState(false);
  const [entries, setEntries] = useState<ListedEntry[]>([]);
  const [filePath, setFilePath] = useState("book/ch1.md");
  const [content, setContent] = useState("");
  const [job, setJob] = useState<Job>(null);
  const [manageMode, setManageMode] = useState(false);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [deletingPath, setDeletingPath] = useState<string | null>(null);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ done: number; total: number } | null>(null);

  const fail = useCallback(
    (e: unknown) => {
      const message = e instanceof Error ? e.message : String(e);
      reportError(message);
      toast.error("错误：" + message);
    },
    [reportError, toast],
  );

  const doList = useCallback(async () => {
    setJob("list");
    try {
      const r = await invokeTool<ListDirData>("list_dir", { path: dirPath });
      if (!r.ok) {
        setListing("");
        setEntries([]);
        setListed(false);
        toast.error(r.content);
        return;
      }
      const dataEntries = Array.isArray(r.data?.entries)
        ? r.data.entries
            .filter((entry) => typeof entry.name === "string")
            .map((entry) => {
              const kind: ListedEntry["kind"] =
                entry.kind === "dir" || entry.kind === "other" ? entry.kind : "file";
              const name = entry.name as string;
              return {
                name,
                path: dirPath ? `${dirPath.replace(/\/$/, "")}/${name}` : name,
                kind,
                size: typeof entry.size === "number" ? entry.size : 0,
              };
            })
        : parseEntries(r.content || "", dirPath);
      setEntries(dataEntries);
      setListing(r.content || "");
      setListed(true);
    } catch (e) {
      fail(e);
    } finally {
      setJob(null);
    }
  }, [dirPath, fail, toast]);

  const doOpen = useCallback(async () => {
    setJob("open");
    try {
      const r = await invokeTool("read_file", { path: filePath });
      if (!r.ok) {
        toast.error(r.content);
      } else {
        setContent(r.content);
        toast.success(`已打开 ${filePath}（${r.metadata.bytes} 字节）`);
      }
    } catch (e) {
      fail(e);
    } finally {
      setJob(null);
    }
  }, [filePath, fail, toast]);

  const doSave = useCallback(async () => {
    setJob("save");
    try {
      const r = await invokeTool("write_file", { path: filePath, content });
      if (r.ok) {
        toast.success(`已保存：${r.summary ?? r.content}`);
      } else {
        toast.error(r.content);
      }
    } catch (e) {
      fail(e);
    } finally {
      setJob(null);
    }
  }, [filePath, content, fail, toast]);

  const removeFromList = useCallback((paths: Set<string>) => {
    setEntries((current) => current.filter((entry) => !paths.has(entry.path)));
    setSelectedPaths((current) => new Set([...current].filter((path) => !paths.has(path))));
    if (paths.has(filePath)) {
      setContent("");
      setFilePath("");
    }
  }, [filePath]);

  const deleteOne = useCallback(
    async (entry: ListedEntry) => {
      if (entry.kind !== "file" || bulkDeleting || !window.confirm(`确定删除文件“${entry.path}”吗？`)) return;
      setDeletingPath(entry.path);
      try {
        const r = await invokeTool("delete_file", { path: entry.path });
        if (!r.ok) {
          toast.error(r.content || "删除失败");
          return;
        }
        removeFromList(new Set([entry.path]));
        toast.success(`已删除：${entry.path}`);
      } catch (e) {
        fail(e);
      } finally {
        setDeletingPath(null);
      }
    },
    [bulkDeleting, fail, removeFromList, toast],
  );

  const toggleSelected = useCallback((path: string) => {
    setSelectedPaths((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const files = entries.filter((entry) => entry.kind === "file");
  const allSelected = files.length > 0 && files.every((entry) => selectedPaths.has(entry.path));

  const toggleAll = useCallback(() => {
    setSelectedPaths(allSelected ? new Set() : new Set(files.map((entry) => entry.path)));
  }, [allSelected, files]);

  const deleteSelected = useCallback(async () => {
    const selected = files.filter((entry) => selectedPaths.has(entry.path));
    if (selected.length === 0 || bulkDeleting || deletingPath !== null) return;
    if (!window.confirm(`确定删除选中的 ${selected.length} 个文件吗？此操作不可撤销。`)) return;
    setBulkDeleting(true);
    setBulkProgress({ done: 0, total: selected.length });
    try {
      const result = await runBatch(
        selected,
        async (entry) => {
          const r = await invokeTool("delete_file", { path: entry.path });
          if (!r.ok) throw new Error(r.content || `删除失败：${entry.path}`);
        },
        (done, total) => setBulkProgress({ done, total }),
      );
      const completedPaths = new Set(result.completed.map((entry) => entry.path));
      removeFromList(completedPaths);
      if (result.failed.length === 0) {
        toast.success(`已删除 ${result.completed.length} 个文件`);
      } else {
        toast.error(`已删除 ${result.completed.length} 个，${result.failed.length} 个失败`);
      }
    } finally {
      setBulkDeleting(false);
      setBulkProgress(null);
    }
  }, [bulkDeleting, deletingPath, files, removeFromList, selectedPaths, toast]);

  // Keep a live ref so the keydown handler always saves the latest content.
  const saveRef = useRef(doSave);
  saveRef.current = doSave;
  const busyRef = useRef<Job>(job);
  busyRef.current = job;

  // Ctrl/Cmd+S saves the open file.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (busyRef.current === null && filePath) void saveRef.current();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [filePath]);

  const busy = job !== null || bulkDeleting || deletingPath !== null;

  return (
    <div>
      <div className="card" style={{ "--i": 0 } as CSSProperties}>
        <div className="card-head">
          <h3>目录浏览</h3>
          <div className="row">
            {files.length > 0 && (
              <Button
                variant="ghost"
                onClick={() => {
                  setManageMode((value) => !value);
                  setSelectedPaths(new Set());
                }}
                disabled={busy}
              >
                {manageMode ? "完成" : "批量管理"}
              </Button>
            )}
            <Button variant="ghost" onClick={doList} loading={job === "list"} disabled={busy}>
              刷新
            </Button>
          </div>
        </div>
        <div className="row">
          <input
            className="grow"
            placeholder="目录路径（留空=工作区根目录），例如 book"
            value={dirPath}
            onChange={(e) => setDirPath(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !busy && void doList()}
          />
          <Button onClick={doList} loading={job === "list"} disabled={busy}>
            列目录
          </Button>
        </div>
        {manageMode && files.length > 0 && (
          <BatchActions
            selectedCount={selectedPaths.size}
            totalCount={files.length}
            allSelected={allSelected}
            busy={bulkDeleting || deletingPath !== null}
            progress={bulkProgress}
            onToggleAll={toggleAll}
            onClear={() => setSelectedPaths(new Set())}
            onDelete={() => void deleteSelected()}
            label="批量管理工作区文件"
          />
        )}
        {listed &&
          (entries.length > 0 ? (
            <div className="batch-list">
              {entries.map((entry) => (
                <div className={`batch-row${selectedPaths.has(entry.path) ? " is-selected" : ""}`} key={entry.path}>
                  {manageMode && entry.kind === "file" && (
                    <input
                      type="checkbox"
                      className="batch-select"
                      checked={selectedPaths.has(entry.path)}
                      onChange={() => toggleSelected(entry.path)}
                      aria-label={`选择文件：${entry.path}`}
                      disabled={busy}
                    />
                  )}
                  <div className="batch-row__main">
                    <div className="batch-row__title">
                      <strong>{entry.kind === "dir" ? "📁 " : "📄 "}{entry.name}{entry.kind === "dir" ? "/" : ""}</strong>
                      {entry.size > 0 && <span className="badge">{formatBytes(entry.size)}</span>}
                    </div>
                    <div className="batch-row__meta">{entry.path}</div>
                  </div>
                  {entry.kind === "file" && (
                    <button
                      type="button"
                      className="batch-row__delete"
                      onClick={() => void deleteOne(entry)}
                      disabled={busy}
                      aria-label={`删除文件：${entry.path}`}
                    >
                      {deletingPath === entry.path ? "删除中…" : "删除"}
                    </button>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <EmptyState icon="📂" title="这个目录是空的" hint="换个路径或先去保存一篇稿件。" />
          ))}
        {!listed && listing && <pre className="output">{listing}</pre>}
      </div>

      <div className="card" style={{ "--i": 1 } as CSSProperties}>
        <div className="card-head">
          <h3>编辑文件</h3>
          <span className="hint">提示：编辑时按 Ctrl/Cmd + S 可快速保存</span>
        </div>
        <div className="row">
          <input
            className="grow"
            placeholder="文件路径，例如 book/ch1.md"
            value={filePath}
            onChange={(e) => setFilePath(e.target.value)}
          />
          <Button variant="ghost" onClick={doOpen} loading={job === "open"} disabled={busy || !filePath}>
            打开
          </Button>
          <Button onClick={doSave} loading={job === "save"} disabled={busy || !filePath}>
            保存
          </Button>
        </div>
        <textarea
          style={{ marginTop: 12, minHeight: 280 }}
          placeholder="文件内容…"
          value={content}
          onChange={(e) => setContent(e.target.value)}
        />
      </div>
    </div>
  );
}
