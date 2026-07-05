// 人物 / 伏笔 / 设定 — all backed by the memory tools.
// Each instance is configured with the set of kinds it represents and the
// default kind for new entries.

import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type FormEvent,
} from "react";
import Panel from "../components/Panel";
import { Spinner } from "../components/Spinner";
import { SkeletonGrid } from "../components/Skeleton";
import EmptyState from "../components/EmptyState";
import Dots from "../components/Dots";
import ConfirmModal from "../components/ConfirmModal";
import {
  IconPlus,
  IconSearch,
  IconRefresh,
  IconClose,
  IconTag,
  IconTrash,
} from "../components/icons";
import { invokeTool, describeError } from "../lib/core";
import {
  KIND_LABEL,
  type MemoryHit,
  type MemoryKind,
  type MemoryListData,
  type MemoryRecallData,
} from "../lib/memory";
import { useToast } from "../components/Toast";

export interface MemoryScreenConfig {
  title: string;
  en: string;
  subtitle: string;
  /** Kinds this screen displays (and filters recall results to). */
  kinds: MemoryKind[];
  /** Default kind for the 新增 form. */
  defaultKind: MemoryKind;
  /** A broad recall query used to surface the whole set. */
  defaultQuery: string;
}

interface NewEntry {
  kind: MemoryKind;
  title: string;
  summary: string;
  content: string;
  tags: string;
  importance: number;
}

function emptyEntry(kind: MemoryKind): NewEntry {
  return { kind, title: "", summary: "", content: "", tags: "", importance: 3 };
}

export default function MemoryScreen({
  config,
}: {
  config: MemoryScreenConfig;
}) {
  const toast = useToast();
  const [hits, setHits] = useState<MemoryHit[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [activeKind, setActiveKind] = useState<MemoryKind | "all">("all");

  const [formOpen, setFormOpen] = useState(false);
  const [entry, setEntry] = useState<NewEntry>(emptyEntry(config.defaultKind));
  const [saving, setSaving] = useState(false);

  const [pending, setPending] = useState<MemoryHit | null>(null);
  const [deleting, setDeleting] = useState(false);

  const kindSet = useMemo(() => new Set(config.kinds), [config.kinds]);

  const recall = useCallback(
    async (q: string) => {
      setLoading(true);
      setError(null);
      try {
        const trimmed = q.trim();
        let all: MemoryHit[];
        if (trimmed) {
          // Search by relevance (top-k; k must stay within the tool's max of 50).
          const res = await invokeTool<MemoryRecallData>("memory_recall", {
            query: trimmed,
            k: 50,
          });
          if (!res.ok) {
            setError(res.content || "检索失败");
            setHits([]);
            return;
          }
          all = res.data?.hits ?? [];
        } else {
          // Browse the whole category — listing, not searching.
          const res = await invokeTool<MemoryListData>("memory_list", {
            kinds: config.kinds,
            limit: 500,
          });
          if (!res.ok) {
            setError(res.content || "加载失败");
            setHits([]);
            return;
          }
          all = res.data?.entries ?? [];
        }
        // keep only the kinds this screen owns
        setHits(all.filter((h) => kindSet.has(h.kind)));
      } catch (e) {
        setError(describeError(e));
        setHits([]);
      } finally {
        setLoading(false);
      }
    },
    [config.kinds, kindSet],
  );

  useEffect(() => {
    void recall("");
    // reset form default when switching screens
    setEntry(emptyEntry(config.defaultKind));
    setActiveKind("all");
    setQuery("");
    setFormOpen(false);
  }, [recall, config.defaultKind]);

  const onSearch = useCallback(
    (e: FormEvent) => {
      e.preventDefault();
      void recall(query);
    },
    [query, recall],
  );

  const submit = useCallback(async () => {
    if (!entry.title.trim()) {
      toast.err("请填写标题");
      return;
    }
    setSaving(true);
    try {
      const tags = entry.tags
        .split(/[,，\s]+/)
        .map((t) => t.trim())
        .filter(Boolean);
      const res = await invokeTool("memory_save", {
        kind: entry.kind,
        title: entry.title.trim(),
        summary: entry.summary.trim(),
        content: entry.content.trim() || entry.summary.trim(),
        tags,
        importance: entry.importance,
      });
      if (!res.ok) {
        toast.err(res.content || "保存失败");
        return;
      }
      toast.ok(`已记入：${entry.title.trim()}`);
      setEntry(emptyEntry(config.defaultKind));
      setFormOpen(false);
      await recall(query);
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setSaving(false);
    }
  }, [entry, config.defaultKind, query, recall, toast]);

  const confirmDelete = useCallback(async () => {
    if (!pending) return;
    setDeleting(true);
    try {
      const res = await invokeTool("memory_delete", { id: pending.id });
      if (!res.ok) {
        toast.err(res.content || "删除失败");
        return;
      }
      toast.ok(`已删除：${pending.title}`);
      setPending(null);
      await recall(query);
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setDeleting(false);
    }
  }, [pending, query, recall, toast]);

  const visible = useMemo(
    () => (activeKind === "all" ? hits : hits.filter((h) => h.kind === activeKind)),
    [hits, activeKind],
  );

  const showSeg = config.kinds.length > 1;

  const headerActions = (
    <>
      <button
        className="btn btn--ghost btn--icon"
        onClick={() => void recall(query)}
        title="刷新"
        aria-label="刷新"
      >
        <IconRefresh size={16} />
      </button>
      <button
        className="btn btn--primary"
        onClick={() => setFormOpen((v) => !v)}
      >
        {formOpen ? <IconClose size={16} /> : <IconPlus size={16} />}
        {formOpen ? "收起" : "新增"}
      </button>
    </>
  );

  const toolbar = (
    <div className="toolbar">
      <form className="search-box" onSubmit={onSearch}>
        <IconSearch size={16} />
        <input
          className="input"
          placeholder={`检索${config.title}…（回车搜索）`}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </form>
      {showSeg && (
        <div className="seg" role="tablist" aria-label="按类别筛选">
          <button
            className={`seg__btn${activeKind === "all" ? " is-active" : ""}`}
            onClick={() => setActiveKind("all")}
          >
            全部
          </button>
          {config.kinds.map((k) => (
            <button
              key={k}
              className={`seg__btn${activeKind === k ? " is-active" : ""}`}
              onClick={() => setActiveKind(k)}
            >
              {KIND_LABEL[k]}
            </button>
          ))}
        </div>
      )}
      <span className="count-pill">{visible.length} 条</span>
    </div>
  );

  return (
    <Panel
      title={config.title}
      en={config.en}
      subtitle={config.subtitle}
      actions={headerActions}
      toolbar={toolbar}
    >
      <div className="scroll-area">
        {loading ? (
          <SkeletonGrid count={6} />
        ) : error ? (
          <div className="banner banner--warn">{error}</div>
        ) : visible.length === 0 ? (
          <EmptyState
            title={`暂无${config.title}`}
            text={`点击右上角“新增”，把你笔下的${config.title}记入创作记忆，AI 与你都能随时回望。`}
            action={
              <button
                className="btn btn--primary"
                onClick={() => setFormOpen(true)}
              >
                <IconPlus size={16} />
                新增{config.title}
              </button>
            }
          />
        ) : (
          <div className="slip-grid">
            {visible.map((h) => (
              <article className="slip" key={h.id}>
                <span className="slip__kind">
                  <span className="chip chip--accent">{KIND_LABEL[h.kind]}</span>
                </span>
                <div className="slip__top">
                  <h3 className="slip__title">{h.title}</h3>
                </div>
                <p className="slip__summary">
                  {h.summary || <span className="muted">（暂无摘要）</span>}
                </p>
                <div className="slip__tags">
                  {h.tags.slice(0, 4).map((t) => (
                    <span className="chip" key={t}>
                      <IconTag size={11} />
                      {t}
                    </span>
                  ))}
                </div>
                <div className="slip__meta">
                  <Dots value={h.importance} />
                  <span className="slip__meta-end">
                    {h.score !== undefined && (
                      <span className="slip__score">
                        相关 {(h.score * 100).toFixed(0)}%
                      </span>
                    )}
                    <button
                      className="slip__del"
                      onClick={() => setPending(h)}
                      title="删除"
                      aria-label="删除"
                    >
                      <IconTrash size={14} />
                    </button>
                  </span>
                </div>
              </article>
            ))}
          </div>
        )}
      </div>

      <ConfirmModal
        open={pending !== null}
        title={`删除这条${config.title}？`}
        sealChar="删"
        danger
        busy={deleting}
        confirmLabel="删除"
        body={
          <>
            将永久删除「{pending?.title}」，此操作不可撤销。
          </>
        }
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          if (!deleting) setPending(null);
        }}
      />

      {formOpen && (
        <aside className="drawer">
          <div className="drawer__head">
            <h3>新增{config.title}</h3>
            <button
              className="btn btn--ghost btn--icon"
              onClick={() => setFormOpen(false)}
              aria-label="关闭"
            >
              <IconClose size={16} />
            </button>
          </div>
          <div className="drawer__body">
            {config.kinds.length > 1 && (
              <div className="field">
                <label className="field__label">类别</label>
                <select
                  className="select"
                  value={entry.kind}
                  onChange={(e) =>
                    setEntry({ ...entry, kind: e.target.value as MemoryKind })
                  }
                >
                  {config.kinds.map((k) => (
                    <option key={k} value={k}>
                      {KIND_LABEL[k]}
                    </option>
                  ))}
                </select>
              </div>
            )}
            <div className="field">
              <label className="field__label">标题</label>
              <input
                className="input"
                value={entry.title}
                onChange={(e) => setEntry({ ...entry, title: e.target.value })}
                placeholder={`${config.title}名称`}
              />
            </div>
            <div className="field">
              <label className="field__label">摘要</label>
              <textarea
                className="textarea"
                value={entry.summary}
                onChange={(e) =>
                  setEntry({ ...entry, summary: e.target.value })
                }
                placeholder="一句话概括"
                style={{ minHeight: 70 }}
              />
            </div>
            <div className="field">
              <label className="field__label">详细内容</label>
              <textarea
                className="textarea"
                value={entry.content}
                onChange={(e) =>
                  setEntry({ ...entry, content: e.target.value })
                }
                placeholder="展开描写（留空则沿用摘要）"
                style={{ minHeight: 120 }}
              />
            </div>
            <div className="field">
              <label className="field__label">标签（逗号或空格分隔）</label>
              <input
                className="input"
                value={entry.tags}
                onChange={(e) => setEntry({ ...entry, tags: e.target.value })}
                placeholder="如：主角, 北境, 旧怨"
              />
            </div>
            <div className="field">
              <label className="field__label">
                重要度：{entry.importance} / 5
              </label>
              <input
                type="range"
                min={1}
                max={5}
                step={1}
                value={entry.importance}
                onChange={(e) =>
                  setEntry({ ...entry, importance: Number(e.target.value) })
                }
                style={{ accentColor: "var(--cinnabar)" }}
              />
            </div>
          </div>
          <div className="drawer__foot">
            <button
              className="btn"
              onClick={() => setEntry(emptyEntry(config.defaultKind))}
              disabled={saving}
            >
              清空
            </button>
            <button
              className="btn btn--primary"
              style={{ flex: 1 }}
              onClick={() => void submit()}
              disabled={saving}
            >
              {saving ? <Spinner size={15} /> : <IconPlus size={16} />}
              记入记忆
            </button>
          </div>
        </aside>
      )}
    </Panel>
  );
}
