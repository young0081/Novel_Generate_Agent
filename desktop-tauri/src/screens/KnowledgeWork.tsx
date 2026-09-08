// 知识库 — per-work RAG knowledge bases. Manage multiple bases, browse & curate
// entries, search (RAG preview), toggle which bases feed creation, and auto-fill
// a base from source material via the active model.

import { useCallback, useEffect, useRef, useState } from "react";
import { Spinner } from "../components/Spinner";
import AgentFeed from "../components/agent/AgentFeed";
import WorkStatus from "../components/agent/WorkStatus";
import ConfirmModal from "../components/ConfirmModal";
import BatchActions from "../components/BatchActions";
import KnowledgeHistory from "../components/KnowledgeHistory";
import {
  IconPlus,
  IconTrash,
  IconSearch,
  IconClose,
  IconCheck,
  IconBrush,
  IconScroll,
  IconProviders,
  IconStop,
  IconWarn,
} from "../components/icons";
import { useToast } from "../components/Toast";
import { useWork } from "../components/WorkContext";
import {
  listBases,
  createBase,
  deleteBase,
  setBaseActive,
  listEntries,
  addEntry,
  deleteEntry,
  searchKnowledge,
  fillFromTopic,
  KIND_LABELS,
  type KnowledgeBaseMeta,
  type KnowledgeEntry,
  type KnowledgeHit,
  type KnowledgeKind,
  type CollectionRecord,
} from "../lib/knowledge";
import { cancel, describeError, isDesktop, newRequestId } from "../lib/core";
import {
  derivePhase,
  settlePendingTools,
  stopReasonLabel,
  upsertStep,
  type RunStep,
} from "../lib/agentRun";
import type { AgentStep } from "../lib/studio";
import { useDialogFocus } from "../lib/dialogLayer";
import { scrollLiveAnchor } from "../lib/liveScroll";
import { runBatch } from "../lib/batch";
import { describeKnowledgeFill } from "../lib/knowledgeFillResult";

const KIND_OPTIONS: KnowledgeKind[] = [
  "character",
  "location",
  "worldbuilding",
  "event",
  "item",
  "term",
  "lore",
  "other",
];

export default function KnowledgeWork() {
  const { current } = useWork();
  return <KnowledgeWorkspace key={current?.id ?? "none"} />;
}

function KnowledgeWorkspace() {
  const toast = useToast();
  const { current } = useWork();

  const [bases, setBases] = useState<KnowledgeBaseMeta[]>([]);
  const [loadingBases, setLoadingBases] = useState(true);
  const [activeKb, setActiveKb] = useState<string | null>(null);
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [loadingEntries, setLoadingEntries] = useState(false);
  const [entryManageMode, setEntryManageMode] = useState(false);
  const [selectedEntryIds, setSelectedEntryIds] = useState<Set<string>>(new Set());
  const [pendingBulkEntries, setPendingBulkEntries] = useState<KnowledgeEntry[] | null>(null);
  const [bulkDeletingEntries, setBulkDeletingEntries] = useState(false);
  const [bulkEntryProgress, setBulkEntryProgress] = useState<{ done: number; total: number } | null>(null);

  const [baseManageMode, setBaseManageMode] = useState(false);
  const [selectedBaseIds, setSelectedBaseIds] = useState<Set<string>>(new Set());
  const [pendingBulkBases, setPendingBulkBases] = useState<KnowledgeBaseMeta[] | null>(null);
  const [bulkDeletingBases, setBulkDeletingBases] = useState(false);
  const [bulkBaseProgress, setBulkBaseProgress] = useState<{ done: number; total: number } | null>(null);

  // search (RAG preview)
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<KnowledgeHit[] | null>(null);
  const [searching, setSearching] = useState(false);

  // create base
  const [newBaseName, setNewBaseName] = useState("");
  const [showNewBase, setShowNewBase] = useState(false);

  // add entry
  const [showAddEntry, setShowAddEntry] = useState(false);
  const [addingEntry, setAddingEntry] = useState(false);
  const [entryDraft, setEntryDraft] = useState<{
    kind: KnowledgeKind;
    title: string;
    content: string;
    tags: string;
  }>({ kind: "lore", title: "", content: "", tags: "" });

  // auto-fill
  const [showFill, setShowFill] = useState(false);
  const [fillTopic, setFillTopic] = useState("");
  const [fillSessionId, setFillSessionId] = useState<string | null>(null);
  const [fillFollowUp, setFillFollowUp] = useState("");
  const [fillHistory, setFillHistory] = useState<CollectionRecord | null>(null);
  const [historyRevision, setHistoryRevision] = useState(0);
  const [filling, setFilling] = useState(false);
  const [fillCancelling, setFillCancelling] = useState(false);
  const [fillSteps, setFillSteps] = useState<RunStep[]>([]);
  const [fillFinished, setFillFinished] = useState(false);
  const [fillSuccess, setFillSuccess] = useState<boolean | null>(null);
  const [fillCancelled, setFillCancelled] = useState(false);
  const [fillFinishNote, setFillFinishNote] = useState<string | null>(null);
  const [fillError, setFillError] = useState<string | null>(null);
  const fillStepSeqRef = useRef(0);
  const fillPendingDeltaRef = useRef<AgentStep | null>(null);
  const fillRafRef = useRef<number | null>(null);
  const fillRequestRef = useRef<string | null>(null);
  const fillCancelRequestedRef = useRef(false);

  // delete base
  const [delBase, setDelBase] = useState<KnowledgeBaseMeta | null>(null);
  const [deletingBase, setDeletingBase] = useState(false);
  const addEntryDialogRef = useRef<HTMLDivElement>(null);
  const fillDialogRef = useRef<HTMLDivElement>(null);
  const fillTailRef = useRef<HTMLDivElement>(null);
  const addingEntryRef = useRef(addingEntry);
  const fillingRef = useRef(filling);
  addingEntryRef.current = addingEntry;
  fillingRef.current = filling;

  const closeAddEntry = useCallback(() => {
    if (!addingEntryRef.current) setShowAddEntry(false);
  }, []);

  const closeFill = useCallback(() => {
    if (!fillingRef.current) setShowFill(false);
  }, []);

  useDialogFocus(showAddEntry, addEntryDialogRef, closeAddEntry);
  useDialogFocus(showFill, fillDialogRef, closeFill);

  useEffect(() => {
    if (!showFill) return;
    const frame = window.requestAnimationFrame(() => {
      scrollLiveAnchor(fillTailRef.current, { live: filling });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [showFill, filling, fillCancelling, fillSteps, fillFinishNote, fillError]);

  useEffect(() => {
    return () => {
      if (fillRafRef.current != null) window.cancelAnimationFrame(fillRafRef.current);
      if (fillRequestRef.current) void cancel(fillRequestRef.current);
    };
  }, []);

  const loadBases = useCallback(async () => {
    if (!isDesktop()) {
      setLoadingBases(false);
      return;
    }
    setLoadingBases(true);
    try {
      const list = await listBases();
      setBases(list);
      // keep selection valid
      setActiveKb((prev) => {
        if (prev && list.some((b) => b.id === prev)) return prev;
        return list[0]?.id ?? null;
      });
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setLoadingBases(false);
    }
  }, [toast]);

  const loadEntries = useCallback(
    async (kbId: string) => {
      setLoadingEntries(true);
      try {
        setEntries(await listEntries(kbId));
      } catch (e) {
        toast.err(describeError(e));
      } finally {
        setLoadingEntries(false);
      }
    },
    [toast],
  );

  // reload bases whenever the active work changes
  useEffect(() => {
    void loadBases();
  }, [loadBases, current?.id]);

  useEffect(() => {
    if (activeKb) void loadEntries(activeKb);
    else setEntries([]);
    setSelectedEntryIds(new Set());
    setEntryManageMode(false);
  }, [activeKb, loadEntries]);

  useEffect(() => {
    const valid = new Set(entries.map((entry) => entry.id));
    setSelectedEntryIds((current) => {
      const next = new Set([...current].filter((id) => valid.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [entries]);

  useEffect(() => {
    const valid = new Set(bases.map((base) => base.id));
    setSelectedBaseIds((current) => {
      const next = new Set([...current].filter((id) => valid.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [bases]);

  const onCreateBase = useCallback(async () => {
    const name = newBaseName.trim();
    if (!name) return;
    try {
      const meta = await createBase(name);
      setNewBaseName("");
      setShowNewBase(false);
      await loadBases();
      setActiveKb(meta.id);
      toast.ok("知识库已创建");
    } catch (e) {
      toast.err(describeError(e));
    }
  }, [newBaseName, loadBases, toast]);

  const onToggleActive = useCallback(
    async (kb: KnowledgeBaseMeta) => {
      try {
        await setBaseActive(kb.id, !kb.active);
        await loadBases();
      } catch (e) {
        toast.err(describeError(e));
      }
    },
    [loadBases, toast],
  );

  const onAddEntry = useCallback(async () => {
    if (addingEntry || !activeKb || !entryDraft.title.trim() || !entryDraft.content.trim()) return;
    setAddingEntry(true);
    try {
      await addEntry({
        kbId: activeKb,
        kind: entryDraft.kind,
        title: entryDraft.title.trim(),
        content: entryDraft.content.trim(),
        tags: entryDraft.tags
          .split(/[,，\s]+/)
          .map((t) => t.trim())
          .filter(Boolean),
      });
      setEntryDraft({ kind: "lore", title: "", content: "", tags: "" });
      setShowAddEntry(false);
      await loadEntries(activeKb);
      await loadBases();
      toast.ok("已添加条目");
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setAddingEntry(false);
    }
  }, [activeKb, addingEntry, entryDraft, loadEntries, loadBases, toast]);

  const onDeleteEntry = useCallback(
    async (entryId: string) => {
      if (!activeKb) return;
      try {
        await deleteEntry(activeKb, entryId);
        await loadEntries(activeKb);
        await loadBases();
      } catch (e) {
        toast.err(describeError(e));
      }
    },
    [activeKb, loadEntries, loadBases, toast],
  );

  const confirmBulkDeleteEntries = useCallback(async () => {
    if (!activeKb || !pendingBulkEntries) return;
    setBulkDeletingEntries(true);
    setBulkEntryProgress({ done: 0, total: pendingBulkEntries.length });
    try {
      const result = await runBatch(
        pendingBulkEntries,
        async (entry) => {
          await deleteEntry(activeKb, entry.id);
        },
        (done, total) => setBulkEntryProgress({ done, total }),
      );
      const completedIds = new Set(result.completed.map((entry) => entry.id));
      setEntries((current) => current.filter((entry) => !completedIds.has(entry.id)));
      setSelectedEntryIds((current) => new Set([...current].filter((id) => !completedIds.has(id))));
      await loadBases();
      if (result.failed.length === 0) toast.ok(`已删除 ${result.completed.length} 条知识条目`);
      else toast.err(`已删除 ${result.completed.length} 条，${result.failed.length} 条删除失败`);
      setPendingBulkEntries(null);
    } finally {
      setBulkDeletingEntries(false);
      setBulkEntryProgress(null);
    }
  }, [activeKb, loadBases, pendingBulkEntries, toast]);

  const onSearch = useCallback(async () => {
    const q = query.trim();
    if (!q) {
      setHits(null);
      return;
    }
    setSearching(true);
    try {
      setHits(await searchKnowledge(q, 10));
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setSearching(false);
    }
  }, [query, toast]);

  const resetFillRun = useCallback(() => {
    if (fillRafRef.current != null) {
      window.cancelAnimationFrame(fillRafRef.current);
      fillRafRef.current = null;
    }
    fillPendingDeltaRef.current = null;
    fillStepSeqRef.current = 0;
    setFillSteps([]);
    setFillFinished(false);
    setFillSuccess(null);
    setFillCancelled(false);
    setFillFinishNote(null);
    setFillError(null);
  }, []);

  const handleFillStepNow = useCallback((step: AgentStep) => {
    setFillSteps((currentSteps) => (
      upsertStep(currentSteps, step, () => (fillStepSeqRef.current += 1))
    ));
    if (step.phase !== "finish") return;

    // The command validates durable entries after the loop finishes.
    // A model's finish event alone is not a successful collection.
  }, []);

  const flushFillDelta = useCallback(() => {
    fillRafRef.current = null;
    const pending = fillPendingDeltaRef.current;
    fillPendingDeltaRef.current = null;
    if (pending) handleFillStepNow(pending);
  }, [handleFillStepNow]);

  const flushFillDeltaImmediately = useCallback(() => {
    if (fillRafRef.current != null) {
      window.cancelAnimationFrame(fillRafRef.current);
      fillRafRef.current = null;
    }
    flushFillDelta();
  }, [flushFillDelta]);

  const handleFillStep = useCallback((step: AgentStep) => {
    if (step.phase !== "delta") {
      flushFillDeltaImmediately();
      handleFillStepNow(step);
      return;
    }

    const pending = fillPendingDeltaRef.current;
    fillPendingDeltaRef.current = pending?.phase === "delta" && pending.step === step.step
      ? { ...pending, delta: pending.delta + step.delta }
      : step;
    if (fillRafRef.current == null) {
      fillRafRef.current = window.requestAnimationFrame(flushFillDelta);
    }
  }, [flushFillDelta, flushFillDeltaImmediately, handleFillStepNow]);

  const onFill = useCallback(async () => {
    if (!activeKb || !fillTopic.trim() || filling || fillRequestRef.current) return;
    resetFillRun();
    setFilling(true);
    setFillCancelling(false);
    fillCancelRequestedRef.current = false;
    const requestId = newRequestId("knowledge");
    fillRequestRef.current = requestId;
    try {
      const result = await fillFromTopic(activeKb, fillTopic.trim(), handleFillStep, requestId, fillSessionId ?? undefined, fillFollowUp);
      setFillSessionId(result.session.id);
      setFillFollowUp("");
      setFillHistory(null);
      flushFillDeltaImmediately();
      const { outcome } = result;
      const summary = describeKnowledgeFill(result);
      const stoppedReason = outcome.stopped_reason;
      const stopped = stoppedReason === "cancelled";
      if (outcome.warning) toast.info(outcome.warning);
      if (stopped) {
        setFillSteps((steps) => settlePendingTools(steps, "cancelled", "已由用户停止"));
        setFillFinished(true);
        setFillSuccess(false);
        setFillCancelled(true);
        setFillFinishNote(summary.note);
        toast.info("已停止填充");
      } else if (!summary.success) {
        setFillSteps((steps) => settlePendingTools(steps, "error", stopReasonLabel(stoppedReason)));
        setFillFinished(true);
        setFillSuccess(false);
        setFillCancelled(false);
        setFillFinishNote(summary.note);
        setFillError(summary.error || (stoppedReason !== "goal_reached" ? stopReasonLabel(stoppedReason) : null));
        toast.info(summary.note);
      } else {
        setFillSteps((steps) => settlePendingTools(steps, "success"));
        setFillFinished(true);
        setFillSuccess(true);
        setFillCancelled(false);
        setFillFinishNote(summary.note);
        toast.ok(summary.note);
      }
    } catch (e) {
      flushFillDeltaImmediately();
      const stopped = fillCancelRequestedRef.current;
      const message = stopped ? "已由用户停止" : describeError(e);
      setFillSteps((steps) => settlePendingTools(
        steps,
        stopped ? "cancelled" : "error",
        message,
      ));
      setFillFinished(true);
      setFillSuccess(false);
      setFillCancelled(stopped);
      if (stopped) {
        setFillFinishNote("已由用户停止");
        setFillError(null);
        toast.info("已停止填充");
      } else {
        setFillFinishNote(null);
        setFillError(message);
        toast.err(message);
      }
    } finally {
      // A transport error can arrive after successful writes; reload those too.
      await loadEntries(activeKb);
      await loadBases();
      setHistoryRevision((n) => n + 1);
      if (fillRequestRef.current === requestId) fillRequestRef.current = null;
      setFilling(false);
      setFillCancelling(false);
    }
  }, [
    activeKb,
    fillTopic,
    fillSessionId,
    fillFollowUp,
    filling,
    flushFillDeltaImmediately,
    handleFillStep,
    loadEntries,
    loadBases,
    resetFillRun,
    toast,
  ]);

  const stopFill = useCallback(async () => {
    const requestId = fillRequestRef.current;
    if (!requestId || fillCancelling) return;
    fillCancelRequestedRef.current = true;
    setFillCancelling(true);
    try {
      await cancel(requestId);
    } catch (error) {
      fillCancelRequestedRef.current = false;
      setFillCancelling(false);
      toast.err(`停止失败：${describeError(error)}`);
    }
  }, [fillCancelling, toast]);

  const confirmDeleteBase = useCallback(async () => {
    if (pendingBulkBases) {
      setBulkDeletingBases(true);
      setBulkBaseProgress({ done: 0, total: pendingBulkBases.length });
      try {
        const result = await runBatch(
          pendingBulkBases,
          async (base) => { await deleteBase(base.id); },
          (done, total) => setBulkBaseProgress({ done, total }),
        );
        const completedIds = new Set(result.completed.map((base) => base.id));
        setSelectedBaseIds((current) => new Set([...current].filter((id) => !completedIds.has(id))));
        if (activeKb && completedIds.has(activeKb)) setActiveKb(null);
        await loadBases();
        if (result.failed.length === 0) toast.ok(`已删除 ${result.completed.length} 个知识库`);
        else toast.err(`已删除 ${result.completed.length} 个，${result.failed.length} 个删除失败`);
        setPendingBulkBases(null);
      } finally {
        setBulkDeletingBases(false);
        setBulkBaseProgress(null);
      }
      return;
    }
    if (!delBase) return;
    setDeletingBase(true);
    try {
      await deleteBase(delBase.id);
      setDelBase(null);
      await loadBases();
      toast.ok("知识库已删除");
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setDeletingBase(false);
    }
  }, [activeKb, delBase, loadBases, pendingBulkBases, toast]);

  const currentBase = bases.find((b) => b.id === activeKb) ?? null;
  const allEntriesSelected = entries.length > 0 && entries.every((entry) => selectedEntryIds.has(entry.id));
  const allBasesSelected = bases.length > 0 && bases.every((base) => selectedBaseIds.has(base.id));
  const toggleEntry = useCallback((id: string) => {
    setSelectedEntryIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);
  const toggleAllEntries = useCallback(() => {
    setSelectedEntryIds(allEntriesSelected ? new Set() : new Set(entries.map((entry) => entry.id)));
  }, [allEntriesSelected, entries]);
  const toggleBase = useCallback((id: string) => {
    setSelectedBaseIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);
  const toggleAllBases = useCallback(() => {
    setSelectedBaseIds(allBasesSelected ? new Set() : new Set(bases.map((base) => base.id)));
  }, [allBasesSelected, bases]);
  const fillActivelyRunning = filling && !fillFinished;
  const hasFillRun = filling || fillSteps.length > 0 || fillFinished || fillError !== null;
  const fillPhase = derivePhase({
    running: fillActivelyRunning,
    steps: fillSteps,
    finished: fillFinished,
    success: fillSuccess,
    errored: fillError !== null,
    cancelling: fillCancelling,
    cancelled: fillCancelled,
  });
  const fillLastStep = fillSteps.length > 0 ? fillSteps[fillSteps.length - 1].step : 0;
  const fillToolCount = fillSteps.reduce((count, step) => count + step.toolCalls.length, 0);
  const fillCurrentTool = fillPhase === "tooling" && fillSteps.length > 0
    ? fillSteps[fillSteps.length - 1].toolCalls.find(
        (tool) => tool.status === "queued" || tool.status === "running",
      )?.name
    : undefined;

  return (
    <div className="kb">
      {/* Left: base list */}
      <aside className="kb__sidebar">
        <div className="kb__sidebar-head">
          <span className="kb__sidebar-title">知识库</span>
          <div className="kb__sidebar-actions">
            {bases.length > 1 && (
              <button
                className={`btn btn--ghost btn--xs${baseManageMode ? " is-active" : ""}`}
                onClick={() => {
                  setBaseManageMode((value) => !value);
                  setSelectedBaseIds(new Set());
                }}
                aria-pressed={baseManageMode}
              >
                {baseManageMode ? "完成" : "管理"}
              </button>
            )}
            <button
              className="icon-btn"
              title="新建知识库"
              aria-label="新建知识库"
              onClick={() => setShowNewBase((s) => !s)}
            >
              <IconPlus size={15} />
            </button>
          </div>
        </div>

        {baseManageMode && bases.length > 0 && (
          <BatchActions
            selectedCount={selectedBaseIds.size}
            totalCount={bases.length}
            allSelected={allBasesSelected}
            onToggleAll={toggleAllBases}
            onClear={() => setSelectedBaseIds(new Set())}
            label="批量管理知识库"
          >
            <button
              type="button"
              className="btn btn--danger btn--sm"
              disabled={selectedBaseIds.size === 0 || bulkDeletingBases}
              onClick={() => setPendingBulkBases(bases.filter((base) => selectedBaseIds.has(base.id)))}
            >
              <IconTrash size={13} /> 删除已选
            </button>
          </BatchActions>
        )}

        {showNewBase && (
          <div className="kb__new-base">
            <input
              className="field__input"
              value={newBaseName}
              onChange={(e) => setNewBaseName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void onCreateBase();
                if (e.key === "Escape") setShowNewBase(false);
              }}
              placeholder="知识库名称…"
              autoFocus
            />
            <button
              className="btn btn--primary btn--sm"
              onClick={() => void onCreateBase()}
              disabled={!newBaseName.trim()}
            >
              创建
            </button>
          </div>
        )}

        <div className="kb__base-list">
          {loadingBases ? (
            <div className="kb__loading">
              <Spinner size={20} />
            </div>
          ) : bases.length === 0 ? (
            <p className="kb__sidebar-empty">还没有知识库</p>
          ) : (
            bases.map((b) => (
              <div key={b.id} className={`kb__base-row${baseManageMode ? " is-manage" : ""}`}>
                {baseManageMode && (
                  <input
                    type="checkbox"
                    className="batch-select"
                    checked={selectedBaseIds.has(b.id)}
                    onChange={() => toggleBase(b.id)}
                    aria-label={`选择知识库：${b.name}`}
                  />
                )}
                <button
                  className={`kb__base${b.id === activeKb ? " is-active" : ""}`}
                  onClick={() => baseManageMode ? toggleBase(b.id) : setActiveKb(b.id)}
                >
                  <span className="kb__base-name">{b.name}</span>
                  <span className="kb__base-count">{b.entry_count}</span>
                  <span
                    className={`kb__base-dot${b.active ? " is-on" : ""}`}
                    title={b.active ? "参与检索" : "已禁用"}
                  />
                </button>
              </div>
            ))
          )}
        </div>

        <div className="kb__sidebar-foot">
          <span className="kb__legend">
            <span className="kb__base-dot is-on" /> 绿点 = 参与创作检索
          </span>
        </div>
      </aside>

      {/* Right: entries + tools */}
      <section className="kb__main">
        {!currentBase ? (
          <div className="kb__empty">
            <div className="kb__empty-icon">
              <IconScroll size={32} />
            </div>
            <h3>为《{current?.title ?? "作品"}》建立知识库</h3>
            <p>
              知识库为创作提供设定准绳（RAG）：AI 写作前会自动检索相关条目，
              <br />
              确保情节不偏离世界观与人物设定。
            </p>
            <button className="btn btn--primary" onClick={() => setShowNewBase(true)}>
              <IconPlus size={16} />
              新建知识库
            </button>
          </div>
        ) : (
          <>
            <header className="kb__main-head">
              <div className="kb__main-title">
                <h2>{currentBase.name}</h2>
                <label className="kb__active-toggle">
                  <input
                    type="checkbox"
                    checked={currentBase.active}
                    onChange={() => void onToggleActive(currentBase)}
                  />
                  参与创作检索
                </label>
              </div>
              <div className="kb__main-actions">
                {hits === null && entries.length > 0 && (
                  <button
                    className={`btn btn--ghost btn--sm${entryManageMode ? " is-active" : ""}`}
                    onClick={() => {
                      setEntryManageMode((value) => !value);
                      setSelectedEntryIds(new Set());
                    }}
                    aria-pressed={entryManageMode}
                  >
                    {entryManageMode ? "完成" : "批量管理"}
                  </button>
                )}
                <button
                  className="btn btn--ghost btn--sm"
                  onClick={() => {
                    resetFillRun();
                    setFillSessionId(null);
                    setFillFollowUp("");
                    setFillHistory(null);
                    setFillTopic(current?.source_material || "");
                    setShowFill(true);
                  }}
                >
                  <IconProviders size={14} />
                  联网填充
                </button>
                <button
                  className="btn btn--ghost btn--sm"
                  onClick={() => setShowAddEntry(true)}
                >
                  <IconPlus size={14} />
                  添加条目
                </button>
                <button
                  className="icon-btn icon-btn--danger"
                  title="删除知识库"
                  aria-label="删除知识库"
                  onClick={() => setDelBase(currentBase)}
                >
                  <IconTrash size={15} />
                </button>
              </div>
            </header>

            <KnowledgeHistory
              key={`${current?.id}:${activeKb}`}
              kbId={currentBase.id}
              revision={historyRevision}
              busy={filling}
              onResume={(record) => {
                if (record.history.kb_id !== activeKb) return;
                resetFillRun();
                setFillSessionId(record.history.id);
                setFillTopic(record.history.topic);
                setFillFollowUp("");
                setFillHistory(record);
                setShowFill(true);
              }}
            />

            {/* RAG search */}
            <div className="kb__search">
              <IconSearch size={15} />
              <input
                className="kb__search-input"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void onSearch();
                  if (e.key === "Escape") {
                    setQuery("");
                    setHits(null);
                  }
                }}
                placeholder="检索设定（模拟创作时的 RAG 召回）…"
              />
              {query && (
                <button
                  className="icon-btn"
                  onClick={() => {
                    setQuery("");
                    setHits(null);
                  }}
                  aria-label="清空"
                >
                  <IconClose size={13} />
                </button>
              )}
              <button
                className="btn btn--primary btn--sm"
                onClick={() => void onSearch()}
                disabled={!query.trim() || searching}
              >
                {searching ? <Spinner size={13} /> : "检索"}
              </button>
            </div>

            {entryManageMode && hits === null && entries.length > 0 && (
              <BatchActions
                selectedCount={selectedEntryIds.size}
                totalCount={entries.length}
                allSelected={allEntriesSelected}
                onToggleAll={toggleAllEntries}
                onClear={() => setSelectedEntryIds(new Set())}
                label="批量管理知识条目"
              >
                <button
                  type="button"
                  className="btn btn--danger btn--sm"
                  disabled={selectedEntryIds.size === 0 || bulkDeletingEntries}
                  onClick={() => setPendingBulkEntries(entries.filter((entry) => selectedEntryIds.has(entry.id)))}
                >
                  <IconTrash size={13} /> 删除已选
                </button>
              </BatchActions>
            )}

            {/* Search results OR all entries */}
            <div className="kb__entries">
              {hits !== null ? (
                <>
                  <div className="kb__entries-label">
                    检索结果 · {hits.length} 条
                    <button
                      className="link-btn"
                      onClick={() => {
                        setHits(null);
                        setQuery("");
                      }}
                    >
                      返回全部
                    </button>
                  </div>
                  {hits.length === 0 ? (
                    <p className="kb__no-results">没有命中的设定条目</p>
                  ) : (
                    hits.map((h) => (
                      <article key={h.entry.id} className="kb-entry">
                        <div className="kb-entry__head">
                          <span className={`kb-entry__kind kind--${h.entry.kind}`}>
                            {KIND_LABELS[h.entry.kind]}
                          </span>
                          <h4 className="kb-entry__title">{h.entry.title}</h4>
                          <span className="kb-entry__score">
                            {h.score.toFixed(2)} · {h.kb_name}
                          </span>
                        </div>
                        <p className="kb-entry__content">{h.entry.content}</p>
                      </article>
                    ))
                  )}
                </>
              ) : loadingEntries ? (
                <div className="kb__loading">
                  <Spinner size={24} />
                </div>
              ) : entries.length === 0 ? (
                <div className="kb__entries-empty">
                  <p>这个知识库还是空的。</p>
                  <p className="kb__entries-empty-hint">
                    手动添加条目，或用「联网填充」让 AI 自动整理设定资料。
                  </p>
                </div>
              ) : (
                entries.map((en) => (
                    <article key={en.id} className={`kb-entry${entryManageMode ? " is-manage" : ""}`}>
                      <div className="kb-entry__head">
                        {entryManageMode && (
                          <input
                            type="checkbox"
                            className="batch-select"
                            checked={selectedEntryIds.has(en.id)}
                            onChange={() => toggleEntry(en.id)}
                            aria-label={`选择条目：${en.title}`}
                          />
                        )}
                        <span className={`kb-entry__kind kind--${en.kind}`}>
                        {KIND_LABELS[en.kind]}
                      </span>
                      <h4 className="kb-entry__title">{en.title}</h4>
                      <button
                        className="icon-btn icon-btn--danger kb-entry__del"
                        title="删除条目"
                        aria-label="删除条目"
                        onClick={() => void onDeleteEntry(en.id)}
                      >
                        <IconTrash size={13} />
                      </button>
                    </div>
                    <p className="kb-entry__content">{en.content}</p>
                    {en.tags.length > 0 && (
                      <div className="kb-entry__tags">
                        {en.tags.map((t) => (
                          <span key={t} className="kb-entry__tag">
                            {t}
                          </span>
                        ))}
                      </div>
                    )}
                  </article>
                ))
              )}
            </div>
          </>
        )}
      </section>

      {/* Add-entry drawer */}
      {showAddEntry && (
        <div className="library__overlay" onClick={closeAddEntry}>
          <div
            ref={addEntryDialogRef}
            className="library__sheet"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-modal="true"
            aria-label="添加条目"
            tabIndex={-1}
          >
            <header className="library__sheet-head">
              <h3>添加设定条目</h3>
              <button className="icon-btn" onClick={closeAddEntry} aria-label="关闭">
                <IconClose size={16} />
              </button>
            </header>
            <div className="library__sheet-body">
              <div className="field-row">
                <label className="field">
                  <span className="field__label">类型</span>
                  <select
                    className="field__input"
                    value={entryDraft.kind}
                    onChange={(e) =>
                      setEntryDraft((d) => ({ ...d, kind: e.target.value as KnowledgeKind }))
                    }
                  >
                    {KIND_OPTIONS.map((k) => (
                      <option key={k} value={k}>
                        {KIND_LABELS[k]}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field" style={{ flex: 2 }}>
                  <span className="field__label">标题 *</span>
                  <input
                    className="field__input"
                    value={entryDraft.title}
                    onChange={(e) => setEntryDraft((d) => ({ ...d, title: e.target.value }))}
                    placeholder="名称 / 术语…"
                    data-autofocus
                  />
                </label>
              </div>
              <label className="field">
                <span className="field__label">内容 *</span>
                <textarea
                  className="field__input field__textarea"
                  value={entryDraft.content}
                  onChange={(e) => setEntryDraft((d) => ({ ...d, content: e.target.value }))}
                  placeholder="详细设定描述…"
                  rows={5}
                />
              </label>
              <label className="field">
                <span className="field__label">标签</span>
                <input
                  className="field__input"
                  value={entryDraft.tags}
                  onChange={(e) => setEntryDraft((d) => ({ ...d, tags: e.target.value }))}
                  placeholder="用逗号分隔，如：主角, 剑客"
                />
              </label>
            </div>
            <footer className="library__sheet-foot">
              <button className="btn btn--ghost" onClick={closeAddEntry}>
                取消
              </button>
              <button
                className="btn btn--primary"
                onClick={() => void onAddEntry()}
                disabled={addingEntry || !entryDraft.title.trim() || !entryDraft.content.trim()}
              >
                {addingEntry ? <Spinner size={14} /> : <IconCheck size={16} />}
                {addingEntry ? "添加中…" : "添加"}
              </button>
            </footer>
          </div>
        </div>
      )}

      {/* Auto-fill drawer */}
      {showFill && (
        <div className="library__overlay" onClick={closeFill}>
          <div
            ref={fillDialogRef}
            className="library__sheet"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-modal="true"
            aria-label="联网填充"
            tabIndex={-1}
          >
            <header className="library__sheet-head">
              <h3>{fillSessionId ? "继续历史采集" : "联网填充设定资料"}</h3>
              <button
                className="icon-btn"
                onClick={closeFill}
                aria-label="关闭"
              >
                <IconClose size={16} />
              </button>
            </header>
            <div className="library__sheet-body">
              <label className="field">
                <span className="field__label">作品 / 题材</span>
                <input
                  className="field__input"
                  value={fillTopic}
                  onChange={(e) => setFillTopic(e.target.value)}
                  placeholder={current?.source_material || "例如：斗破苍穹"}
                  disabled={filling || !!fillSessionId}
                  data-autofocus
                />
              </label>
              {fillSessionId && <>
                <p className="library__hint">沿用原采集会话，补齐未完成资料；已入库条目会跳过重复保存。</p>
                <label className="field"><span className="field__label">本轮补充要求（可选）</span>
                  <textarea className="field__input field__textarea" value={fillFollowUp} onChange={(e) => setFillFollowUp(e.target.value)} disabled={filling} maxLength={4000} rows={3} placeholder="例如：接着补齐重要地点，优先查阅官方资料。" />
                </label>
              </>}
              {fillHistory && <details className="kb-history__detail"><summary>此前采集记录 · {fillHistory.history.runs.length} 轮</summary>
                <div className="kb-history__transcript">{fillHistory.session.messages.filter((m) => m.role !== "system").map((m, i) => <div key={i} className="msg"><strong>{m.role === "user" ? "采集要求" : m.role === "tool" ? "工具结果" : "采集过程"}</strong><pre>{m.content || JSON.stringify(m.tool_call?.args, null, 2)}</pre></div>)}</div>
              </details>}
              <p className="library__hint">
                <IconBrush size={13} />
                AI 将整理该作品的核心人物、世界规则、地点、事件与术语，自动写入当前知识库。
              </p>
              {hasFillRun && (
                <div className="kb__fill-stream">
                  <div className="kb__fill-stream-label">
                    <WorkStatus
                      phase={fillPhase}
                      step={fillLastStep}
                      toolCount={fillToolCount}
                      note={fillCurrentTool}
                    />
                  </div>
                  <AgentFeed
                    steps={fillSteps}
                    running={fillActivelyRunning}
                    phase={fillPhase}
                    pendingText="AI 正在整理下一条设定…"
                    tailRef={fillTailRef}
                  />
                  {!filling && (fillFinishNote || fillError) && (
                    <div
                      className="kb__fill-stream-label"
                      role={fillError ? "alert" : "status"}
                      aria-live={fillError ? "assertive" : "polite"}
                      aria-atomic="true"
                    >
                      {fillSuccess ? (
                        <IconCheck size={14} />
                      ) : fillError ? (
                        <IconWarn size={14} />
                      ) : (
                        <IconStop size={14} />
                      )}
                      <span>{[fillFinishNote, fillError].filter(Boolean).join("；")}</span>
                    </div>
                  )}
                </div>
              )}
            </div>
            <footer className="library__sheet-foot">
              <button
                className="btn btn--ghost"
                onClick={closeFill}
                disabled={filling}
              >
                取消
              </button>
              <button
                className={`btn ${filling ? "btn--danger" : "btn--primary"}`}
                onClick={() => filling ? void stopFill() : void onFill()}
                disabled={filling ? fillCancelling : !fillTopic.trim()}
              >
                {filling ? (
                  fillCancelling ? <Spinner size={14} /> : <IconStop size={14} />
                ) : (
                  <IconProviders size={16} />
                )}
                {fillCancelling ? "停止中…" : filling ? "停止" : fillSessionId ? "继续采集" : "开始填充"}
              </button>
            </footer>
          </div>
        </div>
      )}

      <ConfirmModal
        open={!!delBase || pendingBulkBases !== null}
        title={pendingBulkBases ? `删除选中的 ${pendingBulkBases.length} 个知识库？` : "删除这个知识库？"}
        sealChar="删"
        danger
        busy={deletingBase || bulkDeletingBases}
        confirmLabel="删除"
        body={
          <>
            {pendingBulkBases ? (
              <>
                将永久删除选中的 {pendingBulkBases.length} 个知识库及其中的全部设定。
                {bulkBaseProgress && <><br />正在处理：{bulkBaseProgress.done} / {bulkBaseProgress.total}</>}
                <br />此操作不可撤销。
              </>
            ) : (
              <>将永久删除知识库「{delBase?.name}」及其全部 {delBase?.entry_count} 条设定。<br />此操作不可撤销。</>
            )}
          </>
        }
        onConfirm={() => void confirmDeleteBase()}
        onCancel={() => {
          if (!deletingBase && !bulkDeletingBases) {
            setDelBase(null);
            setPendingBulkBases(null);
          }
        }}
      />

      <ConfirmModal
        open={pendingBulkEntries !== null}
        title={`删除选中的 ${pendingBulkEntries?.length ?? 0} 条知识？`}
        sealChar="删"
        danger
        busy={bulkDeletingEntries}
        confirmLabel="删除"
        body={
          <>
            将永久删除当前知识库中选中的 {pendingBulkEntries?.length ?? 0} 条设定。
            {bulkEntryProgress && <><br />正在处理：{bulkEntryProgress.done} / {bulkEntryProgress.total}</>}
            <br />此操作不可撤销。
          </>
        }
        onConfirm={() => void confirmBulkDeleteEntries()}
        onCancel={() => {
          if (!bulkDeletingEntries) setPendingBulkEntries(null);
        }}
      />
    </div>
  );
}
