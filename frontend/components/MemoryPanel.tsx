"use client";

import { type CSSProperties, useCallback, useEffect, useState } from "react";

import BatchActions from "@/components/BatchActions";
import Button from "@/components/Button";
import { useConnection } from "@/components/Connection";
import EmptyState from "@/components/EmptyState";
import { useToast } from "@/components/Toast";
import { runBatch } from "@/lib/batch";
import { invokeTool } from "@/lib/rpcClient";

const KINDS = [
  "character",
  "setting",
  "worldbuilding",
  "plot",
  "outline",
  "foreshadow",
  "dialogue",
  "lore",
  "other",
] as const;

const KIND_LABEL: Record<string, string> = {
  character: "人物",
  setting: "场景设定",
  worldbuilding: "世界观",
  plot: "情节",
  outline: "大纲",
  foreshadow: "伏笔",
  dialogue: "对话",
  lore: "传说设定",
  other: "其他",
};

interface MemoryEntry {
  id: string;
  kind: string;
  title: string;
  summary: string;
  tags: string[];
  importance: number;
  archived?: boolean;
}

interface MemoryListData {
  entries?: MemoryEntry[];
}

interface MemoryRecallData {
  hits?: unknown[];
}

function errorMessage(value: unknown): string {
  return value instanceof Error ? value.message : String(value);
}

export default function MemoryPanel() {
  const toast = useToast();
  const { reportError } = useConnection();

  const [kind, setKind] = useState<string>("character");
  const [title, setTitle] = useState("");
  const [summary, setSummary] = useState("");
  const [content, setContent] = useState("");
  const [tags, setTags] = useState("");
  const [importance, setImportance] = useState(3);

  const [query, setQuery] = useState("");
  const [hits, setHits] = useState("");
  const [searched, setSearched] = useState(false);
  const [saving, setSaving] = useState(false);
  const [recalling, setRecalling] = useState(false);

  const [entries, setEntries] = useState<MemoryEntry[]>([]);
  const [listLoaded, setListLoaded] = useState(false);
  const [listLoading, setListLoading] = useState(false);
  const [manageMode, setManageMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ done: number; total: number } | null>(null);

  const fail = useCallback(
    (e: unknown) => {
      const message = errorMessage(e);
      reportError(message);
      toast.error("错误：" + message);
    },
    [reportError, toast],
  );

  const refreshList = useCallback(async () => {
    setListLoading(true);
    try {
      const r = await invokeTool<MemoryListData>("memory_list", { limit: 500 });
      if (!r.ok) {
        toast.error(r.content || "读取记忆列表失败");
        return;
      }
      const next = Array.isArray(r.data?.entries) ? r.data.entries : [];
      setEntries(next.filter((entry) => entry && typeof entry.id === "string"));
      setListLoaded(true);
    } catch (e) {
      fail(e);
    } finally {
      setListLoading(false);
    }
  }, [fail, toast]);

  useEffect(() => {
    void refreshList();
  }, [refreshList]);

  useEffect(() => {
    const valid = new Set(entries.map((entry) => entry.id));
    setSelectedIds((current) => {
      const next = new Set([...current].filter((id) => valid.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [entries]);

  const save = useCallback(async () => {
    setSaving(true);
    try {
      const r = await invokeTool("memory_save", {
        kind,
        title,
        summary,
        content,
        tags: tags
          .split(/[,，]/)
          .map((t) => t.trim())
          .filter(Boolean),
        importance,
      });
      if (r.ok) {
        toast.success(`已保存：${r.summary ?? r.content}`);
        await refreshList();
      } else {
        toast.error(r.content);
      }
    } catch (e) {
      fail(e);
    } finally {
      setSaving(false);
    }
  }, [kind, title, summary, content, tags, importance, fail, refreshList, toast]);

  const recall = useCallback(async () => {
    setRecalling(true);
    try {
      const r = await invokeTool<MemoryRecallData>("memory_recall", { query, k: 8 });
      if (!r.ok) {
        setHits("");
        setSearched(false);
        toast.error(r.content);
        return;
      }
      setHits(r.content || "");
      setSearched(true);
    } catch (e) {
      fail(e);
    } finally {
      setRecalling(false);
    }
  }, [query, fail, toast]);

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const allSelected = entries.length > 0 && entries.every((entry) => selectedIds.has(entry.id));

  const toggleAll = useCallback(() => {
    setSelectedIds(allSelected ? new Set() : new Set(entries.map((entry) => entry.id)));
  }, [allSelected, entries]);

  const deleteOne = useCallback(
    async (entry: MemoryEntry) => {
      if (bulkDeleting || !window.confirm(`确定永久删除记忆“${entry.title}”吗？`)) return;
      setDeletingId(entry.id);
      try {
        const r = await invokeTool("memory_delete", { id: entry.id });
        if (!r.ok) {
          toast.error(r.content || "删除失败");
          return;
        }
        setEntries((current) => current.filter((item) => item.id !== entry.id));
        setSelectedIds((current) => {
          const next = new Set(current);
          next.delete(entry.id);
          return next;
        });
        toast.success(`已删除：${entry.title}`);
      } catch (e) {
        fail(e);
      } finally {
        setDeletingId(null);
      }
    },
    [bulkDeleting, fail, toast],
  );

  const deleteSelected = useCallback(async () => {
    const selected = entries.filter((entry) => selectedIds.has(entry.id));
    if (selected.length === 0 || bulkDeleting || deletingId !== null) return;
    if (!window.confirm(`确定永久删除选中的 ${selected.length} 条记忆吗？此操作不可撤销。`)) return;
    setBulkDeleting(true);
    setBulkProgress({ done: 0, total: selected.length });
    try {
      const result = await runBatch(
        selected,
        async (entry) => {
          const r = await invokeTool("memory_delete", { id: entry.id });
          if (!r.ok) throw new Error(r.content || `删除失败：${entry.title}`);
        },
        (done, total) => setBulkProgress({ done, total }),
      );
      const completedIds = new Set(result.completed.map((entry) => entry.id));
      setEntries((current) => current.filter((entry) => !completedIds.has(entry.id)));
      setSelectedIds((current) => new Set([...current].filter((id) => !completedIds.has(id))));
      if (result.failed.length === 0) {
        toast.success(`已删除 ${result.completed.length} 条记忆`);
      } else {
        toast.error(`已删除 ${result.completed.length} 条，${result.failed.length} 条失败`);
      }
    } finally {
      setBulkDeleting(false);
      setBulkProgress(null);
    }
  }, [entries, selectedIds, bulkDeleting, deletingId, toast]);

  return (
    <div>
      <div className="card" style={{ "--i": 0 } as CSSProperties}>
        <div className="card-head">
          <h3>检索记忆（RAG）</h3>
          <span className="hint">中文友好 BM25 · 只返回结构化摘要</span>
        </div>
        <div className="row">
          <input
            className="grow"
            placeholder="搜索，例如：北境 剑客 主角"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !recalling && void recall()}
          />
          <Button onClick={recall} loading={recalling}>
            检索
          </Button>
        </div>
        {searched &&
          (hits ? (
            <pre className="output">{hits}</pre>
          ) : (
            <EmptyState
              icon="🔍"
              title="没有匹配的记忆"
              hint="换个关键词试试，或先在下面新增一条记忆。"
            />
          ))}
      </div>

      <div className="card" style={{ "--i": 1 } as CSSProperties}>
        <div className="card-head">
          <div>
            <h3>记忆条目</h3>
            <span className="hint">可勾选当前列表，批量删除不再需要的设定</span>
          </div>
          <div className="row">
            <span className="badge">{entries.length}</span>
            {entries.length > 0 && (
              <Button
                variant="ghost"
                onClick={() => {
                  setManageMode((value) => !value);
                  setSelectedIds(new Set());
                }}
                disabled={bulkDeleting}
              >
                {manageMode ? "完成" : "批量管理"}
              </Button>
            )}
            <Button variant="ghost" onClick={refreshList} loading={listLoading} disabled={bulkDeleting}>
              刷新
            </Button>
          </div>
        </div>
        {manageMode && entries.length > 0 && (
          <BatchActions
            selectedCount={selectedIds.size}
            totalCount={entries.length}
            allSelected={allSelected}
            busy={bulkDeleting || deletingId !== null}
            progress={bulkProgress}
            onToggleAll={toggleAll}
            onClear={() => setSelectedIds(new Set())}
            onDelete={() => void deleteSelected()}
            label="批量管理记忆"
          />
        )}
        {listLoading && !listLoaded ? (
          <p className="hint loading-line"><span className="spinner" /> 正在读取记忆…</p>
        ) : entries.length === 0 ? (
          <EmptyState icon="🧠" title="暂无记忆条目" hint="在下方新增人物、设定或大纲，后续创作可随时检索。" />
        ) : (
          <div className="batch-list">
            {entries.map((entry) => (
              <div className={`batch-row${selectedIds.has(entry.id) ? " is-selected" : ""}`} key={entry.id}>
                {manageMode && (
                  <input
                    type="checkbox"
                    className="batch-select"
                    checked={selectedIds.has(entry.id)}
                    onChange={() => toggleSelected(entry.id)}
                    aria-label={`选择记忆：${entry.title}`}
                    disabled={bulkDeleting}
                  />
                )}
                <div className="batch-row__main">
                  <div className="batch-row__title">
                    <strong>{entry.title || "（未命名）"}</strong>
                    <span className="badge">{KIND_LABEL[entry.kind] || entry.kind}</span>
                    {entry.archived && <span className="badge">已归档</span>}
                  </div>
                  <div className="batch-row__summary">{entry.summary || "（暂无摘要）"}</div>
                  {entry.tags.length > 0 && <div className="batch-row__meta">{entry.tags.join(" · ")}</div>}
                </div>
                <button
                  type="button"
                  className="batch-row__delete"
                  onClick={() => void deleteOne(entry)}
                  disabled={bulkDeleting || deletingId !== null}
                  aria-label={`删除记忆：${entry.title}`}
                >
                  {deletingId === entry.id ? "删除中…" : "删除"}
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="card" style={{ "--i": 2 } as CSSProperties}>
        <div className="card-head">
          <h3>新增记忆</h3>
        </div>
        <div className="row">
          <label className="field" style={{ width: 140 }}>
            <span>类型</span>
            <select value={kind} onChange={(e) => setKind(e.target.value)}>
              {KINDS.map((k) => (
                <option key={k} value={k}>
                  {KIND_LABEL[k]}
                </option>
              ))}
            </select>
          </label>
          <label className="field grow">
            <span>标题</span>
            <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="例如：林惊羽" />
          </label>
          <label className="field" style={{ width: 130 }}>
            <span>重要度 {importance}</span>
            <input
              type="range"
              min={1}
              max={5}
              value={importance}
              onChange={(e) => setImportance(Number(e.target.value))}
            />
          </label>
        </div>
        <label className="field">
          <span>一句话摘要（检索时展示）</span>
          <input value={summary} onChange={(e) => setSummary(e.target.value)} placeholder="冷静果敢的年轻剑客，主角。" />
        </label>
        <label className="field">
          <span>详细内容（仅在显式取用时才返回全文）</span>
          <textarea value={content} onChange={(e) => setContent(e.target.value)} />
        </label>
        <label className="field">
          <span>标签（逗号分隔）</span>
          <input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="主角, 剑客, 北境" />
        </label>
        <Button onClick={save} loading={saving} disabled={!title}>
          保存记忆
        </Button>
      </div>
    </div>
  );
}
