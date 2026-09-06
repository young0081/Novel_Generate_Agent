// 修订 — Chapter editor (new desk-workflow layout).
// Browse book/ + root, open a file into a serif writing surface, edit, save,
// create and delete chapters. Wires the real list_dir / read_file / write_file
// / delete_file tools.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { LoadingBlock, Spinner } from "../components/Spinner";
import ConfirmModal from "../components/ConfirmModal";
import BatchActions from "../components/BatchActions";
import {
  IconPlus,
  IconSave,
  IconRefresh,
  IconFile,
  IconFolder,
  IconScroll,
  IconTrash,
  IconSearch,
  IconClose,
} from "../components/icons";
import {
  createWorkspaceFile,
  describeError,
  invokeTool,
  isWorkspaceTargetExistsError,
} from "../lib/core";
import { useToast } from "../components/Toast";
import {
  acquireEditorWriteBarrier,
  flushPendingEditors,
  registerEditorFlush,
  tryAcquireWorkspaceAction,
} from "../lib/editorPersistence";
import {
  createRevisionDocumentSafely,
  createRevisionSaveQueue,
  revisionReadOnlyReason,
  type RevisionSaveQueue,
  type RevisionSaveSnapshot,
} from "../lib/revisionSaveQueue";
import { runBatch } from "../lib/batch";

interface DirEntry {
  name: string;
  path: string;
  isDir: boolean;
}

interface FileGroup {
  label: string;
  base: string;
  entries: DirEntry[];
}

const TEXT_EXT = /\.(md|markdown|txt|text|json|yml|yaml|toml|csv|html?|xml)$/i;

function isLikelyTextFile(name: string): boolean {
  return TEXT_EXT.test(name) || !name.includes(".");
}

function parseListing(content: string, base: string): DirEntry[] {
  return content
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0 && l !== "." && l !== "..")
    .map((raw) => {
      const isDir = raw.endsWith("/");
      const name = isDir ? raw.slice(0, -1) : raw;
      const path = base ? `${base}/${name}` : name;
      return { name, path, isDir };
    });
}

function countChars(text: string): number {
  return text.replace(/\s/g, "").length;
}

export default function RevisionWork() {
  const toast = useToast();
  const [groups, setGroups] = useState<FileGroup[]>([]);
  const [loadingList, setLoadingList] = useState(true);
  const [listError, setListError] = useState<string | null>(null);

  const [activePath, setActivePath] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [savedContent, setSavedContent] = useState("");
  const [loadingDoc, setLoadingDoc] = useState(false);
  const [saving, setSaving] = useState(false);
  const [writeLocked, setWriteLocked] = useState(false);
  const [readOnlyReason, setReadOnlyReason] = useState<string | null>(null);
  const [docError, setDocError] = useState<string | null>(null);
  const [delOpen, setDelOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [manageMode, setManageMode] = useState(false);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [pendingBulk, setPendingBulk] = useState<DirEntry[] | null>(null);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ done: number; total: number } | null>(null);

  const surfaceRef = useRef<HTMLTextAreaElement>(null);
  const mountedRef = useRef(true);
  const activePathRef = useRef(activePath);
  const contentRef = useRef(content);
  const savedContentRef = useRef(savedContent);
  const writeLockedRef = useRef(false);
  const documentWritableRef = useRef(false);
  const saveCountRef = useRef(0);
  const lastSaveErrorRef = useRef<string | null>(null);
  activePathRef.current = activePath;
  contentRef.current = content;
  savedContentRef.current = savedContent;
  const dirty = content !== savedContent;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const persistRevision = useCallback(async (path: string, draft: string): Promise<boolean> => {
    saveCountRef.current += 1;
    if (mountedRef.current) setSaving(true);
    try {
      const res = await invokeTool("write_file", { path, content: draft });
      if (!res.ok) {
        lastSaveErrorRef.current = res.content || `保存失败：${path}`;
        return false;
      }
      lastSaveErrorRef.current = null;
      if (mountedRef.current && activePathRef.current === path) {
        savedContentRef.current = draft;
        setSavedContent(draft);
      }
      return true;
    } catch (error) {
      lastSaveErrorRef.current = describeError(error);
      return false;
    } finally {
      saveCountRef.current = Math.max(0, saveCountRef.current - 1);
      if (mountedRef.current) setSaving(saveCountRef.current > 0);
    }
  }, []);

  const persistRevisionRef = useRef(persistRevision);
  persistRevisionRef.current = persistRevision;
  const saveQueueRef = useRef<RevisionSaveQueue | null>(null);
  if (!saveQueueRef.current) {
    saveQueueRef.current = createRevisionSaveQueue(
      (path, draft) => persistRevisionRef.current(path, draft),
    );
  }
  const saveQueue = saveQueueRef.current;

  const readSaveSnapshot = useCallback((): RevisionSaveSnapshot => ({
    path: documentWritableRef.current ? activePathRef.current : null,
    content: contentRef.current,
    savedContent: savedContentRef.current,
  }), []);

  const flushRevision = useCallback(
    () => saveQueue.flushLatest(readSaveSnapshot),
    [readSaveSnapshot, saveQueue],
  );

  const setBarrierLocked = useCallback((locked: boolean) => {
    writeLockedRef.current = locked;
    if (surfaceRef.current) {
      surfaceRef.current.readOnly = locked || !documentWritableRef.current;
    }
    if (mountedRef.current) setWriteLocked(locked);
  }, []);

  useEffect(
    () => registerEditorFlush(flushRevision, setBarrierLocked),
    [flushRevision, setBarrierLocked],
  );

  const loadTree = useCallback(async () => {
    if (mountedRef.current) {
      setLoadingList(true);
      setListError(null);
    }
    try {
      const targets: { label: string; base: string }[] = [
        { label: "book 目录", base: "book" },
        { label: "工作区根目录", base: "" },
      ];
      const next: FileGroup[] = [];
      for (const t of targets) {
        const res = await invokeTool<unknown>("list_dir", { path: t.base });
        if (!res.ok) continue;
        const entries = parseListing(res.content, t.base).filter(
          (e) => e.isDir || isLikelyTextFile(e.name),
        );
        if (entries.length > 0 || t.base === "") {
          next.push({ label: t.label, base: t.base, entries });
        }
      }
      if (mountedRef.current) setGroups(next);
    } catch (e) {
      if (mountedRef.current) setListError(describeError(e));
    } finally {
      if (mountedRef.current) setLoadingList(false);
    }
  }, []);

  useEffect(() => {
    void loadTree();
  }, [loadTree]);

  const readDocument = useCallback(async (path: string): Promise<boolean> => {
    if (!mountedRef.current) return false;
    activePathRef.current = path;
    documentWritableRef.current = false;
    setActivePath(path);
    setLoadingDoc(true);
    setDocError(null);
    setReadOnlyReason(null);
    try {
      const res = await invokeTool<{ bytes?: number }>("read_file", { path });
      if (!mountedRef.current || activePathRef.current !== path) return false;
      if (!res.ok) {
        const message = res.content || "无法读取该文件";
        contentRef.current = "";
        savedContentRef.current = "";
        setDocError(message);
        setContent("");
        setSavedContent("");
        return false;
      }
      const unsafeReason = revisionReadOnlyReason(res.metadata, res.data?.bytes);
      documentWritableRef.current = unsafeReason === null;
      contentRef.current = res.content;
      savedContentRef.current = res.content;
      setContent(res.content);
      setSavedContent(res.content);
      setReadOnlyReason(unsafeReason);
      if (unsafeReason) toast.info(unsafeReason);
      return true;
    } catch (error) {
      if (!mountedRef.current || activePathRef.current !== path) return false;
      contentRef.current = "";
      savedContentRef.current = "";
      documentWritableRef.current = false;
      setContent("");
      setSavedContent("");
      setDocError(describeError(error));
      return false;
    } finally {
      if (mountedRef.current && activePathRef.current === path) setLoadingDoc(false);
    }
  }, [toast]);

  const openFile = useCallback(async (path: string): Promise<boolean> => {
    if (path === activePathRef.current && documentWritableRef.current) return true;
    if (writeLockedRef.current) {
      toast.info("后台任务完成前无法切换书稿");
      return false;
    }
    const releaseWorkspaceAction = tryAcquireWorkspaceAction();
    if (!releaseWorkspaceAction) {
      toast.info("其他文件操作完成前无法切换书稿");
      return false;
    }
    const releaseWriteBarrier = acquireEditorWriteBarrier();
    try {
      if (!(await flushPendingEditors())) {
        toast.err(lastSaveErrorRef.current || "当前章节保存失败，已取消切换");
        return false;
      }
      return await readDocument(path);
    } finally {
      releaseWorkspaceAction();
      releaseWriteBarrier();
    }
  }, [readDocument, toast]);

  const save = useCallback(async (): Promise<boolean> => {
    const path = activePathRef.current;
    const draft = contentRef.current;
    if (!path || draft === savedContentRef.current) return true;
    if (!documentWritableRef.current) {
      toast.err(readOnlyReason || "当前文稿不是可安全回写的完整文本");
      return false;
    }
    if (writeLockedRef.current || saveCountRef.current > 0) return false;

    const releaseWorkspaceAction = tryAcquireWorkspaceAction();
    if (!releaseWorkspaceAction) {
      toast.info("其他文件操作完成前无法保存");
      return false;
    }
    try {
      const saved = await saveQueue.enqueue(path, draft);
      if (!mountedRef.current) return saved;
      if (saved) toast.ok("已保存");
      else toast.err(lastSaveErrorRef.current || "保存失败");
      return saved;
    } finally {
      releaseWorkspaceAction();
    }
  }, [readOnlyReason, saveQueue, toast]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (activePath && dirty && !writeLockedRef.current) void save();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activePath, dirty, save]);

  const createChapter = useCallback(async () => {
    if (writeLockedRef.current) {
      toast.info("后台任务完成前无法新建章节");
      return;
    }
    const raw = window.prompt("新建章节的文件名（默认放入 book 目录）：", "ch1.md");
    if (!raw) return;
    let name = raw.trim();
    if (!name) return;
    if (!/\.[a-z0-9]+$/i.test(name)) name += ".md";
    const path = name.includes("/") ? name : `book/${name}`;
    const releaseWorkspaceAction = tryAcquireWorkspaceAction();
    if (!releaseWorkspaceAction) {
      toast.info("其他文件操作完成前无法新建章节");
      return;
    }
    const releaseWriteBarrier = acquireEditorWriteBarrier();
    try {
      if (!(await flushPendingEditors())) {
        toast.err(lastSaveErrorRef.current || "当前章节保存失败，已取消新建");
        return;
      }
      const template = `# ${name.replace(/\.[^.]+$/, "")}\n\n`;
      const createResult = await createRevisionDocumentSafely({
        path,
        content: template,
        create: createWorkspaceFile,
        isConflict: isWorkspaceTargetExistsError,
        confirmOverwrite: () => window.confirm(`「${path}」已存在，确定要覆盖原文吗？`),
      });
      if (createResult === "cancelled") return;
      toast.ok(`${createResult === "overwritten" ? "已覆盖" : "已创建"} ${path}`);
      await loadTree();
      await readDocument(path);
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      releaseWorkspaceAction();
      releaseWriteBarrier();
    }
  }, [loadTree, readDocument, toast]);

  const deleteChapter = useCallback(async () => {
    if (!activePath) return;
    if (writeLockedRef.current) {
      toast.info("后台任务完成前无法删除章节");
      return;
    }
    const releaseWorkspaceAction = tryAcquireWorkspaceAction();
    if (!releaseWorkspaceAction) {
      toast.info("其他文件操作完成前无法删除章节");
      return;
    }
    const releaseWriteBarrier = acquireEditorWriteBarrier();
    setDeleting(true);
    try {
      await saveQueue.drain();
      const res = await invokeTool("delete_file", { path: activePath });
      if (!res.ok) {
        toast.err(res.content || "删除失败");
        return;
      }
      toast.ok(`已删除 ${activePath}`);
      setDelOpen(false);
      activePathRef.current = null;
      contentRef.current = "";
      savedContentRef.current = "";
      documentWritableRef.current = false;
      setActivePath(null);
      setContent("");
      setSavedContent("");
      setReadOnlyReason(null);
      await loadTree();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setDeleting(false);
      releaseWorkspaceAction();
      releaseWriteBarrier();
    }
  }, [activePath, loadTree, saveQueue, toast]);

  const fileGroups = useMemo(
    () => {
      const q = searchQuery.toLowerCase().trim();
      return groups
        .map((g) => ({
          ...g,
          files: g.entries
            .filter((e) => !e.isDir)
            .filter((e) => !q || e.name.toLowerCase().includes(q)),
          dirs: g.entries.filter((e) => e.isDir),
        }))
        .filter((g) => g.files.length > 0 || g.dirs.length > 0);
    },
    [groups, searchQuery],
  );

  const activeName = activePath ? activePath.split("/").pop() || activePath : null;
  const fileCount = fileGroups.reduce((n, g) => n + g.files.length, 0);
  const selectableFiles = fileGroups.flatMap((group) => group.files);
  const allSelected = selectableFiles.length > 0 && selectableFiles.every((file) => selectedPaths.has(file.path));

  useEffect(() => {
    const valid = new Set(selectableFiles.map((file) => file.path));
    setSelectedPaths((current) => {
      const next = new Set([...current].filter((path) => valid.has(path)));
      return next.size === current.size ? current : next;
    });
  }, [selectableFiles]);

  const toggleSelected = useCallback((path: string) => {
    setSelectedPaths((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path); else next.add(path);
      return next;
    });
  }, []);

  const toggleAll = useCallback(() => {
    setSelectedPaths(allSelected ? new Set() : new Set(selectableFiles.map((file) => file.path)));
  }, [allSelected, selectableFiles]);

  const confirmBulkDelete = useCallback(async () => {
    if (!pendingBulk) return;
    const releaseWorkspaceAction = tryAcquireWorkspaceAction();
    if (!releaseWorkspaceAction) {
      toast.info("其他文件操作完成前无法批量删除");
      return;
    }
    const releaseWriteBarrier = acquireEditorWriteBarrier();
    setBulkDeleting(true);
    setBulkProgress({ done: 0, total: pendingBulk.length });
    try {
      if (!(await flushPendingEditors())) {
        toast.err(lastSaveErrorRef.current || "当前章节保存失败，已取消批量删除");
        return;
      }
      await saveQueue.drain();
      const result = await runBatch(
        pendingBulk,
        async (file) => {
          const res = await invokeTool("delete_file", { path: file.path });
          if (!res.ok) throw new Error(res.content || `删除失败：${file.path}`);
        },
        (done, total) => setBulkProgress({ done, total }),
      );
      const completedPaths = new Set(result.completed.map((file) => file.path));
      setSelectedPaths((current) => new Set([...current].filter((path) => !completedPaths.has(path))));
      if (activePathRef.current && completedPaths.has(activePathRef.current)) {
        activePathRef.current = null;
        contentRef.current = "";
        savedContentRef.current = "";
        documentWritableRef.current = false;
        setActivePath(null);
        setContent("");
        setSavedContent("");
        setReadOnlyReason(null);
      }
      await loadTree();
      if (result.failed.length === 0) toast.ok(`已删除 ${result.completed.length} 个文件`);
      else toast.err(`已删除 ${result.completed.length} 个，${result.failed.length} 个删除失败`);
      setPendingBulk(null);
    } finally {
      setBulkDeleting(false);
      setBulkProgress(null);
      releaseWorkspaceAction();
      releaseWriteBarrier();
    }
  }, [flushPendingEditors, lastSaveErrorRef, loadTree, pendingBulk, saveQueue, toast]);

  return (
    <div className="work-content revision2">
      <aside className="revision2__list">
        <div className="revision2__list-head">
          <span className="revision2__list-title">书稿</span>
          {!loadingList && <span className="chip">{fileCount} 篇</span>}
          {fileCount > 1 && (
            <button
              className={`btn btn--ghost btn--xs${manageMode ? " is-active" : ""}`}
              onClick={() => {
                setManageMode((value) => !value);
                setSelectedPaths(new Set());
              }}
              aria-pressed={manageMode}
            >
              {manageMode ? "完成" : "批量管理"}
            </button>
          )}
          <button
            className="btn btn--ghost btn--icon"
            onClick={() => void loadTree()}
            title="刷新"
            aria-label="刷新"
            style={{ marginLeft: "auto" }}
          >
            <IconRefresh size={15} />
          </button>
        </div>
        <div className="revision2__search">
          <IconSearch size={14} />
          <input
            type="text"
            className="revision2__search-input"
            placeholder="搜索章节…"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          {searchQuery && (
            <button
              className="revision2__search-clear"
              onClick={() => setSearchQuery("")}
              aria-label="清空搜索"
            >
              <IconClose size={12} />
            </button>
          )}
        </div>
        <div className="revision2__files">
          {manageMode && selectableFiles.length > 0 && (
            <BatchActions
              selectedCount={selectedPaths.size}
              totalCount={selectableFiles.length}
              allSelected={allSelected}
              onToggleAll={toggleAll}
              onClear={() => setSelectedPaths(new Set())}
              label="批量管理书稿文件"
            >
              <button
                type="button"
                className="btn btn--danger btn--sm"
                disabled={selectedPaths.size === 0 || bulkDeleting || writeLocked}
                onClick={() => setPendingBulk(selectableFiles.filter((file) => selectedPaths.has(file.path)))}
              >
                <IconTrash size={13} /> 删除已选
              </button>
            </BatchActions>
          )}
          {loadingList ? (
            <LoadingBlock label="正在翻阅书稿…" />
          ) : listError ? (
            <div className="studio2__notice-err">{listError}</div>
          ) : fileGroups.length === 0 ? (
            <div className="empty">
              <p className="empty__title">书房空空</p>
              <p className="empty__text">还没有任何书稿，点击下方新建章节开始你的故事。</p>
            </div>
          ) : (
            fileGroups.map((g) => (
              <div key={g.label || "root"} className="revision2__group">
                <div className="revision2__group-label">{g.label}</div>
                {g.dirs.map((d) => (
                  <div key={d.path} className="revision2__file is-dir">
                    <IconFolder size={14} />
                    {d.name}
                  </div>
                ))}
                  {g.files.map((f) => {
                   const isActive = f.path === activePath;
                   return (
                    <div key={f.path} className={`revision2__file-row${selectedPaths.has(f.path) ? " is-selected" : ""}`}>
                      {manageMode && (
                        <input
                          type="checkbox"
                          className="batch-select"
                          checked={selectedPaths.has(f.path)}
                          onChange={() => toggleSelected(f.path)}
                          aria-label={`选择文件：${f.path}`}
                        />
                      )}
                      <button
                        className={`revision2__file${isActive ? " is-active" : ""}`}
                        onClick={() => manageMode ? toggleSelected(f.path) : void openFile(f.path)}
                        title={f.path}
                        disabled={writeLocked || saving || deleting || loadingDoc}
                        aria-pressed={manageMode ? selectedPaths.has(f.path) : undefined}
                      >
                        <IconFile size={14} />
                        <span className="revision2__file-name">{f.name}</span>
                        {isActive && dirty && <span className="revision2__dot" />}
                      </button>
                    </div>
                   );
                 })}
              </div>
            ))
          )}
        </div>
        <button
          className="btn btn--primary revision2__new"
          onClick={() => void createChapter()}
          disabled={writeLocked || saving || deleting || loadingDoc}
        >
          <IconPlus size={16} />
          新建章节
        </button>
      </aside>

      <div className="revision2__editor">
        {!activePath ? (
          <div className="empty revision2__empty">
            <p className="empty__title">展卷研墨</p>
            <p className="empty__text">
              从左侧选择一篇书稿开始编辑，或新建一章。文字会以宋体从容铺陈在宣纸上。
            </p>
          </div>
        ) : (
          <>
            <div className="revision2__bar">
              <div className="revision2__path">
                <IconScroll size={16} />
                <strong>{activeName}</strong>
                <span className="revision2__path-sub">{activePath}</span>
              </div>
              <div className="revision2__bar-actions">
                <span className="revision2__count">{countChars(content)} 字</span>
                <span className={`revision2__status${dirty ? " is-dirty" : ""}`}>
                  {readOnlyReason ? "只读" : dirty ? "未保存" : "已落墨"}
                </span>
                <button
                  className="btn btn--primary btn--sm"
                  onClick={() => void save()}
                  disabled={!dirty || saving || writeLocked || !documentWritableRef.current}
                >
                  {saving ? <Spinner size={14} /> : <IconSave size={15} />}
                  保存
                </button>
                <button
                  className="btn btn--ghost btn--sm"
                  onClick={() => setDelOpen(true)}
                  disabled={saving || deleting || writeLocked}
                  title="删除本章"
                >
                  <IconTrash size={15} />
                </button>
              </div>
            </div>
            {loadingDoc ? (
              <LoadingBlock label="正在展卷…" />
            ) : docError ? (
              <div className="studio2__notice-err" style={{ margin: "16px" }}>{docError}</div>
            ) : (
              <>
                {readOnlyReason && (
                  <div className="studio2__notice-err" style={{ margin: "16px" }} role="alert">
                    {readOnlyReason}
                  </div>
                )}
                <div className="revision2__sheet">
                  <textarea
                    ref={surfaceRef}
                    className="revision2__surface"
                    value={content}
                    onChange={(e) => {
                      if (writeLockedRef.current || !documentWritableRef.current) return;
                      contentRef.current = e.target.value;
                      setContent(e.target.value);
                    }}
                    placeholder="落笔成文，研墨入纸……"
                    readOnly={writeLocked || !!readOnlyReason}
                    spellCheck={false}
                  />
                </div>
              </>
            )}
          </>
        )}
      </div>

      <ConfirmModal
        open={delOpen || pendingBulk !== null}
        title={pendingBulk ? `删除选中的 ${pendingBulk.length} 个文件？` : "删除这一章？"}
        sealChar="删"
        danger
        busy={deleting || bulkDeleting}
        confirmLabel="删除"
        body={
          <>
            {pendingBulk ? (
              <>将永久删除选中的 {pendingBulk.length} 个工作区文件。{bulkProgress && <><br />正在处理：{bulkProgress.done} / {bulkProgress.total}</>}<br />此操作不可撤销。</>
            ) : (
              <>将永久删除文件<br /><code>{activePath}</code><br />此操作不可撤销。</>
            )}
          </>
        }
        onConfirm={() => pendingBulk ? void confirmBulkDelete() : void deleteChapter()}
        onCancel={() => {
          if (!deleting && !bulkDeleting) {
            setDelOpen(false);
            setPendingBulk(null);
          }
        }}
      />
    </div>
  );
}
