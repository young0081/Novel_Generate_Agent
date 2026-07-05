// 修订 — Chapter editor (new desk-workflow layout).
// Browse book/ + root, open a file into a serif writing surface, edit, save,
// create and delete chapters. Wires the real list_dir / read_file / write_file
// / delete_file tools.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { LoadingBlock, Spinner } from "../components/Spinner";
import ConfirmModal from "../components/ConfirmModal";
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
  const [docError, setDocError] = useState<string | null>(null);
  const [delOpen, setDelOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");

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
        if (!res.ok) continue;
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
        if (res.metadata.truncated) toast.info("文件较大，已截断显示");
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
      const res = await invokeTool("write_file", { path: activePath, content });
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
    const raw = window.prompt("新建章节的文件名（默认放入 book 目录）：", "ch1.md");
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

  return (
    <div className="work-content revision2">
      <aside className="revision2__list">
        <div className="revision2__list-head">
          <span className="revision2__list-title">书稿</span>
          {!loadingList && <span className="chip">{fileCount} 篇</span>}
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
                    <button
                      key={f.path}
                      className={`revision2__file${isActive ? " is-active" : ""}`}
                      onClick={() => void openFile(f.path)}
                      title={f.path}
                    >
                      <IconFile size={14} />
                      <span className="revision2__file-name">{f.name}</span>
                      {isActive && dirty && <span className="revision2__dot" />}
                    </button>
                  );
                })}
              </div>
            ))
          )}
        </div>
        <button className="btn btn--primary revision2__new" onClick={() => void createChapter()}>
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
                  {dirty ? "未保存" : "已落墨"}
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
                  className="btn btn--ghost btn--sm"
                  onClick={() => setDelOpen(true)}
                  disabled={saving || deleting}
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
              <div className="revision2__sheet">
                <textarea
                  ref={surfaceRef}
                  className="revision2__surface"
                  value={content}
                  onChange={(e) => setContent(e.target.value)}
                  placeholder="落笔成文，研墨入纸……"
                  spellCheck={false}
                />
              </div>
            )}
          </>
        )}
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
            此操作不可撤销。
          </>
        }
        onConfirm={() => void deleteChapter()}
        onCancel={() => {
          if (!deleting) setDelOpen(false);
        }}
      />
    </div>
  );
}
