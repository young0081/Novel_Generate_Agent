import { useEffect, useState } from "react";
import { getCollection, listCollections, type CollectionHistory, type CollectionRecord } from "../lib/knowledge";
import { describeError } from "../lib/core";
import { formatTime } from "../lib/sessions";
import { collectionStatus } from "../lib/knowledgeFillResult";
import { stopReasonLabel } from "../lib/agentRun";

export default function KnowledgeHistory({ kbId, revision, busy, onResume }: {
  kbId: string;
  revision: number;
  busy: boolean;
  onResume: (record: CollectionRecord) => void;
}) {
  const [items, setItems] = useState<CollectionHistory[]>([]);
  const [record, setRecord] = useState<CollectionRecord | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadingId, setLoadingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [selection] = useState(() => ({ current: true }));
  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError(null);
    setRecord(null);
    listCollections(kbId).then((rows) => { if (alive) setItems(rows); })
      .catch((e) => { if (alive) setError(describeError(e)); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [kbId, revision, refresh]);

  // The parent keys this component by work/base, so late requests from a
  // previous selection cannot open a continuation in the new base.
  useEffect(() => {
    selection.current = true;
    return () => { selection.current = false; };
  }, []);
  const open = async (id: string, resume: boolean) => {
    if (busy || loadingId) return;
    setLoadingId(id);
    setError(null);
    try {
      const loaded = await getCollection(kbId, id);
      if (!selection.current) return;
      if (resume) onResume(loaded);
      else setRecord(loaded);
    } catch (e) { if (selection.current) setError(describeError(e)); }
    finally { if (selection.current) setLoadingId(null); }
  };
  return (
    <section className="kb-history" aria-label="历史采集">
      <header className="kb-history__head">
        <div><h3>历史采集</h3><p>沿用此前主题与采集过程，继续补齐资料。</p></div>
        <button className="btn btn--ghost btn--sm" disabled={loading || busy || !!loadingId} onClick={() => setRefresh((n) => n + 1)}>刷新记录</button>
      </header>
      {error && <p role="alert">采集记录载入失败：{error}</p>}
      {loading ? <p role="status">正在载入采集记录…</p> : items.length === 0 ? <p>暂无历史采集。首次联网填充后，记录会保存在这里。</p> : (
        <div className="kb-history__list">
          {items.map((item) => {
            const last = item.runs[item.runs.length - 1];
            return <article key={item.id} className="kb-history__item">
              <div><h4>{item.topic}</h4><p>{formatTime(item.updated_ms)} · {collectionStatus(last?.status)} · {item.runs.length} 轮</p>
                <p>已确认新增 {item.runs.reduce((n, run) => n + run.added, 0)} 条{last ? ` · 最近一轮 ${last.added} 条 / ${last.sources} 个来源` : ""}</p>
              </div>
              <div className="kb-history__actions">
                <button className="btn btn--ghost btn--sm" disabled={busy || !!loadingId} onClick={() => void open(item.id, false)}>查看记录</button>
                <button className="btn btn--primary btn--sm" disabled={busy || !!loadingId} onClick={() => void open(item.id, true)}>继续采集</button>
              </div>
            </article>;
          })}
        </div>
      )}
      {record && <div className="kb-history__detail" aria-label="采集详情">
        <header className="kb-history__head"><h4>{record.history.topic} · 采集记录</h4><button className="btn btn--ghost btn--sm" onClick={() => setRecord(null)}>收起记录</button></header>
        {record.history.runs.map((run, i) => <p key={i}>第 {i + 1} 轮 · {collectionStatus(run.status)} · 新增 {run.added} 条 · {run.steps} 步{run.stopped_reason && ` · ${stopReasonLabel(run.stopped_reason)}`}{run.follow_up && ` · 要求：${run.follow_up}`}{run.error && ` · ${run.error}`}</p>)}
        <div className="kb-history__transcript">{record.session.messages.filter((m) => m.role !== "system").map((m, i) => <div key={i} className="msg"><strong>{{ user: "采集要求", assistant: "采集过程", tool: "工具结果", system: "系统" }[m.role]}{m.tool_call && ` · ${m.tool_call.name}`}</strong><pre>{m.content || JSON.stringify(m.tool_call?.args, null, 2)}</pre></div>)}</div>
      </div>}
    </section>
  );
}
