// SessionsDrawer — a slide-in right drawer listing all persisted sessions
// (创作 / 探讨 / 策划), allowing the user to resume or delete them.

import { useCallback, useEffect, useRef, useState } from "react";
import { SkeletonGrid } from "./Skeleton";
import EmptyState from "./EmptyState";
import ConfirmModal from "./ConfirmModal";
import BatchActions from "./BatchActions";
import {
  IconRefresh,
  IconBrush,
  IconChat,
  IconTrash,
  IconScroll,
  IconRestore,
  WinCloseIcon,
} from "./icons";
import { describeError } from "../lib/core";
import { useToast } from "./Toast";
import {
  listSessions,
  deleteSession,
  KIND_LABEL,
  formatTime,
  type SessionSummary,
} from "../lib/sessions";
import { useDialogFocus, useLayerPresence } from "../lib/dialogLayer";
import { runBatch } from "../lib/batch";
import { sessionResumeTarget, type SessionResumeMode } from "../lib/sessionResume";

interface SessionsDrawerProps {
  open: boolean;
  onClose: () => void;
  /** Called when the user continues a supported session. */
  onResume: (kind: SessionResumeMode, sessionId: string) => void;
}

export default function SessionsDrawer({ open, onClose, onResume }: SessionsDrawerProps) {
  const toast = useToast();
  const [items, setItems] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<SessionSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [manageMode, setManageMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [pendingBulk, setPendingBulk] = useState<SessionSummary[] | null>(null);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ done: number; total: number } | null>(null);
  const drawerRef = useRef<HTMLElement>(null);
  const { mounted, closing } = useLayerPresence(open);
  useDialogFocus(open, drawerRef, onClose, pending !== null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setItems(await listSessions());
    } catch (e) {
      setError(describeError(e));
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open) void load();
  }, [open, load]);

  useEffect(() => {
    const valid = new Set(items.map((item) => item.id));
    setSelectedIds((current) => {
      const next = new Set([...current].filter((id) => valid.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [items]);

  const resume = useCallback(
    (s: SessionSummary) => {
      const target = sessionResumeTarget(s.kind);
      if (!target) return;
      onResume(target.mode, s.id);
      onClose(); // close drawer after resuming
    },
    [onResume, onClose],
  );

  const confirmDelete = useCallback(async () => {
    if (pendingBulk) {
      setBulkDeleting(true);
      setBulkProgress({ done: 0, total: pendingBulk.length });
      let latest: SessionSummary[] | null = null;
      try {
        const result = await runBatch(
          pendingBulk,
          async (session) => {
            latest = await deleteSession(session.id);
          },
          (done, total) => setBulkProgress({ done, total }),
        );
        if (latest) setItems(latest);
        else {
          const completedIds = new Set(result.completed.map((session) => session.id));
          setItems((current) => current.filter((session) => !completedIds.has(session.id)));
        }
        const completedIds = new Set(result.completed.map((session) => session.id));
        setSelectedIds((current) => new Set([...current].filter((id) => !completedIds.has(id))));
        if (result.failed.length === 0) {
          toast.ok(`已删除 ${result.completed.length} 个会话`);
        } else {
          toast.err(`已删除 ${result.completed.length} 个，${result.failed.length} 个删除失败`);
        }
        setPendingBulk(null);
      } finally {
        setBulkDeleting(false);
        setBulkProgress(null);
      }
      return;
    }
    if (!pending) return;
    setDeleting(true);
    try {
      const next = await deleteSession(pending.id);
      setItems(next);
      toast.ok("已删除该会话");
      setPending(null);
    } catch (e) {
      toast.err(`删除失败：${describeError(e)}`);
    } finally {
      setDeleting(false);
    }
  }, [pending, pendingBulk, toast]);

  const allSelected = items.length > 0 && items.every((item) => selectedIds.has(item.id));

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const toggleAll = useCallback(() => {
    setSelectedIds(allSelected ? new Set() : new Set(items.map((item) => item.id)));
  }, [allSelected, items]);

  if (!mounted) return null;

  return (
    <>
      <div className={`drawer-overlay${closing ? " is-closing" : ""}`} onClick={onClose} />
      <aside
        ref={drawerRef}
        className={`drawer sessions-drawer${closing ? " is-closing" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label="会话历史"
        tabIndex={-1}
      >
        <header className="drawer__head">
          <div>
            <h2 className="drawer__title">会话</h2>
            <p className="drawer__subtitle">历次策划、创作与探讨的存档 · 挑一个继续</p>
          </div>
          <div className="drawer__head-actions">
            <button
              className={`btn btn--ghost btn--sm${manageMode ? " is-active" : ""}`}
              onClick={() => {
                setManageMode((value) => !value);
                setSelectedIds(new Set());
              }}
              title="批量管理会话"
              aria-pressed={manageMode}
            >
              {manageMode ? "完成" : "批量管理"}
            </button>
            <button
              data-autofocus
              className="btn btn--ghost btn--icon"
              onClick={() => void load()}
              title="刷新"
              aria-label="刷新"
            >
              <IconRefresh size={16} />
            </button>
            <button
              className="drawer__close"
              onClick={onClose}
              title="关闭"
              aria-label="关闭"
            >
              <WinCloseIcon />
            </button>
          </div>
        </header>

        {manageMode && items.length > 0 && (
          <BatchActions
            selectedCount={selectedIds.size}
            totalCount={items.length}
            allSelected={allSelected}
            onToggleAll={toggleAll}
            onClear={() => setSelectedIds(new Set())}
          >
            <button
              type="button"
              className="btn btn--danger btn--sm"
              disabled={selectedIds.size === 0 || bulkDeleting}
              onClick={() => setPendingBulk(items.filter((item) => selectedIds.has(item.id)))}
            >
              <IconTrash size={13} /> 删除已选
            </button>
          </BatchActions>
        )}

        <div className="drawer__body">
          {loading ? (
            <SkeletonGrid count={6} />
          ) : error ? (
            <div className="banner banner--warn">{error}</div>
          ) : items.length === 0 ? (
            <EmptyState
              title="暂无会话"
              text="到「策划」生成设定、「创作」写一段，或到「探讨」聊一聊，会话会存到这里，方便日后继续。"
            />
          ) : (
            <div className="sessions-list">
              {items.map((s) => {
                const isDiscuss = s.kind === "discuss";
                const target = sessionResumeTarget(s.kind);
                return (
                  <article className={`session-card${manageMode ? " is-manage" : ""}`} key={s.id}>
                    {manageMode && (
                      <input
                        type="checkbox"
                        className="batch-select"
                        checked={selectedIds.has(s.id)}
                        onChange={() => toggleSelected(s.id)}
                        aria-label={`选择会话：${s.title || "未命名"}`}
                      />
                    )}
                    <span
                      className={`chip session-card__kind ${isDiscuss ? "chip--jade" : "chip--accent"}`}
                    >
                      {isDiscuss ? <IconChat size={11} /> : <IconBrush size={11} />}
                      {KIND_LABEL[s.kind] ?? s.kind}
                    </span>
                    <h3 className="session-card__title">{s.title || "（未命名）"}</h3>
                    <p className="session-card__preview">
                      {s.preview || <span className="muted">（暂无内容）</span>}
                    </p>
                    <div className="session-card__meta">
                      <IconScroll size={11} />
                      {s.messages} 条 · {formatTime(s.updated_ms)}
                    </div>
                    <div className="session-card__actions">
                      {target && (
                        <button
                          className="btn btn--primary btn--sm"
                          onClick={() => resume(s)}
                          title={target.label}
                        >
                          <IconRestore size={14} />
                          {target.label}
                        </button>
                      )}
                      <button
                        className="btn btn--ghost btn--icon"
                        onClick={() => setPending(s)}
                        title="删除会话"
                        aria-label="删除会话"
                      >
                        <IconTrash size={15} />
                      </button>
                    </div>
                  </article>
                );
              })}
            </div>
          )}
        </div>
      </aside>

      <ConfirmModal
        open={pending !== null || pendingBulk !== null}
        title={pendingBulk ? `删除选中的 ${pendingBulk.length} 个会话？` : "删除这个会话？"}
        sealChar="删"
        danger
        busy={deleting || bulkDeleting}
        confirmLabel="删除"
        body={
          <>
            {pendingBulk ? (
              <>
                将永久删除当前选中的 {pendingBulk.length} 个会话及其对话记录。
                {bulkProgress && <><br />正在处理：{bulkProgress.done} / {bulkProgress.total}</>}
                <br />此操作不可撤销。
              </>
            ) : (
              <>将永久删除会话「{pending?.title || "（未命名）"}」及其全部对话记录，此操作不可撤销。</>
            )}
          </>
        }
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          if (!deleting && !bulkDeleting) {
            setPending(null);
            setPendingBulk(null);
          }
        }}
      />
    </>
  );
}
