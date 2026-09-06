"use client";

import { type CSSProperties, useCallback, useEffect, useState } from "react";

import BatchActions from "@/components/BatchActions";
import Button from "@/components/Button";
import { useConnection } from "@/components/Connection";
import EmptyState from "@/components/EmptyState";
import { useToast } from "@/components/Toast";
import { runBatch } from "@/lib/batch";
import { invokeTool } from "@/lib/rpcClient";

interface Checkpoint {
  id: string;
  label: string;
  file_count?: number;
  created_ms?: number;
}

interface CheckpointListData {
  checkpoints?: Checkpoint[];
}

function formatWhen(value: number | undefined): string {
  if (value == null) return "";
  const date = new Date(value > 1e12 ? value : value * 1000);
  return Number.isNaN(date.getTime())
    ? String(value)
    : date.toLocaleString("zh-CN", { dateStyle: "medium", timeStyle: "short" });
}

export default function CheckpointPanel() {
  const toast = useToast();
  const { reportError } = useConnection();

  const [label, setLabel] = useState("");
  const [list, setList] = useState("");
  const [checkpoints, setCheckpoints] = useState<Checkpoint[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [restoreId, setRestoreId] = useState("");
  const [creating, setCreating] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [manageMode, setManageMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ done: number; total: number } | null>(null);

  const fail = useCallback(
    (e: unknown) => {
      const message = e instanceof Error ? e.message : String(e);
      reportError(message);
      toast.error("错误：" + message);
    },
    [reportError, toast],
  );

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const r = await invokeTool<CheckpointListData>("checkpoint_list", {});
      if (!r.ok) {
        setList("");
        setCheckpoints([]);
        setLoaded(false);
        toast.error(r.content);
        return;
      }
      setList(r.content || "");
      setCheckpoints(Array.isArray(r.data?.checkpoints) ? r.data.checkpoints : []);
      setLoaded(true);
    } catch (e) {
      fail(e);
    } finally {
      setRefreshing(false);
    }
  }, [fail, toast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const valid = new Set(checkpoints.map((checkpoint) => checkpoint.id));
    setSelectedIds((current) => {
      const next = new Set([...current].filter((id) => valid.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [checkpoints]);

  const create = useCallback(async () => {
    setCreating(true);
    try {
      const r = await invokeTool("checkpoint_create", { label: label || "未命名快照" });
      if (r.ok) {
        toast.success(`已创建：${r.summary ?? r.content}`);
        setLabel("");
        await refresh();
      } else {
        toast.error(r.content);
      }
    } catch (e) {
      fail(e);
    } finally {
      setCreating(false);
    }
  }, [label, refresh, fail, toast]);

  const restore = useCallback(async () => {
    if (!restoreId) return;
    setRestoring(true);
    try {
      const r = await invokeTool("checkpoint_restore", { id: restoreId });
      if (r.ok) {
        toast.success(`已回滚到 ${restoreId}`);
      } else {
        toast.error(r.content);
      }
    } catch (e) {
      fail(e);
    } finally {
      setRestoring(false);
    }
  }, [restoreId, fail, toast]);

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const allSelected = checkpoints.length > 0 && checkpoints.every((checkpoint) => selectedIds.has(checkpoint.id));

  const toggleAll = useCallback(() => {
    setSelectedIds(allSelected ? new Set() : new Set(checkpoints.map((checkpoint) => checkpoint.id)));
  }, [allSelected, checkpoints]);

  const deleteOne = useCallback(
    async (checkpoint: Checkpoint) => {
      if (bulkDeleting || !window.confirm(`确定删除快照“${checkpoint.label || checkpoint.id}”吗？`)) return;
      setDeletingId(checkpoint.id);
      try {
        const r = await invokeTool("checkpoint_delete", { id: checkpoint.id });
        if (!r.ok) {
          toast.error(r.content || "删除失败");
          return;
        }
        setCheckpoints((current) => current.filter((item) => item.id !== checkpoint.id));
        setSelectedIds((current) => new Set([...current].filter((id) => id !== checkpoint.id)));
        toast.success(`已删除：${checkpoint.label || checkpoint.id}`);
      } catch (e) {
        fail(e);
      } finally {
        setDeletingId(null);
      }
    },
    [bulkDeleting, fail, toast],
  );

  const deleteSelected = useCallback(async () => {
    const selected = checkpoints.filter((checkpoint) => selectedIds.has(checkpoint.id));
    if (selected.length === 0 || bulkDeleting || deletingId !== null) return;
    if (!window.confirm(`确定删除选中的 ${selected.length} 个快照吗？此操作不可撤销。`)) return;
    setBulkDeleting(true);
    setBulkProgress({ done: 0, total: selected.length });
    try {
      const result = await runBatch(
        selected,
        async (checkpoint) => {
          const r = await invokeTool("checkpoint_delete", { id: checkpoint.id });
          if (!r.ok) throw new Error(r.content || `删除失败：${checkpoint.id}`);
        },
        (done, total) => setBulkProgress({ done, total }),
      );
      const completedIds = new Set(result.completed.map((checkpoint) => checkpoint.id));
      setCheckpoints((current) => current.filter((checkpoint) => !completedIds.has(checkpoint.id)));
      setSelectedIds((current) => new Set([...current].filter((id) => !completedIds.has(id))));
      if (result.failed.length === 0) {
        toast.success(`已删除 ${result.completed.length} 个快照`);
      } else {
        toast.error(`已删除 ${result.completed.length} 个，${result.failed.length} 个失败`);
      }
      await refresh();
    } finally {
      setBulkDeleting(false);
      setBulkProgress(null);
    }
  }, [checkpoints, selectedIds, bulkDeleting, deletingId, refresh, toast]);

  const busy = creating || refreshing || restoring || bulkDeleting || deletingId !== null;

  return (
    <div>
      <div className="card" style={{ "--i": 0 } as CSSProperties}>
        <div className="card-head">
          <h3>创建快照</h3>
          <div className="row">
            {checkpoints.length > 0 && (
              <Button
                variant="ghost"
                onClick={() => {
                  setManageMode((value) => !value);
                  setSelectedIds(new Set());
                }}
                disabled={busy}
              >
                {manageMode ? "完成" : "批量管理"}
              </Button>
            )}
            <Button variant="ghost" onClick={refresh} loading={refreshing} disabled={busy}>
              刷新列表
            </Button>
          </div>
        </div>
        <div className="row">
          <input
            className="grow"
            placeholder="快照标签，例如：第三章初稿"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !busy && void create()}
          />
          <Button onClick={create} loading={creating} disabled={busy}>
            创建快照
          </Button>
        </div>
        {manageMode && checkpoints.length > 0 && (
          <BatchActions
            selectedCount={selectedIds.size}
            totalCount={checkpoints.length}
            allSelected={allSelected}
            busy={bulkDeleting || deletingId !== null}
            progress={bulkProgress}
            onToggleAll={toggleAll}
            onClear={() => setSelectedIds(new Set())}
            onDelete={() => void deleteSelected()}
            label="批量管理快照"
          />
        )}
        {loaded &&
          (checkpoints.length > 0 ? (
            <div className="batch-list">
              {checkpoints.map((checkpoint) => (
                <div className={`batch-row${selectedIds.has(checkpoint.id) ? " is-selected" : ""}`} key={checkpoint.id}>
                  {manageMode && (
                    <input
                      type="checkbox"
                      className="batch-select"
                      checked={selectedIds.has(checkpoint.id)}
                      onChange={() => toggleSelected(checkpoint.id)}
                      aria-label={`选择快照：${checkpoint.label || checkpoint.id}`}
                      disabled={busy}
                    />
                  )}
                  <div className="batch-row__main">
                    <div className="batch-row__title"><strong>{checkpoint.label || "未命名快照"}</strong></div>
                    <div className="batch-row__meta">
                      {checkpoint.id}
                      {checkpoint.file_count != null && ` · ${checkpoint.file_count} 个文件`}
                      {checkpoint.created_ms != null && ` · ${formatWhen(checkpoint.created_ms)}`}
                    </div>
                  </div>
                  <div className="batch-row__actions">
                    <button
                      type="button"
                      className="batch-row__link"
                      onClick={() => setRestoreId(checkpoint.id)}
                      disabled={busy}
                    >
                      选作回滚
                    </button>
                    <button
                      type="button"
                      className="batch-row__delete"
                      onClick={() => void deleteOne(checkpoint)}
                      disabled={busy}
                    >
                      {deletingId === checkpoint.id ? "删除中…" : "删除"}
                    </button>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <EmptyState
              icon="💾"
              title="还没有任何快照"
              hint="写到一个满意的版本时，填个标签点「创建快照」存档。"
            />
          ))}
        {loaded && checkpoints.length === 0 && list && list !== "(no checkpoints)" && (
          <pre className="output">{list}</pre>
        )}
      </div>

      <div className="card" style={{ "--i": 1 } as CSSProperties}>
        <div className="card-head">
          <h3>回滚到快照</h3>
        </div>
        <div className="row">
          <input
            className="grow"
            placeholder="checkpoint id，例如 ckpt_..."
            value={restoreId}
            onChange={(e) => setRestoreId(e.target.value)}
          />
          <Button variant="danger" onClick={restore} loading={restoring} disabled={busy || !restoreId}>
            回滚
          </Button>
        </div>
      </div>
    </div>
  );
}
