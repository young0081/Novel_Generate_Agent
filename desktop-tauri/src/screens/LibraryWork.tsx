// 书库 — the work library. A shelf of every novel project, each fully isolated.
// Create new works, switch between them, edit metadata, and delete. The active
// work drives the whole rest of the app (manuscript, memory, knowledge).

import { useCallback, useRef, useState } from "react";
import { Spinner } from "../components/Spinner";
import ConfirmModal from "../components/ConfirmModal";
import BatchActions from "../components/BatchActions";
import {
  IconPlus,
  IconBrush,
  IconTrash,
  IconPencil,
  IconCheck,
  IconScroll,
  IconClose,
} from "../components/icons";
import { useToast } from "../components/Toast";
import { useWork } from "../components/WorkContext";
import { createWork, updateWork, deleteWork, type WorkSummary } from "../lib/works";
import { describeError } from "../lib/core";
import { useDialogFocus } from "../lib/dialogLayer";
import { runBatch } from "../lib/batch";

function fmtDate(ms: number): string {
  try {
    return new Date(ms).toLocaleDateString("zh-CN", {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  } catch {
    return "";
  }
}

interface DraftForm {
  title: string;
  genre: string;
  source_material: string;
  blurb: string;
}

const EMPTY_DRAFT: DraftForm = { title: "", genre: "", source_material: "", blurb: "" };

export default function LibraryWork() {
  const toast = useToast();
  const { works, current, loading, error, refresh, switchTo } = useWork();
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState<DraftForm>(EMPTY_DRAFT);
  const [busy, setBusy] = useState(false);
  const [editId, setEditId] = useState<string | null>(null);
  const [delTarget, setDelTarget] = useState<WorkSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [manageMode, setManageMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [pendingBulk, setPendingBulk] = useState<WorkSummary[] | null>(null);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ done: number; total: number } | null>(null);
  const editorSheetRef = useRef<HTMLDivElement>(null);
  const busyRef = useRef(busy);
  busyRef.current = busy;

  const closeEditor = useCallback(() => {
    if (!busyRef.current) setCreating(false);
  }, []);

  useDialogFocus(creating, editorSheetRef, closeEditor);

  const submitCreate = useCallback(async () => {
    if (!draft.title.trim() || busy) return;
    setBusy(true);
    try {
      if (editId) {
        await updateWork(editId, {
          title: draft.title.trim(),
          genre: draft.genre.trim(),
          source_material: draft.source_material.trim(),
          blurb: draft.blurb.trim(),
        });
        toast.ok("作品已更新");
      } else {
        await createWork({
          title: draft.title.trim(),
          genre: draft.genre.trim(),
          source_material: draft.source_material.trim(),
          blurb: draft.blurb.trim(),
        });
        toast.ok("作品已创建并切换");
      }
      setCreating(false);
      setEditId(null);
      setDraft(EMPTY_DRAFT);
      await refresh();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setBusy(false);
    }
  }, [draft, busy, editId, toast, refresh]);

  const onSwitch = useCallback(
    async (id: string) => {
      if (id === current?.id) return;
      try {
        await switchTo(id);
        const w = works.find((x) => x.id === id);
        toast.ok(`已切换到《${w?.title ?? "作品"}》`);
      } catch (e) {
        toast.err(describeError(e));
      }
    },
    [current, works, switchTo, toast],
  );

  const onEdit = useCallback((w: WorkSummary) => {
    setEditId(w.id);
    setDraft({
      title: w.title,
      genre: w.genre,
      source_material: w.source_material,
      blurb: w.blurb,
    });
    setCreating(true);
  }, []);

  const confirmDelete = useCallback(async () => {
    if (pendingBulk) {
      setBulkDeleting(true);
      setBulkProgress({ done: 0, total: pendingBulk.length });
      try {
        const result = await runBatch(
          pendingBulk,
          async (work) => {
            await deleteWork(work.id, true);
          },
          (done, total) => setBulkProgress({ done, total }),
        );
        await refresh();
        const completedIds = new Set(result.completed.map((work) => work.id));
        setSelectedIds((current) => new Set([...current].filter((id) => !completedIds.has(id))));
        if (result.failed.length === 0) {
          toast.ok(`已删除 ${result.completed.length} 部作品`);
        } else {
          toast.err(`已删除 ${result.completed.length} 部，${result.failed.length} 部删除失败`);
        }
        setPendingBulk(null);
      } finally {
        setBulkDeleting(false);
        setBulkProgress(null);
      }
      return;
    }
    if (!delTarget) return;
    setDeleting(true);
    try {
      await deleteWork(delTarget.id, true);
      toast.ok("作品已删除");
      setDelTarget(null);
      await refresh();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setDeleting(false);
    }
  }, [delTarget, pendingBulk, toast, refresh]);

  const deletableWorks = works.filter((work) => !work.active);
  const allSelected = deletableWorks.length > 0 && deletableWorks.every((work) => selectedIds.has(work.id));

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const toggleAll = useCallback(() => {
    setSelectedIds(allSelected ? new Set() : new Set(deletableWorks.map((work) => work.id)));
  }, [allSelected, deletableWorks]);

  return (
    <div className="library">
      <header className="library__head">
        <div>
          <p className="panel__kicker">书库 · LIBRARY</p>
          <h2 className="library__title">我的作品</h2>
          <p className="library__sub">
            每一部作品都拥有独立的手稿、记忆与知识库，互不干扰。
          </p>
        </div>
        <div className="library__head-actions">
          <button
            className="btn btn--primary"
            onClick={() => {
              setEditId(null);
              setDraft(EMPTY_DRAFT);
              setCreating(true);
            }}
          >
            <IconPlus size={16} />
            新建作品
          </button>
        </div>
      </header>

      {loading ? (
        <div className="library__loading">
          <Spinner size={28} />
          <span>正在打开书库…</span>
        </div>
      ) : error ? (
        <div className="banner banner--warn">
          书库载入失败：{error}
          <button className="link-btn" onClick={() => void refresh()}>重试</button>
        </div>
      ) : works.length === 0 ? (
        <div className="library__empty">
          <div className="library__empty-main">
            <div className="library__empty-icon">
              <IconScroll size={34} />
            </div>
            <div className="library__empty-copy">
              <span className="library__empty-kicker">FIRST FOLIO</span>
              <h3>把第一个故事放上书架</h3>
              <p>新建作品后，设定、记忆、知识库与每一章手稿都会被独立保存。</p>
              <button
                className="btn btn--primary"
                onClick={() => {
                  setEditId(null);
                  setDraft(EMPTY_DRAFT);
                  setCreating(true);
                }}
              >
                <IconPlus size={16} />
                新建作品
              </button>
            </div>
          </div>
          <aside className="library__empty-aside" aria-label="创作路径">
            <span className="library__empty-aside-kicker">CREATIVE ROUTE</span>
            <h4>一部作品的四个落点</h4>
            <ol>
              <li><span>01</span>立下世界与人物</li>
              <li><span>02</span>收束主线与章节</li>
              <li><span>03</span>让模型协助运笔</li>
              <li><span>04</span>复盘、修订、成稿</li>
            </ol>
          </aside>
        </div>
      ) : (
        <>
        <div className="library__list-toolbar">
          <div className="library__list-meta">
            <span className="library__list-label">作品列表</span>
            <span className="library__list-count">{works.length} 部作品</span>
          </div>
          <div className="library__list-actions">
            {manageMode && deletableWorks.length > 0 && (
              <BatchActions
                selectedCount={selectedIds.size}
                totalCount={deletableWorks.length}
                allSelected={allSelected}
                onToggleAll={toggleAll}
                onClear={() => setSelectedIds(new Set())}
              >
                <button
                  type="button"
                  className="btn btn--danger btn--sm"
                  disabled={selectedIds.size === 0 || bulkDeleting}
                  onClick={() => setPendingBulk(deletableWorks.filter((work) => selectedIds.has(work.id)))}
                >
                  <IconTrash size={13} /> 删除已选
                </button>
              </BatchActions>
            )}
            {works.length > 1 && (
              <button
                className={`btn btn--ghost btn--sm${manageMode ? " is-active" : ""}`}
                onClick={() => {
                  setManageMode((value) => !value);
                  setSelectedIds(new Set());
                }}
                aria-pressed={manageMode}
              >
                {manageMode ? "完成" : "批量管理"}
              </button>
            )}
          </div>
        </div>
        <div className={`library__grid${manageMode ? " is-manage" : ""}`}>
          {works.map((w) => (
            <article
              key={w.id}
              className={`work-card${w.active ? " is-active" : ""}${selectedIds.has(w.id) ? " is-selected" : ""}`}
              onClick={() => manageMode ? !w.active && toggleSelected(w.id) : void onSwitch(w.id)}
              role="button"
              tabIndex={0}
              aria-current={w.active ? "true" : undefined}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) return;
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  if (manageMode) {
                    if (!w.active) toggleSelected(w.id);
                  } else {
                    void onSwitch(w.id);
                  }
                }
              }}
            >
              <div className="work-card__spine" />
              <div className="work-card__body">
                <div className="work-card__top">
                  {manageMode && (
                    <input
                      type="checkbox"
                      className="batch-select work-card__select"
                      checked={selectedIds.has(w.id)}
                      disabled={w.active}
                      onClick={(event) => event.stopPropagation()}
                      onChange={() => toggleSelected(w.id)}
                      aria-label={`选择作品：${w.title}`}
                    />
                  )}
                  <h3 className="work-card__title">{w.title}</h3>
                  {w.active && (
                    <span className="work-card__badge">
                      <IconCheck size={11} />
                      当前
                    </span>
                  )}
                </div>
                {w.genre && <span className="work-card__genre">{w.genre}</span>}
                {w.source_material && (
                  <p className="work-card__source">原作：{w.source_material}</p>
                )}
                {w.blurb && <p className="work-card__blurb">{w.blurb}</p>}
                <div className="work-card__foot">
                  <span className="work-card__date">{fmtDate(w.updated_ms)}</span>
                  <div className="work-card__actions">
                    <button
                      className="icon-btn"
                      title="编辑"
                      aria-label="编辑作品"
                      onClick={(e) => {
                        e.stopPropagation();
                        onEdit(w);
                      }}
                    >
                      <IconPencil size={14} />
                    </button>
                    <button
                      className="icon-btn icon-btn--danger"
                      title="删除"
                      aria-label="删除作品"
                      onClick={(e) => {
                        e.stopPropagation();
                        setDelTarget(w);
                      }}
                    >
                      <IconTrash size={14} />
                    </button>
                  </div>
                </div>
              </div>
            </article>
          ))}
        </div>
        </>
      )}

      {/* Create / edit drawer */}
      {creating && (
        <div className="library__overlay" onClick={closeEditor}>
          <div
            ref={editorSheetRef}
            className="library__sheet"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-modal="true"
            aria-label={editId ? "编辑作品" : "新建作品"}
            tabIndex={-1}
          >
            <header className="library__sheet-head">
              <h3>{editId ? "编辑作品" : "新建作品"}</h3>
              <button
                className="icon-btn"
                onClick={closeEditor}
                aria-label="关闭"
              >
                <IconClose size={16} />
              </button>
            </header>
            <div className="library__sheet-body">
              <label className="field">
                <span className="field__label">作品名称 *</span>
                <input
                  className="field__input"
                  value={draft.title}
                  onChange={(e) => setDraft((d) => ({ ...d, title: e.target.value }))}
                  placeholder="例如：北境剑歌"
                  data-autofocus
                />
              </label>
              <div className="field-row">
                <label className="field">
                  <span className="field__label">题材</span>
                  <input
                    className="field__input"
                    value={draft.genre}
                    onChange={(e) => setDraft((d) => ({ ...d, genre: e.target.value }))}
                    placeholder="玄幻 / 同人 / 都市…"
                  />
                </label>
                <label className="field">
                  <span className="field__label">原作（同人）</span>
                  <input
                    className="field__input"
                    value={draft.source_material}
                    onChange={(e) =>
                      setDraft((d) => ({ ...d, source_material: e.target.value }))
                    }
                    placeholder="如基于某部作品"
                  />
                </label>
              </div>
              <label className="field">
                <span className="field__label">简介</span>
                <textarea
                  className="field__input field__textarea"
                  value={draft.blurb}
                  onChange={(e) => setDraft((d) => ({ ...d, blurb: e.target.value }))}
                  placeholder="一句话故事梗概…"
                  rows={3}
                />
              </label>
              {draft.source_material.trim() && !editId && (
                <p className="library__hint">
                  <IconBrush size={13} />
                  创建后可在「知识库」一键联网填充《{draft.source_material.trim()}》的设定资料。
                </p>
              )}
            </div>
            <footer className="library__sheet-foot">
              <button
                className="btn btn--ghost"
                onClick={closeEditor}
              >
                取消
              </button>
              <button
                className="btn btn--primary"
                onClick={() => void submitCreate()}
                disabled={!draft.title.trim() || busy}
              >
                {busy ? <Spinner size={14} /> : <IconCheck size={16} />}
                {editId ? "保存" : "创建"}
              </button>
            </footer>
          </div>
        </div>
      )}

      <ConfirmModal
        open={!!delTarget || pendingBulk !== null}
        title={pendingBulk ? `删除选中的 ${pendingBulk.length} 部作品？` : "删除这部作品？"}
        sealChar="删"
        danger
        busy={deleting || bulkDeleting}
        confirmLabel="删除"
        body={
          <>
            {pendingBulk ? (
              <>
                将永久删除当前选中的 {pendingBulk.length} 部作品及其全部手稿、记忆与知识库。
                {bulkProgress && <><br />正在处理：{bulkProgress.done} / {bulkProgress.total}</>}
                <br />此操作不可撤销。
              </>
            ) : (
              <>将永久删除《{delTarget?.title}》及其全部手稿、记忆与知识库。<br />此操作不可撤销。</>
            )}
          </>
        }
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          if (!deleting && !bulkDeleting) {
            setDelTarget(null);
            setPendingBulk(null);
          }
        }}
      />
    </div>
  );
}
