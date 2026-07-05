// MemoryDrawer — a slide-in right drawer for browsing the memory library
// (人物 / 伏笔 / 设定). Tabs switch between memory kinds, with search and delete.

import { useCallback, useEffect, useState } from "react";
import { SkeletonGrid } from "./Skeleton";
import EmptyState from "./EmptyState";
import ConfirmModal from "./ConfirmModal";
import {
  IconRefresh,
  IconSearch,
  IconTrash,
  IconTag,
  WinCloseIcon,
  IconUser,
  IconThread,
  IconScroll,
  IconMountain,
} from "./icons";
import { describeError, invokeTool } from "../lib/core";
import { useToast } from "./Toast";
import {
  KIND_LABEL,
  type MemoryHit,
  type MemoryKind,
  type MemoryRecallData,
  type MemoryListData,
} from "../lib/memory";

interface MemoryDrawerProps {
  open: boolean;
  onClose: () => void;
}

const KINDS: MemoryKind[] = ["character", "worldbuilding", "outline", "foreshadow", "setting"];
const KIND_ICONS: Record<MemoryKind, typeof IconUser> = {
  character:    IconUser,
  worldbuilding: IconMountain,
  outline:      IconScroll,
  foreshadow:   IconThread,
  setting:      IconScroll,
  plot:         IconThread,
  dialogue:     IconUser,
  lore:         IconScroll,
  other:        IconTag,
};

export default function MemoryDrawer({ open, onClose }: MemoryDrawerProps) {
  const toast = useToast();
  const [activeKind, setActiveKind] = useState<MemoryKind>("character");
  const [hits, setHits] = useState<MemoryHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState<MemoryHit | null>(null);
  const [deleting, setDeleting] = useState(false);

  const load = useCallback(
    async (q: string) => {
      setLoading(true);
      setError(null);
      try {
        if (q.trim()) {
          // Search mode: use memory_recall (requires query)
          const result = await invokeTool<MemoryRecallData>("memory_recall", {
            query: q.trim(),
            k: 50,
          });
          setHits(result.data.hits ?? []);
        } else {
          // Browse mode: use memory_list (no query required, returns all)
          const result = await invokeTool<MemoryListData>("memory_list", {
            limit: 500,
          });
          setHits(result.data.entries ?? []);
        }
      } catch (e) {
        setError(describeError(e));
        setHits([]);
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  useEffect(() => {
    if (open) void load(query);
  }, [open, activeKind]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleSearch = useCallback(() => {
    void load(query);
  }, [query, load]);

  const confirmDelete = useCallback(async () => {
    if (!pending) return;
    setDeleting(true);
    try {
      await invokeTool("memory_delete", { id: pending.id });
      setHits((prev) => prev.filter((h) => h.id !== pending.id));
      toast.ok("已删除该记忆");
      setPending(null);
    } catch (e) {
      toast.err(`删除失败：${describeError(e)}`);
    } finally {
      setDeleting(false);
    }
  }, [pending, toast]);

  if (!open) return null;

  // Filter hits by active kind for display
  const visibleHits = hits.filter((h) => h.kind === activeKind);

  return (
    <>
      <div className="drawer-overlay" onClick={onClose} />
      <aside className="drawer memory-drawer">
        <header className="drawer__head">
          <div>
            <h2 className="drawer__title">记忆库</h2>
            <p className="drawer__subtitle">作品的人物、伏笔与设定存档</p>
          </div>
          <div className="drawer__head-actions">
            <button
              className="btn btn--ghost btn--icon"
              onClick={() => void load(query)}
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

        <div className="memory-drawer__tabs">
          {KINDS.map((k) => {
            const Icon = KIND_ICONS[k];
            const count = hits.filter((h) => h.kind === k).length;
            return (
              <button
                key={k}
                className={`memory-drawer__tab${activeKind === k ? " is-active" : ""}`}
                onClick={() => setActiveKind(k)}
              >
                <Icon size={14} />
                {KIND_LABEL[k]}
                {count > 0 && <span className="memory-drawer__badge">{count}</span>}
              </button>
            );
          })}
        </div>

        <div className="memory-drawer__search">
          <input
            type="text"
            className="input"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSearch()}
            placeholder="搜索关键词…"
          />
          <button
            className="btn btn--ghost btn--icon"
            onClick={handleSearch}
            title="搜索"
            aria-label="搜索"
          >
            <IconSearch size={16} />
          </button>
        </div>

        <div className="drawer__body">
          {loading ? (
            <SkeletonGrid count={6} />
          ) : error ? (
            <div className="banner banner--warn">{error}</div>
          ) : visibleHits.length === 0 ? (
            <EmptyState
              title={`暂无${KIND_LABEL[activeKind]}`}
              text="在策划屏生成设定，或到创作屏让 AI 写作时自动提取人物与伏笔。"
            />
          ) : (
            <div className="memory-list">
              {visibleHits.map((h) => (
                <article className="memory-card" key={h.id}>
                  <div className="memory-card__head">
                    <h3 className="memory-card__title">{h.title}</h3>
                    <button
                      className="btn btn--ghost btn--icon"
                      onClick={() => setPending(h)}
                      title="删除"
                      aria-label="删除"
                    >
                      <IconTrash size={14} />
                    </button>
                  </div>
                  {h.summary && <p className="memory-card__summary">{h.summary}</p>}
                  {h.tags.length > 0 && (
                    <div className="memory-card__tags">
                      <IconTag size={11} />
                      {h.tags.join(" · ")}
                    </div>
                  )}
                </article>
              ))}
            </div>
          )}
        </div>
      </aside>

      <ConfirmModal
        open={pending !== null}
        title="删除这条记忆？"
        sealChar="删"
        danger
        busy={deleting}
        confirmLabel="删除"
        body={
          <>
            将永久删除「{pending?.title || "（未命名）"}」，此操作不可撤销。
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
