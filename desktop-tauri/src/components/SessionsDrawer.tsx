// SessionsDrawer — a slide-in right drawer listing all persisted sessions
// (创作 / 探讨), allowing the user to resume or delete them.

import { useCallback, useEffect, useState } from "react";
import { SkeletonGrid } from "./Skeleton";
import EmptyState from "./EmptyState";
import ConfirmModal from "./ConfirmModal";
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

interface SessionsDrawerProps {
  open: boolean;
  onClose: () => void;
  /** Called when user clicks "继续创作/探讨" with (kind, sessionId) */
  onResume: (kind: "discuss" | "studio", sessionId: string) => void;
}

export default function SessionsDrawer({ open, onClose, onResume }: SessionsDrawerProps) {
  const toast = useToast();
  const [items, setItems] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<SessionSummary | null>(null);
  const [deleting, setDeleting] = useState(false);

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

  const resume = useCallback(
    (s: SessionSummary) => {
      const kind = s.kind === "discuss" ? "discuss" : "studio";
      onResume(kind, s.id);
      onClose(); // close drawer after resuming
    },
    [onResume, onClose],
  );

  const confirmDelete = useCallback(async () => {
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
  }, [pending, toast]);

  if (!open) return null;

  return (
    <>
      <div className="drawer-overlay" onClick={onClose} />
      <aside className="drawer sessions-drawer">
        <header className="drawer__head">
          <div>
            <h2 className="drawer__title">会话</h2>
            <p className="drawer__subtitle">历次创作与探讨的存档 · 挑一个接着往下写</p>
          </div>
          <div className="drawer__head-actions">
            <button
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

        <div className="drawer__body">
          {loading ? (
            <SkeletonGrid count={6} />
          ) : error ? (
            <div className="banner banner--warn">{error}</div>
          ) : items.length === 0 ? (
            <EmptyState
              title="暂无会话"
              text="到「创作」让 AI 写一段，或到「探讨」聊一聊——你们的每一次会话都会存到这里，方便日后接着写。"
            />
          ) : (
            <div className="sessions-list">
              {items.map((s) => {
                const isDiscuss = s.kind === "discuss";
                return (
                  <article className="session-card" key={s.id}>
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
                      <button
                        className="btn btn--primary btn--sm"
                        onClick={() => resume(s)}
                        title={isDiscuss ? "继续探讨" : "继续创作"}
                      >
                        <IconRestore size={14} />
                        {isDiscuss ? "继续探讨" : "继续创作"}
                      </button>
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
        open={pending !== null}
        title="删除这个会话？"
        sealChar="删"
        danger
        busy={deleting}
        confirmLabel="删除"
        body={
          <>
            将永久删除会话「{pending?.title || "（未命名）"}」及其全部对话记录，此操作不可撤销。
          </>
        }
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          if (!deleting) setPending(null);
        }}
      />
    </>
  );
}
