// MemoryDrawer — a slide-in right drawer for browsing the memory library
// (人物 / 伏笔 / 设定). Tabs switch between memory kinds, with search and delete.

import { useCallback, useEffect, useRef, useState } from "react";
import { SkeletonGrid } from "./Skeleton";
import EmptyState from "./EmptyState";
import ConfirmModal from "./ConfirmModal";
import BatchActions from "./BatchActions";
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
import { describeError, invokeTool, requireToolSuccess } from "../lib/core";
import { useToast } from "./Toast";
import {
  KIND_LABEL,
  type MemoryHit,
  type MemoryKind,
  type MemoryRecallData,
  type MemoryListData,
} from "../lib/memory";
import { useDialogFocus, useLayerPresence } from "../lib/dialogLayer";
import { runBatch } from "../lib/batch";

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
  const [manageMode, setManageMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [pendingBulk, setPendingBulk] = useState<MemoryHit[] | null>(null);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ done: number; total: number } | null>(null);
  const drawerRef = useRef<HTMLElement>(null);
  const { mounted, closing } = useLayerPresence(open);
  useDialogFocus(open, drawerRef, onClose, pending !== null);

  const load = useCallback(
    async (q: string) => {
      setLoading(true);
      setError(null);
      try {
        if (q.trim()) {
          // Search mode: use memory_recall (requires query)
          const result = requireToolSuccess(
            await invokeTool<MemoryRecallData>("memory_recall", {
              query: q.trim(),
              k: 50,
            }),
            "检索记忆失败",
          );
          setHits(result.data.hits ?? []);
        } else {
          // Browse mode: use memory_list (no query required, returns all)
          const result = requireToolSuccess(
            await invokeTool<MemoryListData>("memory_list", {
              limit: 500,
            }),
            "读取记忆库失败",
          );
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
  }, [open, load]); // query is submitted explicitly; typing must not fire requests

  useEffect(() => {
    const valid = new Set(hits.map((hit) => hit.id));
    setSelectedIds((current) => {
      const next = new Set([...current].filter((id) => valid.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [hits]);

  // A selection belongs to the visible category. Do not carry hidden rows
  // into a different tab where the count and bulk-delete target would be
  // surprising.
  useEffect(() => {
    setSelectedIds(new Set());
  }, [activeKind]);

  const handleSearch = useCallback(() => {
    void load(query);
  }, [query, load]);

  const confirmDelete = useCallback(async () => {
    if (pendingBulk) {
      setBulkDeleting(true);
      setBulkProgress({ done: 0, total: pendingBulk.length });
      try {
        const result = await runBatch(
          pendingBulk,
          async (hit) => {
            requireToolSuccess(
              await invokeTool("memory_delete", { id: hit.id }),
              "删除记忆失败",
            );
          },
          (done, total) => setBulkProgress({ done, total }),
        );
        const completedIds = new Set(result.completed.map((hit) => hit.id));
        setHits((prev) => prev.filter((hit) => !completedIds.has(hit.id)));
        setSelectedIds((prev) => new Set([...prev].filter((id) => !completedIds.has(id))));
        if (result.failed.length === 0) {
          toast.ok(`已删除 ${result.completed.length} 条记忆`);
        } else {
          toast.err(`已删除 ${result.completed.length} 条，${result.failed.length} 条删除失败`);
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
      requireToolSuccess(
        await invokeTool("memory_delete", { id: pending.id }),
        "删除记忆失败",
      );
      setHits((prev) => prev.filter((h) => h.id !== pending.id));
      setSelectedIds((prev) => {
        const next = new Set(prev);
        next.delete(pending.id);
        return next;
      });
      toast.ok("已删除该记忆");
      setPending(null);
    } catch (e) {
      toast.err(`删除失败：${describeError(e)}`);
    } finally {
      setDeleting(false);
    }
  }, [pending, pendingBulk, toast]);

  const visibleHits = hits.filter((h) => h.kind === activeKind);
  const allVisibleSelected = visibleHits.length > 0 && visibleHits.every((hit) => selectedIds.has(hit.id));
  const visibleSelectedCount = visibleHits.filter((hit) => selectedIds.has(hit.id)).length;

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const toggleAllVisible = useCallback(() => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (allVisibleSelected) visibleHits.forEach((hit) => next.delete(hit.id));
      else visibleHits.forEach((hit) => next.add(hit.id));
      return next;
    });
  }, [allVisibleSelected, visibleHits]);

  if (!mounted) return null;

  return (
    <>
      <div className={`drawer-overlay${closing ? " is-closing" : ""}`} onClick={onClose} />
      <aside
        ref={drawerRef}
        className={`drawer memory-drawer${closing ? " is-closing" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label="记忆库"
        tabIndex={-1}
      >
        <header className="drawer__head">
          <div>
            <h2 className="drawer__title">记忆库</h2>
            <p className="drawer__subtitle">作品的人物、伏笔与设定存档</p>
          </div>
          <div className="drawer__head-actions">
            <button
              className={`btn btn--ghost btn--sm${manageMode ? " is-active" : ""}`}
              onClick={() => {
                setManageMode((value) => !value);
                setSelectedIds(new Set());
              }}
              title="批量管理记忆"
              aria-pressed={manageMode}
            >
              {manageMode ? "完成" : "批量管理"}
            </button>
            <button
              data-autofocus
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

        {manageMode && visibleHits.length > 0 && (
          <BatchActions
            selectedCount={visibleSelectedCount}
            totalCount={visibleHits.length}
            allSelected={allVisibleSelected}
            onToggleAll={toggleAllVisible}
            onClear={() => setSelectedIds(new Set())}
          >
            <button
              type="button"
              className="btn btn--danger btn--sm"
              disabled={visibleSelectedCount === 0 || bulkDeleting}
              onClick={() => setPendingBulk(visibleHits.filter((hit) => selectedIds.has(hit.id)))}
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
          ) : visibleHits.length === 0 ? (
            <EmptyState
              title={`暂无${KIND_LABEL[activeKind]}`}
              text="在策划屏生成设定，或到创作屏让 AI 写作时自动提取人物与伏笔。"
            />
          ) : (
              <div className="memory-list">
              {visibleHits.map((h) => (
                <article className={`memory-card${manageMode ? " is-manage" : ""}`} key={h.id}>
                  <div className="memory-card__head">
                    {manageMode && (
                      <input
                        type="checkbox"
                        className="batch-select"
                        checked={selectedIds.has(h.id)}
                        onChange={() => toggleSelected(h.id)}
                        aria-label={`选择记忆：${h.title}`}
                      />
                    )}
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
        open={pending !== null || pendingBulk !== null}
        title={pendingBulk ? `删除选中的 ${pendingBulk.length} 条记忆？` : "删除这条记忆？"}
        sealChar="删"
        danger
        busy={deleting || bulkDeleting}
        confirmLabel="删除"
        body={
          <>
            {pendingBulk ? (
              <>
                将永久删除当前选中的 {pendingBulk.length} 条记忆。
                {bulkProgress && <><br />正在处理：{bulkProgress.done} / {bulkProgress.total}</>}
                <br />此操作不可撤销。
              </>
            ) : (
              <>将永久删除「{pending?.title || "（未命名）"}」，此操作不可撤销。</>
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
