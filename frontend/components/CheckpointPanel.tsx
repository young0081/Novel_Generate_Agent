"use client";

import { type CSSProperties, useCallback, useEffect, useState } from "react";

import Button from "@/components/Button";
import { useConnection } from "@/components/Connection";
import EmptyState from "@/components/EmptyState";
import { useToast } from "@/components/Toast";
import { invokeTool } from "@/lib/rpcClient";

export default function CheckpointPanel() {
  const toast = useToast();
  const { reportError } = useConnection();

  const [label, setLabel] = useState("");
  const [list, setList] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [restoreId, setRestoreId] = useState("");
  const [creating, setCreating] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [restoring, setRestoring] = useState(false);

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
      const r = await invokeTool("checkpoint_list", {});
      setList(r.content || "");
      setLoaded(true);
    } catch (e) {
      fail(e);
    } finally {
      setRefreshing(false);
    }
  }, [fail]);

  const create = useCallback(async () => {
    setCreating(true);
    try {
      const r = await invokeTool("checkpoint_create", { label: label || "未命名快照" });
      if (r.ok) {
        toast.success(`已创建：${r.summary ?? r.content}`);
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

  // Load the list once so the empty state is meaningful on first open.
  useEffect(() => {
    void refresh();
  }, [refresh]);

  const busy = creating || refreshing || restoring;

  return (
    <div>
      <div className="card" style={{ "--i": 0 } as CSSProperties}>
        <div className="card-head">
          <h3>创建快照</h3>
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
          <Button variant="ghost" onClick={refresh} loading={refreshing} disabled={busy}>
            刷新列表
          </Button>
        </div>
        {loaded &&
          (list ? (
            <pre className="output">{list}</pre>
          ) : (
            <EmptyState
              icon="💾"
              title="还没有任何快照"
              hint="写到一个满意的版本时，填个标签点「创建快照」存档。"
            />
          ))}
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
