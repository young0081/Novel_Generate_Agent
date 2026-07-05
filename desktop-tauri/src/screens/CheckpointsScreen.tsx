// 快照 — create / list / restore workspace checkpoints.

import { useCallback, useEffect, useMemo, useState } from "react";
import Panel from "../components/Panel";
import { LoadingBlock, Spinner } from "../components/Spinner";
import EmptyState from "../components/EmptyState";
import ConfirmModal from "../components/ConfirmModal";
import {
  IconPlus,
  IconRefresh,
  IconRestore,
  IconClock,
  IconTrash,
} from "../components/icons";
import { invokeTool, describeError } from "../lib/core";
import { useToast } from "../components/Toast";

interface Checkpoint {
  id: string;
  label: string;
  created_at?: string | number | null;
  [k: string]: unknown;
}

interface CheckpointListData {
  checkpoints?: Checkpoint[];
  items?: Checkpoint[];
  [k: string]: unknown;
}

function pickList(data: CheckpointListData | null): Checkpoint[] {
  if (!data) return [];
  if (Array.isArray(data.checkpoints)) return data.checkpoints;
  if (Array.isArray(data.items)) return data.items;
  // some cores return the array directly under data
  if (Array.isArray(data as unknown)) return data as unknown as Checkpoint[];
  return [];
}

function formatWhen(v: Checkpoint["created_at"]): string {
  if (v == null) return "";
  let d: Date;
  if (typeof v === "number") {
    d = new Date(v > 1e12 ? v : v * 1000);
  } else {
    const n = Number(v);
    d = Number.isFinite(n) && v.trim() !== "" ? new Date(n > 1e12 ? n : n * 1000) : new Date(v);
  }
  if (Number.isNaN(d.getTime())) return String(v);
  return d.toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default function CheckpointsScreen() {
  const toast = useToast();
  const [list, setList] = useState<Checkpoint[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [label, setLabel] = useState("");
  const [creating, setCreating] = useState(false);

  const [target, setTarget] = useState<Checkpoint | null>(null);
  const [restoring, setRestoring] = useState(false);

  const [delTarget, setDelTarget] = useState<Checkpoint | null>(null);
  const [deleting, setDeleting] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await invokeTool<CheckpointListData>("checkpoint_list", {});
      if (!res.ok) {
        setError(res.content || "无法读取快照列表");
        setList([]);
        return;
      }
      setList(pickList(res.data));
    } catch (e) {
      setError(describeError(e));
      setList([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const create = useCallback(async () => {
    const text = label.trim() || `快照 · ${new Date().toLocaleString("zh-CN")}`;
    setCreating(true);
    try {
      const res = await invokeTool("checkpoint_create", { label: text });
      if (!res.ok) {
        toast.err(res.content || "创建快照失败");
        return;
      }
      toast.ok("已创建快照");
      setLabel("");
      await refresh();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setCreating(false);
    }
  }, [label, refresh, toast]);

  const restore = useCallback(async () => {
    if (!target) return;
    setRestoring(true);
    try {
      const res = await invokeTool("checkpoint_restore", { id: target.id });
      if (!res.ok) {
        toast.err(res.content || "回溯失败");
        return;
      }
      toast.ok("已回溯到该快照");
      setTarget(null);
      await refresh();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setRestoring(false);
    }
  }, [target, refresh, toast]);

  const removeCp = useCallback(async () => {
    if (!delTarget) return;
    setDeleting(true);
    try {
      const res = await invokeTool("checkpoint_delete", { id: delTarget.id });
      if (!res.ok) {
        toast.err(res.content || "删除失败");
        return;
      }
      toast.ok("已删除该快照");
      setDelTarget(null);
      await refresh();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setDeleting(false);
    }
  }, [delTarget, refresh, toast]);

  const ordered = useMemo(() => list.slice().reverse(), [list]);

  const headerActions = (
    <button
      className="btn btn--ghost btn--icon"
      onClick={() => void refresh()}
      title="刷新"
      aria-label="刷新"
    >
      <IconRefresh size={16} />
    </button>
  );

  const toolbar = (
    <div className="toolbar">
      <input
        className="input"
        style={{ flex: 1, maxWidth: 420 }}
        placeholder="为这一刻命名，如：第三章完稿、推翻重写前"
        value={label}
        onChange={(e) => setLabel(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void create();
        }}
      />
      <button
        className="btn btn--primary"
        onClick={() => void create()}
        disabled={creating}
      >
        {creating ? <Spinner size={15} /> : <IconPlus size={16} />}
        立此存照
      </button>
      <span className="count-pill">{list.length} 个快照</span>
    </div>
  );

  return (
    <Panel
      title="快照"
      en="Checkpoints"
      subtitle="为创作旅程立碑刻石 · 随时回溯到任一时刻"
      actions={headerActions}
      toolbar={toolbar}
    >
      <div className="scroll-area">
        {loading ? (
          <LoadingBlock label="正在翻检卷宗…" />
        ) : error ? (
          <div className="banner banner--warn">{error}</div>
        ) : ordered.length === 0 ? (
          <EmptyState
            title="尚无快照"
            text="在重要节点为工作区拍一张“快照”，日后若想反悔，便能一键回到此刻。"
          />
        ) : (
          <div className="timeline">
            {ordered.map((cp) => {
              const when = formatWhen(cp.created_at);
              return (
                <div className="cp" key={cp.id}>
                  <div className="cp__rail">
                    <span className="cp__node" />
                  </div>
                  <div className="cp__card">
                    <div>
                      <div className="cp__label">
                        {cp.label || "（未命名快照）"}
                      </div>
                      <div className="cp__meta">
                        <IconClock size={12} style={{ verticalAlign: "-2px", marginRight: 4 }} />
                        {when || "时间未知"}
                        <span style={{ margin: "0 8px", opacity: 0.5 }}>·</span>
                        <code style={{ fontSize: 11 }}>{cp.id}</code>
                      </div>
                    </div>
                    <div className="cp__actions">
                      <button
                        className="btn btn--danger btn--sm"
                        onClick={() => setTarget(cp)}
                      >
                        <IconRestore size={15} />
                        回溯
                      </button>
                      <button
                        className="btn btn--ghost btn--icon"
                        onClick={() => setDelTarget(cp)}
                        title="删除快照"
                        aria-label="删除快照"
                      >
                        <IconTrash size={15} />
                      </button>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      <ConfirmModal
        open={target !== null}
        title="回溯到此快照？"
        sealChar="溯"
        danger
        busy={restoring}
        confirmLabel="确认回溯"
        body={
          <>
            工作区将被还原到快照
            <br />
            <code>{target?.label || target?.id}</code>
            <br />
            当前未保存或未存照的改动可能会丢失，此操作请谨慎。
          </>
        }
        onConfirm={() => void restore()}
        onCancel={() => setTarget(null)}
      />

      <ConfirmModal
        open={delTarget !== null}
        title="删除这个快照？"
        sealChar="删"
        danger
        busy={deleting}
        confirmLabel="删除"
        body={
          <>
            将永久删除快照
            <br />
            <code>{delTarget?.label || delTarget?.id}</code>
            <br />
            仅删除这一存档记录，当前工作区文件不受影响。
          </>
        }
        onConfirm={() => void removeCp()}
        onCancel={() => {
          if (!deleting) setDelTarget(null);
        }}
      />
    </Panel>
  );
}
