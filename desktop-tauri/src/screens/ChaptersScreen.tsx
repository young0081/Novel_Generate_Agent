// 章节 — browse the workspace (book/ + root), open a file into a serif
// writing surface, edit, save (write_file), and create new chapters.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Panel from "../components/Panel";
import { LoadingBlock, Spinner } from "../components/Spinner";
import EmptyState from "../components/EmptyState";
import ConfirmModal from "../components/ConfirmModal";
import {
  IconPlus,
  IconSave,
  IconRefresh,
  IconFile,
  IconFolder,
  IconScroll,
  IconTrash,
} from "../components/icons";
import { invokeTool, describeError } from "../lib/core";
import { useToast } from "../components/Toast";

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

/**
 * list_dir returns a newline listing in `content`. Directory rows commonly
 * end with "/". We parse defensively and tolerate either style.
 */
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
  // count without whitespace, a friendlier "字数" for CJK prose
  return text.replace(/\s/g, "").length;
}

export default function ChaptersScreen() {
  const toast = useToast();
  const [groups, setGroups] = useState<FileGroup[]>([]);
  const [loadingList, setLoadingList] = useState(true);
  const [listError, setListError] = useState<string | null>(null);

  const [activePath, setActivePath] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [savedContent, setSavedContent] = useState("");
  const [loadingDoc, setLoadingDoc] = useState(false);
  const [saving, setSaving] = useState(false);
  const [docError, setDocError] = useState<string | null>(null);
  const [delOpen, setDelOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const surfaceRef = useRef<HTMLTextAreaElement>(null);
  const dirty = content !== savedContent;

  const loadTree = useCallback(async () => {
    setLoadingList(true);
    setListError(null);
    try {
      const targets: { label: string; base: string }[] = [
        { label: "book 目录", base: "book" },
        { label: "工作区根目录", base: "" },
      ];
      const next: FileGroup[] = [];
      for (const t of targets) {
        const res = await invokeTool<unknown>("list_dir", { path: t.base });
        if (!res.ok) {
          // a missing book/ dir is fine — just skip it
          continue;
        }
        const entries = parseListing(res.content, t.base).filter(
          (e) => e.isDir || isLikelyTextFile(e.name),
        );
        if (entries.length > 0 || t.base === "") {
          next.push({ label: t.label, base: t.base, entries });
        }
      }
      setGroups(next);
    } catch (e) {
      setListError(describeError(e));
    } finally {
      setLoadingList(false);
    }
  }, []);

  useEffect(() => {
    void loadTree();
  }, [loadTree]);

  const openFile = useCallback(
    async (path: string) => {
      if (dirty) {
        const ok = window.confirm("当前章节有未保存的改动，确定要切换吗？");
        if (!ok) return;
      }
      setActivePath(path);
      setLoadingDoc(true);
      setDocError(null);
      try {
        const res = await invokeTool<unknown>("read_file", { path });
        if (!res.ok) {
          setDocError(res.content || "无法读取该文件");
          setContent("");
          setSavedContent("");
          return;
        }
        setContent(res.content);
        setSavedContent(res.content);
        if (res.metadata.truncated) {
          toast.info("文件较大，已截断显示");
        }
      } catch (e) {
        setDocError(describeError(e));
      } finally {
        setLoadingDoc(false);
      }
    },
    [dirty, toast],
  );

  const save = useCallback(async () => {
    if (!activePath || saving) return;
    setSaving(true);
    try {
      const res = await invokeTool("write_file", {
        path: activePath,
        content,
      });
      if (!res.ok) {
        toast.err(res.content || "保存失败");
        return;
      }
      setSavedContent(content);
      toast.ok("已保存");
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setSaving(false);
    }
  }, [activePath, content, saving, toast]);

  // Ctrl/Cmd+S
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (activePath && dirty) void save();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activePath, dirty, save]);

  const createChapter = useCallback(async () => {
    const raw = window.prompt(
      "新建章节的文件名（默认放入 book 目录）：",
      "ch1.md",
    );
    if (!raw) return;
    let name = raw.trim();
    if (!name) return;
    if (!/\.[a-z0-9]+$/i.test(name)) name += ".md";
    const path = name.includes("/") ? name : `book/${name}`;
    try {
      const res = await invokeTool("write_file", {
        path,
        content: `# ${name.replace(/\.[^.]+$/, "")}\n\n`,
      });
      if (!res.ok) {
        toast.err(res.content || "创建失败");
        return;
      }
      toast.ok(`已创建 ${path}`);
      await loadTree();
      await openFile(path);
    } catch (e) {
      toast.err(describeError(e));
    }
  }, [loadTree, openFile, toast]);

  const deleteChapter = useCallback(async () => {
    if (!activePath) return;
    setDeleting(true);
    try {
      const res = await invokeTool("delete_file", { path: activePath });
      if (!res.ok) {
        toast.err(res.content || "删除失败");
        return;
      }
      toast.ok(`已删除 ${activePath}`);
      setDelOpen(false);
      setActivePath(null);
      setContent("");
      setSavedContent("");
      await loadTree();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setDeleting(false);
    }
  }, [activePath, loadTree, toast]);

  const fileGroups = useMemo(
    () =>
      groups
        .map((g) => ({
          ...g,
          files: g.entries.filter((e) => !e.isDir),
          dirs: g.entries.filter((e) => e.isDir),
        }))
        .filter((g) => g.files.length > 0 || g.dirs.length > 0),
    [groups],
  );

  const activeName = activePath
    ? activePath.split("/").pop() || activePath
    : null;

  const headerActions = (
    <>
      <button
        className="btn btn--ghost btn--icon"
        onClick={() => void loadTree()}
        title="刷新列表"
        aria-label="刷新"
      >
        <IconRefresh size={16} />
      </button>
      <button className="btn btn--primary" onClick={() => void createChapter()}>
        <IconPlus size={16} />
        新建章节
      </button>
    </>
  );

  return (
    <Panel
      title="章节"
      en="Chapters"
      subtitle="书稿编辑 · 自动保存提示 · Ctrl/Cmd + S 落墨"
      actions={headerActions}
    >
      <div className="chapters">
        <aside className="chapters__list">
          <div className="chapters__listhead">
            <h3>书稿</h3>
            {!loadingList && (
              <span className="count-pill">
                {fileGroups.reduce((n, g) => n + g.files.length, 0)} 篇
              </span>
            )}
          </div>
          <div className="chapters__files">
            {loadingList ? (
              <LoadingBlock label="正在翻阅书稿…" />
            ) : listError ? (
              <div className="banner banner--warn" style={{ margin: "8px 4px" }}>
                {listError}
              </div>
            ) : fileGroups.length === 0 ? (
              <EmptyState
                title="书房空空"
                text="还没有任何书稿，点击右上角“新建章节”开始你的故事。"
              />
            ) : (
              fileGroups.map((g) => (
                <div key={g.label || "root"}>
                  <div className="filegroup__label">{g.label}</div>
                  {g.dirs.map((d) => (
                    <div key={d.path} className="file-item" style={{ cursor: "default", opacity: 0.7 }}>
                      <span className="file-item__icon">
                        <IconFolder size={15} />
                      </span>
                      <span className="file-item__name">{d.name}</span>
                    </div>
                  ))}
                  {g.files.map((f) => {
                    const isActive = f.path === activePath;
                    const isDirty = isActive && dirty;
                    return (
                      <button
                        key={f.path}
                        className={`file-item${isActive ? " is-active" : ""}`}
                        onClick={() => void openFile(f.path)}
                        title={f.path}
                      >
                        <span className="file-item__icon">
                          <IconFile size={15} />
                        </span>
                        <span className="file-item__name">{f.name}</span>
                        {isDirty ? <span className="file-item__dot" /> : null}
                      </button>
                    );
                  })}
                </div>
              ))
            )}
          </div>
        </aside>

        <div className="editor">
          {!activePath ? (
            <EmptyState
              title="展卷研墨"
              text="从左侧选择一篇书稿开始编辑，或新建一章。文字会以宋体竖排般的从容铺陈在宣纸上。"
              action={
                <button
                  className="btn btn--primary"
                  onClick={() => void createChapter()}
                >
                  <IconPlus size={16} />
                  新建章节
                </button>
              }
            />
          ) : (
            <>
              <div className="editor__bar">
                <div className="editor__path">
                  <IconScroll size={16} />
                  <strong>{activeName}</strong>
                  <span className="editor__path-sub">{activePath}</span>
                </div>
                <div className="row">
                  <span className="editor__count">
                    {countChars(content)} 字
                  </span>
                  <span className="divider-v" />
                  <span
                    className={`editor__status${dirty ? " is-dirty" : ""}`}
                  >
                    {dirty ? (
                      <>
                        <span className="ink-mark" />
                        未保存
                      </>
                    ) : (
                      "已落墨"
                    )}
                  </span>
                  <button
                    className="btn btn--primary btn--sm"
                    onClick={() => void save()}
                    disabled={!dirty || saving}
                  >
                    {saving ? <Spinner size={14} /> : <IconSave size={15} />}
                    保存
                  </button>
                  <button
                    className="btn btn--danger btn--sm"
                    onClick={() => setDelOpen(true)}
                    disabled={saving || deleting}
                    title="删除本章"
                  >
                    <IconTrash size={15} />
                    删除
                  </button>
                </div>
              </div>
              {loadingDoc ? (
                <LoadingBlock label="正在展卷…" />
              ) : docError ? (
                <div className="editor__surface-wrap">
                  <div
                    className="banner banner--warn"
                    style={{ margin: 0, alignSelf: "flex-start" }}
                  >
                    {docError}
                  </div>
                </div>
              ) : (
                <div className="editor__surface-wrap">
                  <div className="editor__sheet">
                    <textarea
                      ref={surfaceRef}
                      className="editor__surface"
                      value={content}
                      onChange={(e) => setContent(e.target.value)}
                      placeholder="落笔成文，研墨入纸……"
                      spellCheck={false}
                    />
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </div>

      <ConfirmModal
        open={delOpen}
        title="删除这一章？"
        sealChar="删"
        danger
        busy={deleting}
        confirmLabel="删除"
        body={
          <>
            将永久删除文件
            <br />
            <code>{activePath}</code>
            <br />
            此操作不可撤销（如需留底，可先到「快照」立此存照）。
          </>
        }
        onConfirm={() => void deleteChapter()}
        onCancel={() => {
          if (!deleting) setDelOpen(false);
        }}
      />
    </Panel>
  );
}
