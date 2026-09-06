// 推演 — World Simulator.
// Users pick existing memory entries to include, add new ad-hoc settings,
// then the AI simulates a cause-effect timeline. New settings are auto-
// classified and saved back to the memory library after simulation.

import { useCallback, useEffect, useRef, useState } from "react";
import { Spinner } from "../components/Spinner";
import {
  IconSimulate, IconCheck, IconWarn, IconProviders,
  IconStar, IconPlus, IconClose, IconRefresh, IconStop,
} from "../components/icons";
import {
  cancel,
  describeError,
  isDesktop,
  invokeTool,
  newRequestId,
  requireToolSuccess,
} from "../lib/core";
import { useToast } from "../components/Toast";
import { runGoalLive, type AgentStep } from "../lib/studio";
import { getProviders, PROVIDERS_CHANGED_EVENT } from "../lib/providers";
import {
  derivePhase, workflowView, isNoProviderError,
  isProviderCompatibilityError, reachedWorkflowStage, settlePendingTools,
  stopReasonLabel, upsertStep, SIMULATE_STAGES, type RunStep,
} from "../lib/agentRun";
import { type MemoryHit, type MemoryListData, KIND_LABEL } from "../lib/memory";
import WorkStatus from "../components/agent/WorkStatus";
import WorkflowSteps from "../components/agent/WorkflowSteps";
import AgentFeed from "../components/agent/AgentFeed";
import { scrollLiveAnchor } from "../lib/liveScroll";

interface SimulateWorkProps { onOpenSettings?: () => void }

type SimType = "plot" | "character" | "world" | "cause";
const SIM_TYPES: Array<{ key: SimType; label: string; blurb: string }> = [
  { key: "plot",      label: "情节推演", blurb: "推演场景后最可能的情节走向" },
  { key: "character", label: "角色反应", blurb: "各角色面对场景时的决策与行动" },
  { key: "world",     label: "世界演变", blurb: "场景对整个世界格局的长期影响" },
  { key: "cause",     label: "因果链",   blurb: "从场景出发推导完整因果影响链" },
];
const TYPE_FOCUS: Record<SimType, string> = {
  plot:      "从下一个节点起，按时间顺序推演 5-8 个情节事件",
  character: "对每个主要角色分析：心理→决策→行动→后果（至少 3 位角色）",
  world:     "宏观视角：局势变化→势力消长→世界规则应激反应",
  cause:     "构建完整因果链：直接原因→结果→间接影响→蝴蝶效应（至少 3 层）",
};

// Kinds shown in the "选择要素" panel
const SELECTABLE_KINDS = [
  "character", "worldbuilding", "setting", "outline", "foreshadow",
] as const;

function buildGoal(
  scenario: string,
  types: SimType[],
  selectedMems: MemoryHit[],
  newSettings: string[],
): string {
  const focusList = types
    .map((t, i) => `${i + 1}. 【${SIM_TYPES.find((s) => s.key === t)!.label}】${TYPE_FOCUS[t]}`)
    .join("\n");

  const memSection = selectedMems.length > 0
    ? `\n\n## 用户指定参与推演的设定要素（直接使用，无需再召回这些条目）\n` +
      selectedMems.map(m =>
        `- **[${KIND_LABEL[m.kind as keyof typeof KIND_LABEL] ?? m.kind}] ${m.title}**：${m.summary}`
      ).join("\n")
    : "";

  const newSection = newSettings.length > 0
    ? `\n\n## 用户新增的设定（纳入推演，推演完成后必须自动录入设定集）\n` +
      newSettings.map((s, i) => `${i + 1}. ${s}`).join("\n")
    : "";

  const recallRule = selectedMems.length > 0
    ? "可补充调用 memory_recall 查询其他未列出的相关设定"
    : '必须先调用 memory_recall 查询"角色 人物"和"世界观 规则 约束"';

  const newSettingsRule = newSettings.length > 0
    ? `6. 推演完成后，对上方每条「用户新增设定」，分析其类型，调用 memory_save 录入：\n` +
      `   - kind 自动判断（character/worldbuilding/setting/foreshadow/lore/other）\n` +
      `   - title 提炼设定名称，summary 写一句话摘要，content 写完整设定\n`
    : "";

  return (
    `你是这部同人小说世界的全知模拟器。任务是中立客观地模拟事件走向，不是创作散文。` +
    memSection + newSection +
    `\n\n## ⚠️ 核心规则（违反任一条视为失败）\n` +
    `1. ${recallRule}\n` +
    `2. 不写散文，只输出结构化事件节点（编号列表，格式：[时序] 主体 → 行动 → 后果）\n` +
    `3. 严格遵守设定：角色性格不能 OOC，世界规则不能违反\n` +
    `4. 必须调用 memory_save 保存推演结果，参数完整且合法：kind="plot", ` +
    `title="推演：${scenario.slice(0, 20)}…", summary=一句话结论, content=完整事件节点, tags=["simulation"]\n` +
    `5. 至少推演 5 个事件节点\n` +
    newSettingsRule +
    `\n## 模拟场景\n「${scenario}」\n\n` +
    `## 本次推演维度\n${focusList}\n\n` +
    `现在开始：获取设定 → 推演事件节点 → 保存推演` +
    (newSettings.length > 0 ? ` → 录入新增设定` : "") +
    ` → 一句话总结。每步都必须调用工具。`
  );
}

interface ActiveModel { provider: string; model: string }

export default function SimulateWork({ onOpenSettings }: SimulateWorkProps) {
  const toast = useToast();

  // ── input state ──────────────────────────────────────────────────────────
  const [scenario, setScenario] = useState("");
  const [simTypes, setSimTypes] = useState<SimType[]>(["plot"]);

  // existing memory selection
  const [memoryItems, setMemoryItems] = useState<MemoryHit[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [loadingMem, setLoadingMem] = useState(false);

  // user-added new settings
  const [newSettings, setNewSettings] = useState<string[]>([]);
  const [newDraft, setNewDraft] = useState("");
  const [showMemoryOptions, setShowMemoryOptions] = useState(false);
  const [showNewSettings, setShowNewSettings] = useState(false);

  // ── run state ─────────────────────────────────────────────────────────────
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null);
  const [running, setRunning] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [cancelled, setCancelled] = useState(false);
  const [steps, setSteps] = useState<RunStep[]>([]);
  const [finished, setFinished] = useState(false);
  const [success, setSuccess] = useState<boolean | null>(null);
  const [finishNote, setFinishNote] = useState<string | null>(null);
  const [finalAnswer, setFinalAnswer] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [noProvider, setNoProvider] = useState(false);
  const [providerCompat, setProviderCompat] = useState(false);
  const stepSeq = useRef(0);
  const activeRequestRef = useRef<string | null>(null);
  const cancelRequestedRef = useRef(false);
  const runTailRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    return () => {
      if (activeRequestRef.current) void cancel(activeRequestRef.current);
    };
  }, []);

  // ── load provider ─────────────────────────────────────────────────────────
  useEffect(() => {
    if (!isDesktop()) return;
    let alive = true;
    const load = async () => {
      try {
        const s = await getProviders();
        const prov = s.providers.find((p) => p.id === s.active_provider);
        if (!alive) return;
        if (prov && s.active_model) {
          setActiveModel({ provider: prov.name || "（未命名）", model: s.active_model });
        } else setActiveModel(null);
      } catch {
        if (alive) setActiveModel(null);
      }
    };
    void load();
    const handleProvidersChanged = () => { void load(); };
    window.addEventListener(PROVIDERS_CHANGED_EVENT, handleProvidersChanged);
    return () => {
      alive = false;
      window.removeEventListener(PROVIDERS_CHANGED_EVENT, handleProvidersChanged);
    };
  }, []);

  // ── load memory items ─────────────────────────────────────────────────────
  const loadMemories = useCallback(async () => {
    if (!isDesktop()) return;
    setLoadingMem(true);
    try {
      const result = requireToolSuccess(
        await invokeTool<MemoryListData>("memory_list", {
          kinds: SELECTABLE_KINDS,
          limit: 500,
        }),
        "无法读取记忆库",
      );
      setMemoryItems(result.data.entries ?? []);
    } catch (loadError) {
      setMemoryItems([]);
      toast.err(`读取设定失败：${describeError(loadError)}`);
    }
    finally { setLoadingMem(false); }
  }, [toast]);

  useEffect(() => { void loadMemories(); }, [loadMemories]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      scrollLiveAnchor(runTailRef.current, { live: running });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [steps, running, cancelling, finishNote, error]);

  const toggleId = (id: string) =>
    setSelectedIds((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });

  const addNewSetting = () => {
    const t = newDraft.trim();
    if (!t) return;
    setNewSettings((prev) => [...prev, t]);
    setNewDraft("");
  };

  // ── simulation ────────────────────────────────────────────────────────────
  const startSimulation = useCallback(async () => {
    const s = scenario.trim();
    if (!s) { toast.err("请先描述要模拟的场景"); return; }
    if (simTypes.length === 0) { toast.err("请至少选择一种推演维度"); return; }

    const selectedMems = memoryItems.filter((m) => selectedIds.has(m.id));
    const goal = buildGoal(s, simTypes, selectedMems, newSettings);

    stepSeq.current = 0;
    setSteps([]); setFinished(false); setSuccess(null); setCancelled(false);
    setFinishNote(null); setFinalAnswer(null);
    setError(null); setNoProvider(false); setProviderCompat(false);
    setRunning(true);
    setCancelling(false);
    cancelRequestedRef.current = false;

    let pendingDelta: Extract<AgentStep, { phase: "delta" }> | null = null;
    let deltaFrame: number | null = null;
    const applyStep = (ev: AgentStep) => {
      setSteps((prev) => upsertStep(prev, ev, () => (stepSeq.current += 1)));
      if (ev.phase === "finish") {
        setRunning(false); setFinished(true); setSuccess(ev.success);
        setCancelled(ev.reason === "cancelled");
        setFinishNote(ev.success
          ? `推演完成（共 ${ev.steps} 步）`
          : `${stopReasonLabel(ev.reason)}（${ev.steps} 步）`);
      }
    };
    const flushStepFrame = () => {
      if (deltaFrame !== null) {
        window.cancelAnimationFrame(deltaFrame);
        deltaFrame = null;
      }
      const pending = pendingDelta;
      pendingDelta = null;
      if (pending) applyStep(pending);
    };
    const handleStep = (ev: AgentStep) => {
      if (ev.phase !== "delta") {
        flushStepFrame();
        applyStep(ev);
        return;
      }
      pendingDelta = pendingDelta?.step === ev.step
        ? { ...ev, delta: pendingDelta.delta + ev.delta }
        : ev;
      if (deltaFrame === null) {
        deltaFrame = window.requestAnimationFrame(flushStepFrame);
      }
    };

    const requestId = newRequestId("simulation");
    activeRequestRef.current = requestId;
    try {
      const result = await runGoalLive(
        goal,
        `推演·${simTypes.map((k) => SIM_TYPES.find((t) => t.key === k)!.label).join("+")}`,
        handleStep,
        undefined,
        "simulation",
        requestId,
      );
      flushStepFrame();
      if (cancelRequestedRef.current) {
        setSteps((prev) => settlePendingTools(prev, "cancelled"));
        setFinished(true);
        setSuccess(false);
        setCancelled(true);
        setFinishNote("已由用户停止");
        return;
      }
      if (result.outcome.stopped_reason !== "goal_reached") {
        setSteps((prev) => settlePendingTools(
          prev,
          result.outcome.stopped_reason === "cancelled" ? "cancelled" : "error",
        ));
        setFinished(true);
        setSuccess(false);
        setCancelled(result.outcome.stopped_reason === "cancelled");
        setFinishNote(stopReasonLabel(result.outcome.stopped_reason));
        return;
      }
      if (result.outcome.warning) toast.info(result.outcome.warning);
      setSteps((prev) => settlePendingTools(prev, "success"));
      setFinalAnswer(result.outcome.final_answer);
      const saved = result.session.messages.filter(
        (message) =>
          message.tool_result?.name === "memory_save" && message.tool_result.ok,
      ).length;
      toast.ok(saved > 0 ? `推演完成，已保存 ${saved} 条记录` : "推演结束");
      // Reload memories in case new ones were saved
      if (newSettings.length > 0 && saved > 0) void loadMemories();
    } catch (e) {
      flushStepFrame();
      const stopped = cancelRequestedRef.current;
      const msg = stopped ? "已由用户停止" : describeError(e);
      setRunning(false);
      if (stopped) {
        setSteps((prev) => settlePendingTools(prev, "cancelled", msg));
        setCancelled(true); setFinished(true); setSuccess(false);
        setFinishNote("已由用户停止"); setError(null);
      } else {
        setSteps((prev) => settlePendingTools(prev, "error", msg));
        setError(msg);
      }
      setNoProvider(!stopped && isNoProviderError(msg));
      setProviderCompat(!stopped && isProviderCompatibilityError(msg));
      if (!stopped && !isNoProviderError(msg) && !isProviderCompatibilityError(msg))
        toast.err(`推演失败：${msg}`);
    } finally {
      if (deltaFrame !== null) window.cancelAnimationFrame(deltaFrame);
      deltaFrame = null;
      pendingDelta = null;
      if (activeRequestRef.current === requestId) activeRequestRef.current = null;
      setRunning(false);
      setCancelling(false);
    }
  }, [scenario, simTypes, selectedIds, memoryItems, newSettings, toast, loadMemories]);

  const stopSimulation = useCallback(async () => {
    const requestId = activeRequestRef.current;
    if (!requestId || cancelling) return;
    cancelRequestedRef.current = true;
    setCancelling(true);
    try {
      await cancel(requestId);
    } catch (stopError) {
      cancelRequestedRef.current = false;
      setCancelling(false);
      toast.err(`停止失败：${describeError(stopError)}`);
    }
  }, [cancelling, toast]);

  // ── derived ───────────────────────────────────────────────────────────────
  const hasResult = running || steps.length > 0 || finished || error !== null;
  const guideStage = hasResult ? 3 : scenario.trim() ? 2 : 1;
  const phase = derivePhase({
    running,
    steps,
    finished,
    success,
    errored: error !== null,
    cancelling,
    cancelled,
  });
  const wf = workflowView(phase, reachedWorkflowStage(steps));
  const lastStep = steps.length > 0 ? steps[steps.length - 1].step : 0;
  const toolCount = steps.reduce((n, s) => n + s.toolCalls.length, 0);
  const resultTone = running ? "running" : success === true ? "success" : cancelled ? "cancelled" : error || finished ? "error" : "idle";
  const resultLabel = running ? "推演进行中" : success === true ? "推演完成" : cancelled ? "已停止" : error || finished ? "未完成" : "等待开始";

  // Group selectable memory items by kind
  const memGroups = SELECTABLE_KINDS
    .map((kind) => ({
      kind,
      label: KIND_LABEL[kind],
      items: memoryItems.filter((m) => m.kind === kind),
    }))
    .filter((g) => g.items.length > 0);

  return (
    <div className="work-content simulate2">

      {/* ── Left ─────────────────────────────────────────────────────────── */}
      <div className="simulate2__left">
        <section className="panel simulate2__input-panel">
          <div className="simulate2__intro">
            <p className="panel__kicker">世界模拟器 · 01</p>
            <h2 className="panel__title">模拟推演</h2>
          </div>
          <p className="panel__subtitle">
            用一个变化作为起点，快速看见接下来可能发生的事。先写场景，再决定要观察的方向。
          </p>

          {activeModel ? (
            <div className="planning2__model simulate2__model">
              <IconStar size={12} />
              当前模型：{activeModel.provider} · {activeModel.model}
            </div>
          ) : (
            <div className="planning2__no-model simulate2__model-warning">
              <IconProviders size={18} />
              <span>尚未配置模型。<button className="link-btn" onClick={onOpenSettings}>前往设置</button></span>
            </div>
          )}

          <ol className="simulate2__guide" aria-label="推演步骤">
            <li className={guideStage === 1 ? "is-current" : "is-done"}><span>1</span><b>描述场景</b></li>
            <li className={guideStage === 2 ? "is-current" : guideStage > 2 ? "is-done" : ""}><span>2</span><b>选择方向</b></li>
            <li className={guideStage === 3 ? "is-current" : ""}><span>3</span><b>开始推演</b></li>
          </ol>

          {/* Scenario */}
          <div className="simulate2__field">
            <label className="input-field">
              <span className="input-field__label simulate2__field-label">
                <span className="simulate2__step-index">1</span>
                <span>描述一个变化</span>
              </span>
              <textarea
                className="textarea"
                value={scenario}
                onChange={(e) => setScenario(e.target.value)}
                placeholder={"例如：主角在第三章拒绝了导师的邀请……\n例如：反派提前得知了主角的计划……"}
                rows={4}
                spellCheck={false}
              />
            </label>
            <p className="simulate2__field-hint">写清楚“谁在什么时间做了什么”，不必提前写结局。</p>
          </div>

          {/* Simulation types */}
          <div className="simulate2__section-heading">
            <span className="simulate2__step-index">2</span>
            <div>
              <p className="simulate2__section-title">选择推演方向</p>
              <p className="simulate2__section-caption">可多选，至少保留一个方向</p>
            </div>
          </div>
          <div className="simulate2__types">
            {SIM_TYPES.map((t) => {
              const active = simTypes.includes(t.key);
              return (
                <button
                  key={t.key}
                  type="button"
                  className={`simulate2__type-btn${active ? " is-active" : ""}`}
                  aria-pressed={active}
                  onClick={() => {
                    if (running) return;
                    setSimTypes((prev) =>
                      prev.includes(t.key)
                        ? prev.length > 1 ? prev.filter((k) => k !== t.key) : prev
                        : [...prev, t.key]
                    );
                  }}
                  disabled={running}
                >
                  <div className="simulate2__type-row">
                    <div className={`simulate2__checkbox${active ? " is-checked" : ""}`}>
                      {active && <svg width="10" height="8" viewBox="0 0 10 8" fill="none">
                        <path d="M1 4L3.5 6.5L9 1" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
                      </svg>}
                    </div>
                    <span className="simulate2__type-label">{t.label}</span>
                  </div>
                  <span className="simulate2__type-blurb">{t.blurb}</span>
                </button>
              );
            })}
          </div>

          {/* Optional memory selection */}
          <div className="simulate2__optional">
            <button
              type="button"
              className="simulate2__optional-trigger"
              onClick={() => setShowMemoryOptions((open) => !open)}
              aria-expanded={showMemoryOptions}
              aria-controls="simulate-memory-options"
            >
              <span className="simulate2__optional-title">
                <span className="simulate2__step-index">+</span>
                <span>
                  <strong>指定已有设定</strong>
                  <small>可选 · 从记忆库挑选角色、世界规则或伏笔</small>
                </span>
              </span>
              <span className="simulate2__optional-meta">
                {selectedIds.size > 0 ? `${selectedIds.size} 项已选` : "稍后再选"}
                <span className="simulate2__optional-chevron" aria-hidden="true">{showMemoryOptions ? "−" : "+"}</span>
              </span>
            </button>

            {showMemoryOptions && (
              <div id="simulate-memory-options" className="simulate2__optional-body">
                <div className="simulate2__memory-toolbar">
                  <span>{loadingMem ? "正在读取设定…" : "勾选后会直接带入本次推演"}</span>
                  <button
                    type="button"
                    className="btn btn--ghost btn--xs"
                    onClick={() => void loadMemories()}
                    disabled={loadingMem || running}
                    title="刷新设定列表"
                  >
                    {loadingMem ? <Spinner size={12} /> : <IconRefresh size={12} />} 刷新
                  </button>
                </div>

                {memGroups.length === 0 && !loadingMem && (
                  <p className="simulate2__hint">暂无可选设定。你可以先在「策划」中生成，或直接开始推演。</p>
                )}

                {memGroups.map((g) => (
                  <div key={g.kind} className="simulate2__mem-group">
                    <div className="simulate2__mem-group-label">
                      <span>{g.label}</span>
                      <button
                        type="button"
                        className="simulate2__group-toggle"
                        disabled={running}
                        onClick={() => {
                          const allSelected = g.items.every((m) => selectedIds.has(m.id));
                          setSelectedIds((prev) => {
                            const next = new Set(prev);
                            g.items.forEach((m) => allSelected ? next.delete(m.id) : next.add(m.id));
                            return next;
                          });
                        }}
                      >
                        {g.items.every((m) => selectedIds.has(m.id)) ? "取消全选" : "全选"}
                      </button>
                    </div>
                    <div className="simulate2__mem-chips">
                      {g.items.map((m) => (
                        <button
                          key={m.id}
                          type="button"
                          className={`simulate2__chip${selectedIds.has(m.id) ? " is-selected" : ""}`}
                          onClick={() => toggleId(m.id)}
                          disabled={running}
                          title={m.summary}
                        >
                          {m.title}
                        </button>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Optional new settings */}
          <div className="simulate2__optional">
            <button
              type="button"
              className="simulate2__optional-trigger"
              onClick={() => setShowNewSettings((open) => !open)}
              aria-expanded={showNewSettings}
              aria-controls="simulate-new-settings"
            >
              <span className="simulate2__optional-title">
                <span className="simulate2__step-index">+</span>
                <span>
                  <strong>补充一条新设定</strong>
                  <small>可选 · 推演完成后自动归档到记忆库</small>
                </span>
              </span>
              <span className="simulate2__optional-meta">
                {newSettings.length > 0 ? `${newSettings.length} 条待录入` : "现在不补充"}
                <span className="simulate2__optional-chevron" aria-hidden="true">{showNewSettings ? "−" : "+"}</span>
              </span>
            </button>

            {showNewSettings && (
              <div id="simulate-new-settings" className="simulate2__optional-body">
                <p className="simulate2__hint">例如：林惊羽获得了风系灵根。每次输入一条，按 Enter 添加。</p>
                <div className="simulate2__new-setting-input">
                  <input
                    className="input"
                    placeholder="输入新设定内容…"
                    value={newDraft}
                    onChange={(e) => setNewDraft(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); addNewSetting(); } }}
                    disabled={running}
                  />
                  <button
                    type="button"
                    className="btn btn--primary btn--icon"
                    onClick={addNewSetting}
                    disabled={!newDraft.trim() || running}
                    aria-label="添加设定"
                  >
                    <IconPlus size={15} />
                  </button>
                </div>

                {newSettings.length > 0 && (
                  <div className="simulate2__new-list">
                    {newSettings.map((s, i) => (
                      <div key={i} className="simulate2__new-item">
                        <span className="simulate2__new-text">{s}</span>
                        <button
                          type="button"
                          className="simulate2__new-del"
                          onClick={() => setNewSettings((prev) => prev.filter((_, j) => j !== i))}
                          disabled={running}
                          aria-label={`移除设定 ${i + 1}`}
                        >
                          <IconClose size={12} />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>

          <div className="simulate2__run-actions">
            <div className="simulate2__run-copy">
              <span className="simulate2__run-title">{running ? "正在观察事件链" : "准备好了吗？"}</span>
              <span className="simulate2__run-hint">
                {running ? "可以随时停止，已完成的步骤会保留" : !activeModel ? "先在设置中配置一个模型" : !scenario.trim() ? "先写下要模拟的场景" : "AI 会按所选方向生成结构化事件"}
              </span>
            </div>
            <button
              type="button"
              className={`btn simulate2__run-btn ${running ? "btn--danger" : "btn--primary"}`}
              onClick={() => running ? void stopSimulation() : void startSimulation()}
              disabled={running ? cancelling : !scenario.trim() || !activeModel}
            >
              {running ? (
                <>{cancelling ? <Spinner size={15} /> : <IconStop size={15} />} {cancelling ? "停止中…" : "停止推演"}</>
              ) : (
                <><IconSimulate size={15} /> 开始推演</>
              )}
            </button>
          </div>
        </section>
      </div>

      {/* ── Right ────────────────────────────────────────────────────────── */}
      <div className="simulate2__right">
        {!hasResult && (
          <div className="simulate2__empty">
            <div className="simulate2__empty-mark"><IconSimulate size={30} /></div>
            <div className="simulate2__empty-copy">
              <p className="simulate2__empty-kicker">等待你的第一个变化</p>
              <h3>从场景到事件链</h3>
              <p className="simulate2__empty-hint">推演会把一个场景拆成连续的事件节点，帮助你提前发现冲突、后果和新的可能。</p>
            </div>
            <div className="simulate2__empty-steps">
              <div className="simulate2__empty-step"><span>01</span><div><strong>写下场景</strong><p>一句话说清楚发生了什么</p></div></div>
              <div className="simulate2__empty-step"><span>02</span><div><strong>选观察方向</strong><p>情节、角色、世界或因果链</p></div></div>
              <div className="simulate2__empty-step"><span>03</span><div><strong>开始推演</strong><p>实时查看每个事件节点</p></div></div>
            </div>
            <p className="simulate2__empty-note">已有设定和新增设定都可以稍后补充，不会挡住你的第一步。</p>
          </div>
        )}

        {hasResult && (
          <section className="panel simulate2__result">
            <header className="simulate2__result-header">
              <div>
                <p className="simulate2__result-kicker">事件链 · 实时记录</p>
                <h3>推演结果</h3>
                <p className="simulate2__result-scenario" title={scenario.trim()}>{scenario.trim()}</p>
              </div>
              <span className={`simulate2__result-state is-${resultTone}`}>
                {running && <Spinner size={12} />}
                {!running && success && <IconCheck size={12} />}
                {!running && success !== true && (error || cancelled || finished) && <IconWarn size={12} />}
                {resultLabel}
              </span>
            </header>
            {(noProvider || providerCompat) ? (
              <div className="studio2__notice-err"><IconWarn size={14} /> {error}</div>
            ) : (
              <>
                {(running || steps.length > 0) && (
                  <div className="agent-console">
                    <WorkflowSteps stages={SIMULATE_STAGES} current={wf.current} state={wf.state} />
                    <WorkStatus phase={phase} step={lastStep} toolCount={toolCount} />
                  </div>
                )}
                {(running || steps.length > 0) && (
                  <AgentFeed
                    steps={steps}
                    running={running}
                    phase={phase}
                    pendingText="AI 正在推演下一个事件节点…"
                    tailRef={runTailRef}
                  />
                )}
                {!running && finalAnswer && (
                  <div className="simulate2__answer">
                    <p className="simulate2__answer-label">一句话结论</p>
                    <div className="simulate2__answer-text">{finalAnswer}</div>
                  </div>
                )}
                {!running && error && !noProvider && !providerCompat && (
                  <div className="studio2__notice-err"><IconWarn size={14} /> {error}</div>
                )}
                {finishNote && (
                  <div className={`planning2__finish${success ? "" : " planning2__finish--warn"}`}>
                    {success ? <IconCheck size={14} /> : <IconWarn size={14} />} {finishNote}
                  </div>
                )}
              </>
            )}
          </section>
        )}
      </div>
    </div>
  );
}
