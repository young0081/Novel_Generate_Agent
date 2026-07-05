// 知识库 — per-work RAG knowledge bases. Manage multiple bases, browse & curate
// entries, search (RAG preview), toggle which bases feed creation, and auto-fill
// a base from source material via the active model.

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Spinner } from "../components/Spinner";
import ConfirmModal from "../components/ConfirmModal";
import {
  IconPlus,
  IconTrash,
  IconSearch,
  IconClose,
  IconCheck,
  IconBrush,
  IconScroll,
  IconProviders,
} from "../components/icons";
import { useToast } from "../components/Toast";
import { useWork } from "../components/WorkContext";
import {
  listBases,
  createBase,
  deleteBase,
  setBaseActive,
  listEntries,
  addEntry,
  deleteEntry,
  searchKnowledge,
  fillFromTopic,
  KIND_LABELS,
  type KnowledgeBaseMeta,
  type KnowledgeEntry,
  type KnowledgeHit,
  type KnowledgeKind,
} from "../lib/knowledge";
import { describeError, isDesktop } from "../lib/core";

const KIND_OPTIONS: KnowledgeKind[] = [
  "character",
  "location",
  "worldbuilding",
  "event",
  "item",
  "term",
  "lore",
  "other",
];

export default function KnowledgeWork() {
  const toast = useToast();
  const { current } = useWork();

  const [bases, setBases] = useState<KnowledgeBaseMeta[]>([]);
  const [loadingBases, setLoadingBases] = useState(true);
  const [activeKb, setActiveKb] = useState<string | null>(null);
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [loadingEntries, setLoadingEntries] = useState(false);

  // search (RAG preview)
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<KnowledgeHit[] | null>(null);
  const [searching, setSearching] = useState(false);

  // create base
  const [newBaseName, setNewBaseName] = useState("");
  const [showNewBase, setShowNewBase] = useState(false);

  // add entry
  const [showAddEntry, setShowAddEntry] = useState(false);
  const [entryDraft, setEntryDraft] = useState<{
    kind: KnowledgeKind;
    title: string;
    content: string;
    tags: string;
  }>({ kind: "lore", title: "", content: "", tags: "" });

  // auto-fill
  const [showFill, setShowFill] = useState(false);
  const [fillTopic, setFillTopic] = useState("");
  const [filling, setFilling] = useState(false);
  const fillStreamRef = useRef("");
  const [fillStream, setFillStream] = useState("");

  // delete base
  const [delBase, setDelBase] = useState<KnowledgeBaseMeta | null>(null);
  const [deletingBase, setDeletingBase] = useState(false);

  const loadBases = useCallback(async () => {
    if (!isDesktop()) {
      setLoadingBases(false);
      return;
    }
    setLoadingBases(true);
    try {
      const list = await listBases();
      setBases(list);
      // keep selection valid
      setActiveKb((prev) => {
        if (prev && list.some((b) => b.id === prev)) return prev;
        return list[0]?.id ?? null;
      });
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setLoadingBases(false);
    }
  }, [toast]);

  const loadEntries = useCallback(
    async (kbId: string) => {
      setLoadingEntries(true);
      try {
        setEntries(await listEntries(kbId));
      } catch (e) {
        toast.err(describeError(e));
      } finally {
        setLoadingEntries(false);
      }
    },
    [toast],
  );

  // reload bases whenever the active work changes
  useEffect(() => {
    void loadBases();
  }, [loadBases, current?.id]);

  useEffect(() => {
    if (activeKb) void loadEntries(activeKb);
    else setEntries([]);
  }, [activeKb, loadEntries]);

  // stream agent progress during auto-fill
  useEffect(() => {
    if (!isDesktop()) return;
    const un = listen<{ step: string; content: string }>("agent-step", (e) => {
      // Append agent thoughts/tool calls to the stream display
      const text = e.payload.content || "";
      fillStreamRef.current += text + "\n";
      setFillStream(fillStreamRef.current);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const onCreateBase = useCallback(async () => {
    const name = newBaseName.trim();
    if (!name) return;
    try {
      const meta = await createBase(name);
      setNewBaseName("");
      setShowNewBase(false);
      await loadBases();
      setActiveKb(meta.id);
      toast.ok("知识库已创建");
    } catch (e) {
      toast.err(describeError(e));
    }
  }, [newBaseName, loadBases, toast]);

  const onToggleActive = useCallback(
    async (kb: KnowledgeBaseMeta) => {
      try {
        await setBaseActive(kb.id, !kb.active);
        await loadBases();
      } catch (e) {
        toast.err(describeError(e));
      }
    },
    [loadBases, toast],
  );

  const onAddEntry = useCallback(async () => {
    if (!activeKb || !entryDraft.title.trim() || !entryDraft.content.trim()) return;
    try {
      await addEntry({
        kbId: activeKb,
        kind: entryDraft.kind,
        title: entryDraft.title.trim(),
        content: entryDraft.content.trim(),
        tags: entryDraft.tags
          .split(/[,，\s]+/)
          .map((t) => t.trim())
          .filter(Boolean),
      });
      setEntryDraft({ kind: "lore", title: "", content: "", tags: "" });
      setShowAddEntry(false);
      await loadEntries(activeKb);
      await loadBases();
      toast.ok("已添加条目");
    } catch (e) {
      toast.err(describeError(e));
    }
  }, [activeKb, entryDraft, loadEntries, loadBases, toast]);

  const onDeleteEntry = useCallback(
    async (entryId: string) => {
      if (!activeKb) return;
      try {
        await deleteEntry(activeKb, entryId);
        await loadEntries(activeKb);
        await loadBases();
      } catch (e) {
        toast.err(describeError(e));
      }
    },
    [activeKb, loadEntries, loadBases, toast],
  );

  const onSearch = useCallback(async () => {
    const q = query.trim();
    if (!q) {
      setHits(null);
      return;
    }
    setSearching(true);
    try {
      setHits(await searchKnowledge(q, 10));
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setSearching(false);
    }
  }, [query, toast]);

  const onFill = useCallback(async () => {
    if (!activeKb || !fillTopic.trim() || filling) return;
    setFilling(true);
    fillStreamRef.current = "";
    setFillStream("");
    try {
      const { added } = await fillFromTopic(activeKb, fillTopic.trim());
      toast.ok(`已填充 ${added} 条设定资料`);
      setShowFill(false);
      setFillTopic("");
      await loadEntries(activeKb);
      await loadBases();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setFilling(false);
    }
  }, [activeKb, fillTopic, filling, loadEntries, loadBases, toast]);

  const confirmDeleteBase = useCallback(async () => {
    if (!delBase) return;
    setDeletingBase(true);
    try {
      await deleteBase(delBase.id);
      setDelBase(null);
      await loadBases();
      toast.ok("知识库已删除");
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setDeletingBase(false);
    }
  }, [delBase, loadBases, toast]);

  const currentBase = bases.find((b) => b.id === activeKb) ?? null;

  return (
    <div className="kb">
      {/* Left: base list */}
      <aside className="kb__sidebar">
        <div className="kb__sidebar-head">
          <span className="kb__sidebar-title">知识库</span>
          <button
            className="icon-btn"
            title="新建知识库"
            aria-label="新建知识库"
            onClick={() => setShowNewBase((s) => !s)}
          >
            <IconPlus size={15} />
          </button>
        </div>

        {showNewBase && (
          <div className="kb__new-base">
            <input
              className="field__input"
              value={newBaseName}
              onChange={(e) => setNewBaseName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void onCreateBase();
                if (e.key === "Escape") setShowNewBase(false);
              }}
              placeholder="知识库名称…"
              autoFocus
            />
            <button
              className="btn btn--primary btn--sm"
              onClick={() => void onCreateBase()}
              disabled={!newBaseName.trim()}
            >
              创建
            </button>
          </div>
        )}

        <div className="kb__base-list">
          {loadingBases ? (
            <div className="kb__loading">
              <Spinner size={20} />
            </div>
          ) : bases.length === 0 ? (
            <p className="kb__sidebar-empty">还没有知识库</p>
          ) : (
            bases.map((b) => (
              <button
                key={b.id}
                className={`kb__base${b.id === activeKb ? " is-active" : ""}`}
                onClick={() => setActiveKb(b.id)}
              >
                <span className="kb__base-name">{b.name}</span>
                <span className="kb__base-count">{b.entry_count}</span>
                <span
                  className={`kb__base-dot${b.active ? " is-on" : ""}`}
                  title={b.active ? "参与检索" : "已禁用"}
                />
              </button>
            ))
          )}
        </div>

        <div className="kb__sidebar-foot">
          <span className="kb__legend">
            <span className="kb__base-dot is-on" /> 绿点 = 参与创作检索
          </span>
        </div>
      </aside>

      {/* Right: entries + tools */}
      <section className="kb__main">
        {!currentBase ? (
          <div className="kb__empty">
            <div className="kb__empty-icon">
              <IconScroll size={32} />
            </div>
            <h3>为《{current?.title ?? "作品"}》建立知识库</h3>
            <p>
              知识库为创作提供设定准绳（RAG）：AI 写作前会自动检索相关条目，
              <br />
              确保情节不偏离世界观与人物设定。
            </p>
            <button className="btn btn--primary" onClick={() => setShowNewBase(true)}>
              <IconPlus size={16} />
              新建知识库
            </button>
          </div>
        ) : (
          <>
            <header className="kb__main-head">
              <div className="kb__main-title">
                <h2>{currentBase.name}</h2>
                <label className="kb__active-toggle">
                  <input
                    type="checkbox"
                    checked={currentBase.active}
                    onChange={() => void onToggleActive(currentBase)}
                  />
                  参与创作检索
                </label>
              </div>
              <div className="kb__main-actions">
                <button className="btn btn--ghost btn--sm" onClick={() => setShowFill(true)}>
                  <IconProviders size={14} />
                  联网填充
                </button>
                <button
                  className="btn btn--ghost btn--sm"
                  onClick={() => setShowAddEntry(true)}
                >
                  <IconPlus size={14} />
                  添加条目
                </button>
                <button
                  className="icon-btn icon-btn--danger"
                  title="删除知识库"
                  aria-label="删除知识库"
                  onClick={() => setDelBase(currentBase)}
                >
                  <IconTrash size={15} />
                </button>
              </div>
            </header>

            {/* RAG search */}
            <div className="kb__search">
              <IconSearch size={15} />
              <input
                className="kb__search-input"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void onSearch();
                  if (e.key === "Escape") {
                    setQuery("");
                    setHits(null);
                  }
                }}
                placeholder="检索设定（模拟创作时的 RAG 召回）…"
              />
              {query && (
                <button
                  className="icon-btn"
                  onClick={() => {
                    setQuery("");
                    setHits(null);
                  }}
                  aria-label="清空"
                >
                  <IconClose size={13} />
                </button>
              )}
              <button
                className="btn btn--primary btn--sm"
                onClick={() => void onSearch()}
                disabled={!query.trim() || searching}
              >
                {searching ? <Spinner size={13} /> : "检索"}
              </button>
            </div>

            {/* Search results OR all entries */}
            <div className="kb__entries">
              {hits !== null ? (
                <>
                  <div className="kb__entries-label">
                    检索结果 · {hits.length} 条
                    <button
                      className="link-btn"
                      onClick={() => {
                        setHits(null);
                        setQuery("");
                      }}
                    >
                      返回全部
                    </button>
                  </div>
                  {hits.length === 0 ? (
                    <p className="kb__no-results">没有命中的设定条目</p>
                  ) : (
                    hits.map((h) => (
                      <article key={h.entry.id} className="kb-entry">
                        <div className="kb-entry__head">
                          <span className={`kb-entry__kind kind--${h.entry.kind}`}>
                            {KIND_LABELS[h.entry.kind]}
                          </span>
                          <h4 className="kb-entry__title">{h.entry.title}</h4>
                          <span className="kb-entry__score">
                            {h.score.toFixed(2)} · {h.kb_name}
                          </span>
                        </div>
                        <p className="kb-entry__content">{h.entry.content}</p>
                      </article>
                    ))
                  )}
                </>
              ) : loadingEntries ? (
                <div className="kb__loading">
                  <Spinner size={24} />
                </div>
              ) : entries.length === 0 ? (
                <div className="kb__entries-empty">
                  <p>这个知识库还是空的。</p>
                  <p className="kb__entries-empty-hint">
                    手动添加条目，或用「联网填充」让 AI 自动整理设定资料。
                  </p>
                </div>
              ) : (
                entries.map((en) => (
                  <article key={en.id} className="kb-entry">
                    <div className="kb-entry__head">
                      <span className={`kb-entry__kind kind--${en.kind}`}>
                        {KIND_LABELS[en.kind]}
                      </span>
                      <h4 className="kb-entry__title">{en.title}</h4>
                      <button
                        className="icon-btn icon-btn--danger kb-entry__del"
                        title="删除条目"
                        aria-label="删除条目"
                        onClick={() => void onDeleteEntry(en.id)}
                      >
                        <IconTrash size={13} />
                      </button>
                    </div>
                    <p className="kb-entry__content">{en.content}</p>
                    {en.tags.length > 0 && (
                      <div className="kb-entry__tags">
                        {en.tags.map((t) => (
                          <span key={t} className="kb-entry__tag">
                            {t}
                          </span>
                        ))}
                      </div>
                    )}
                  </article>
                ))
              )}
            </div>
          </>
        )}
      </section>

      {/* Add-entry drawer */}
      {showAddEntry && (
        <div className="library__overlay" onClick={() => setShowAddEntry(false)}>
          <div
            className="library__sheet"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-label="添加条目"
          >
            <header className="library__sheet-head">
              <h3>添加设定条目</h3>
              <button className="icon-btn" onClick={() => setShowAddEntry(false)} aria-label="关闭">
                <IconClose size={16} />
              </button>
            </header>
            <div className="library__sheet-body">
              <div className="field-row">
                <label className="field">
                  <span className="field__label">类型</span>
                  <select
                    className="field__input"
                    value={entryDraft.kind}
                    onChange={(e) =>
                      setEntryDraft((d) => ({ ...d, kind: e.target.value as KnowledgeKind }))
                    }
                  >
                    {KIND_OPTIONS.map((k) => (
                      <option key={k} value={k}>
                        {KIND_LABELS[k]}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field" style={{ flex: 2 }}>
                  <span className="field__label">标题 *</span>
                  <input
                    className="field__input"
                    value={entryDraft.title}
                    onChange={(e) => setEntryDraft((d) => ({ ...d, title: e.target.value }))}
                    placeholder="名称 / 术语…"
                    autoFocus
                  />
                </label>
              </div>
              <label className="field">
                <span className="field__label">内容 *</span>
                <textarea
                  className="field__input field__textarea"
                  value={entryDraft.content}
                  onChange={(e) => setEntryDraft((d) => ({ ...d, content: e.target.value }))}
                  placeholder="详细设定描述…"
                  rows={5}
                />
              </label>
              <label className="field">
                <span className="field__label">标签</span>
                <input
                  className="field__input"
                  value={entryDraft.tags}
                  onChange={(e) => setEntryDraft((d) => ({ ...d, tags: e.target.value }))}
                  placeholder="用逗号分隔，如：主角, 剑客"
                />
              </label>
            </div>
            <footer className="library__sheet-foot">
              <button className="btn btn--ghost" onClick={() => setShowAddEntry(false)}>
                取消
              </button>
              <button
                className="btn btn--primary"
                onClick={() => void onAddEntry()}
                disabled={!entryDraft.title.trim() || !entryDraft.content.trim()}
              >
                <IconCheck size={16} />
                添加
              </button>
            </footer>
          </div>
        </div>
      )}

      {/* Auto-fill drawer */}
      {showFill && (
        <div className="library__overlay" onClick={() => !filling && setShowFill(false)}>
          <div
            className="library__sheet"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-label="联网填充"
          >
            <header className="library__sheet-head">
              <h3>联网填充设定资料</h3>
              <button
                className="icon-btn"
                onClick={() => !filling && setShowFill(false)}
                aria-label="关闭"
              >
                <IconClose size={16} />
              </button>
            </header>
            <div className="library__sheet-body">
              <label className="field">
                <span className="field__label">作品 / 题材</span>
                <input
                  className="field__input"
                  value={fillTopic}
                  onChange={(e) => setFillTopic(e.target.value)}
                  placeholder={current?.source_material || "例如：斗破苍穹"}
                  disabled={filling}
                  autoFocus
                />
              </label>
              <p className="library__hint">
                <IconBrush size={13} />
                AI 将整理该作品的核心人物、世界规则、地点、事件与术语，自动写入当前知识库。
              </p>
              {filling && fillStream && (
                <div className="kb__fill-stream">
                  <div className="kb__fill-stream-label">
                    <Spinner size={12} />
                    正在整理…
                  </div>
                  <pre className="kb__fill-stream-text">{fillStream.slice(-600)}</pre>
                </div>
              )}
            </div>
            <footer className="library__sheet-foot">
              <button
                className="btn btn--ghost"
                onClick={() => !filling && setShowFill(false)}
                disabled={filling}
              >
                取消
              </button>
              <button
                className="btn btn--primary"
                onClick={() => void onFill()}
                disabled={!fillTopic.trim() || filling}
              >
                {filling ? <Spinner size={14} /> : <IconProviders size={16} />}
                {filling ? "填充中…" : "开始填充"}
              </button>
            </footer>
          </div>
        </div>
      )}

      <ConfirmModal
        open={!!delBase}
        title="删除这个知识库？"
        sealChar="删"
        danger
        busy={deletingBase}
        confirmLabel="删除"
        body={
          <>
            将永久删除知识库「{delBase?.name}」及其全部 {delBase?.entry_count} 条设定。
            <br />
            此操作不可撤销。
          </>
        }
        onConfirm={() => void confirmDeleteBase()}
        onCancel={() => {
          if (!deletingBase) setDelBase(null);
        }}
      />
    </div>
  );
}
