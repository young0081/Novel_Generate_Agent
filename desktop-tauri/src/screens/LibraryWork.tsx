// 书库 — the work library. A shelf of every novel project, each fully isolated.
// Create new works, switch between them, edit metadata, and delete. The active
// work drives the whole rest of the app (manuscript, memory, knowledge).

import { useCallback, useEffect, useState } from "react";
import { Spinner } from "../components/Spinner";
import ConfirmModal from "../components/ConfirmModal";
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
  const { works, current, loading, refresh, switchTo } = useWork();
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState<DraftForm>(EMPTY_DRAFT);
  const [busy, setBusy] = useState(false);
  const [editId, setEditId] = useState<string | null>(null);
  const [delTarget, setDelTarget] = useState<WorkSummary | null>(null);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    void refresh();
  }, [refresh]);

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
  }, [delTarget, toast, refresh]);

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
      </header>

      {loading ? (
        <div className="library__loading">
          <Spinner size={28} />
          <span>正在打开书库…</span>
        </div>
      ) : works.length === 0 ? (
        <div className="library__empty">
          <div className="library__empty-icon">
            <IconScroll size={34} />
          </div>
          <h3>书库尚空</h3>
          <p>创建你的第一部作品，开启创作之旅。</p>
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
      ) : (
        <div className="library__grid">
          {works.map((w) => (
            <article
              key={w.id}
              className={`work-card${w.active ? " is-active" : ""}`}
              onClick={() => void onSwitch(w.id)}
            >
              <div className="work-card__spine" />
              <div className="work-card__body">
                <div className="work-card__top">
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
      )}

      {/* Create / edit drawer */}
      {creating && (
        <div className="library__overlay" onClick={() => !busy && setCreating(false)}>
          <div
            className="library__sheet"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-label={editId ? "编辑作品" : "新建作品"}
          >
            <header className="library__sheet-head">
              <h3>{editId ? "编辑作品" : "新建作品"}</h3>
              <button
                className="icon-btn"
                onClick={() => !busy && setCreating(false)}
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
                  autoFocus
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
                onClick={() => !busy && setCreating(false)}
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
        open={!!delTarget}
        title="删除这部作品？"
        sealChar="删"
        danger
        busy={deleting}
        confirmLabel="删除"
        body={
          <>
            将永久删除《{delTarget?.title}》及其全部手稿、记忆与知识库。
            <br />
            此操作不可撤销。
          </>
        }
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          if (!deleting) setDelTarget(null);
        }}
      />
    </div>
  );
}
