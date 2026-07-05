// 会话 — the session library. Lists every persisted 创作 / 探讨 session
// (newest first) so the author can pick one up and continue, or prune old ones.
// "继续" hands the session id to the matching screen (创作 / 探讨) to resume.

import { useCallback, useEffect, useState } from "react";
import Panel from "../components/Panel";
import { SkeletonGrid } from "../components/Skeleton";
import EmptyState from "../components/EmptyState";
import ConfirmModal from "../components/ConfirmModal";
import {
  IconRefresh,
  IconBrush,
  IconChat,
  IconTrash,
  IconScroll,
  IconRestore,
} from "../components/icons";
import { describeError } from "../lib/core";
import { useToast } from "../components/Toast";
import {
  listSessions,
  deleteSession,
  KIND_LABEL,
  formatTime,
  type SessionSummary,
} from "../lib/sessions";
import type { ScreenId } from "../lib/screens";

interface SessionsScreenProps {
  onNavigate?: (id: ScreenId) => void;
  onResume: (screen: ScreenId, id: string) => void;
}

export default function SessionsScreen({ onResume }: SessionsScreenProps) {
  const toast = useToast();
  const [items, setItems] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(true);
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
    void load();
  }, [load]);

  const resume = useCallback(
    (s: SessionSummary) => {
      const target: ScreenId = s.kind === "discuss" ? "chat" : "studio";
      onResume(target, s.id);
    },
    [onResume],
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

  const headerActions = (
    <button
      className="btn btn--ghost btn--icon"
      onClick={() => void load()}
      title="刷新"
      aria-label="刷新"
    >
      <IconRefresh size={16} />
    </button>
  );

  return (
    <Panel
      title="会话"
      en="Sessions"
      subtitle="历次创作与探讨的存档 · 挑一个接着往下写"
      actions={headerActions}
    >
      <div className="scroll-area">
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
          <div className="slip-grid">
            {items.map((s) => {
              const isDiscuss = s.kind === "discuss";
              return (
                <article className="slip session-slip" key={s.id}>
                  <span className="slip__kind">
                    <span
                      className={`chip ${isDiscuss ? "chip--jade" : "chip--accent"}`}
                    >
                      {isDiscuss ? <IconChat size={11} /> : <IconBrush size={11} />}
                      {KIND_LABEL[s.kind] ?? s.kind}
                    </span>
                  </span>
                  <div className="slip__top">
                    <h3 className="slip__title">{s.title || "（未命名）"}</h3>
                  </div>
                  <p className="slip__summary">
                    {s.preview || <span className="muted">（暂无内容）</span>}
                  </p>
                  <div className="session-slip__meta">
                    <span className="session-slip__time">
                      <IconScroll size={11} />
                      {s.messages} 条 · {formatTime(s.updated_ms)}
                    </span>
                  </div>
                  <div className="session-slip__foot">
                    <button
                      className="btn btn--primary btn--sm session-slip__resume"
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

      <ConfirmModal
        open={pending !== null}
        title="删除这个会话？"
        sealChar="删"
        danger
        busy={deleting}
        confirmLabel="删除"
        body={
          <>
            将永久删除会话「{pending?.title || "（未命名）"}」及其全部对话记录，
            此操作不可撤销。
          </>
        }
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          if (!deleting) setPending(null);
        }}
      />
    </Panel>
  );
}
