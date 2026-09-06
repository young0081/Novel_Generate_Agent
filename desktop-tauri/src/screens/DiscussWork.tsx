// 探讨工作台: native Agent conversation with visible reasoning and tool use.

import { useCallback, useEffect, useRef, useState } from "react";
import { Spinner } from "../components/Spinner";
import AgentFeed from "../components/agent/AgentFeed";
import WorkStatus from "../components/agent/WorkStatus";
import WorkflowSteps from "../components/agent/WorkflowSteps";
import {
  IconAgentMode,
  IconArrowRight,
  IconBrush,
  IconChat,
  IconCheck,
  IconChevron,
  IconCompass,
  IconProviders,
  IconRefresh,
  IconSearch,
  IconSettings,
  IconStar,
  IconStop,
  IconThought,
  IconTools,
  IconUser,
} from "../components/icons";
import { cancel, describeError, newRequestId } from "../lib/core";
import { useToast } from "../components/Toast";
import {
  runGoalLive,
  type AgentStep,
  type Message,
  type ThinkingLevel,
} from "../lib/studio";
import {
  DISCUSS_STAGES,
  derivePhase,
  isNoProviderError,
  isProviderCompatibilityError,
  reachedWorkflowStage,
  settlePendingTools,
  stopReasonLabel,
  upsertStep,
  workflowView,
  type AgentPhase,
  type RunStep,
} from "../lib/agentRun";
import {
  getProviders,
  PROVIDERS_CHANGED_EVENT,
  type SamplingParams,
} from "../lib/providers";
import { getSession } from "../lib/sessions";
import { scrollLiveAnchor } from "../lib/liveScroll";

interface DiscussWorkProps {
  onOpenSettings: () => void;
  initialSessionId?: string;
}

type TurnStatus = "complete" | "cancelled" | "stopped" | "error";

interface TurnTrace {
  steps: RunStep[];
  status: TurnStatus;
  note: string;
}

interface Turn {
  id: number;
  role: "user" | "assistant";
  content: string;
  status?: TurnStatus;
  error?: string;
  trace?: TurnTrace;
}

interface ActiveModel {
  provider: string;
  model: string;
}

const DEFAULT_TEMPERATURE = 1;
const DEFAULT_TOP_P = 1;
const DISCUSS_MESSAGE_MARKER = "\n\n【作者本轮消息】\n";

const QUICK_TOPICS = [
  { label: "人物深化", prompt: "帮我深化一下主角的性格与成长弧线" },
  { label: "情节推演", prompt: "我想探讨一下接下来的剧情走向，有哪些可能性？" },
  { label: "冲突设计", prompt: "这个故事的核心冲突应该如何设计？" },
  { label: "伏笔布局", prompt: "我想在前期埋下哪些伏笔比较合适？" },
];

const THINKING_OPTIONS: Array<{
  value: ThinkingLevel;
  label: string;
  title: string;
}> = [
  { value: "light", label: "轻", title: "快速回答，少量核验" },
  { value: "balanced", label: "均衡", title: "兼顾速度与推演深度" },
  { value: "deep", label: "深", title: "分解问题并充分核验" },
];

function buildDiscussionGoal(concept: string, message: string): string {
  const anchor = concept.trim();
  if (!anchor) return message;
  return `【当前构思锚点】\n${anchor}${DISCUSS_MESSAGE_MARKER}${message}`;
}

function visibleUserMessage(content: string): string {
  const markerIndex = content.indexOf(DISCUSS_MESSAGE_MARKER);
  return markerIndex >= 0
    ? content.slice(markerIndex + DISCUSS_MESSAGE_MARKER.length).trim()
    : content;
}

function discussionTitle(message: string): string {
  const oneLine = message.replace(/\s+/g, " ").trim();
  const clipped = Array.from(oneLine).slice(0, 20).join("");
  return clipped.length < oneLine.length ? `${clipped}…` : clipped || "探讨";
}

function lastAssistantAnswer(messages: Message[]): string {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (
      message.role === "assistant" &&
      !message.tool_call &&
      message.content.trim()
    ) {
      return message.content;
    }
  }
  return "";
}

function tracePhase(trace: TurnTrace): AgentPhase {
  switch (trace.status) {
    case "complete": return "done";
    case "cancelled": return "cancelled";
    case "stopped": return "stopped";
    case "error": return "error";
  }
}

function toolCount(steps: RunStep[]): number {
  return steps.reduce((count, step) => count + step.toolCalls.length, 0);
}

function applyTerminalSteps(
  steps: RunStep[],
  status: TurnStatus,
  message?: string,
): RunStep[] {
  return settlePendingTools(
    steps,
    status === "complete" ? "success" : status === "cancelled" ? "cancelled" : "error",
    message,
  );
}

function HistoricalTrace({
  trace,
  initiallyOpen,
}: {
  trace: TurnTrace;
  initiallyOpen: boolean;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  const phase = tracePhase(trace);
  const workflow = workflowView(phase, reachedWorkflowStage(trace.steps));
  const traceToolCount = toolCount(trace.steps);
  const lastStep = trace.steps.length > 0
    ? trace.steps[trace.steps.length - 1].step
    : undefined;

  return (
    <details
      className={`discuss__trace is-${trace.status}`}
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>
        <span><IconThought size={13} />Agent 过程</span>
        <span>
          {trace.steps.length} 步
          {traceToolCount > 0 ? ` / ${traceToolCount} 次工具` : ""}
          <IconChevron size={13} />
        </span>
      </summary>
      <div className="discuss__trace-body">
        <WorkStatus
          phase={phase}
          step={lastStep}
          toolCount={traceToolCount}
          note={trace.note}
        />
        <WorkflowSteps
          stages={DISCUSS_STAGES}
          current={workflow.current}
          state={workflow.state}
        />
        {trace.steps.length > 0 && (
          <AgentFeed
            steps={trace.steps}
            running={false}
            phase={phase}
          />
        )}
      </div>
    </details>
  );
}

export default function DiscussWork({ onOpenSettings, initialSessionId }: DiscussWorkProps) {
  const toast = useToast();
  const [concept, setConcept] = useState("");
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState("");
  const [running, setRunning] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [runSteps, setRunSteps] = useState<RunStep[]>([]);
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null);
  const [loadingModel, setLoadingModel] = useState(true);
  const [sessionId, setSessionId] = useState<string | null>(initialSessionId ?? null);
  const [loadingSession, setLoadingSession] = useState(!!initialSessionId);
  const [showSavedHint, setShowSavedHint] = useState(false);
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel>("balanced");
  const [temperature, setTemperature] = useState(DEFAULT_TEMPERATURE);
  const [topP, setTopP] = useState(DEFAULT_TOP_P);
  const [controlsOpen, setControlsOpen] = useState(false);

  const savedHintTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeRequestRef = useRef<string | null>(null);
  const cancelRequestedRef = useRef(false);
  const stepKeyRef = useRef(0);
  const turnKeyRef = useRef(0);
  const runStepsRef = useRef<RunStep[]>([]);
  const pendingDeltaRef = useRef<Extract<AgentStep, { phase: "delta" }> | null>(null);
  const streamFrameRef = useRef<number | null>(null);
  const tailRef = useRef<HTMLDivElement>(null);

  const nextTurnId = useCallback(() => {
    turnKeyRef.current += 1;
    return turnKeyRef.current;
  }, []);

  const applyStepNow = useCallback((event: AgentStep) => {
    const next = upsertStep(
      runStepsRef.current,
      event,
      () => (stepKeyRef.current += 1),
    );
    runStepsRef.current = next;
    setRunSteps(next);
  }, []);

  const cancelAgentFrame = useCallback(() => {
    if (streamFrameRef.current !== null) {
      window.cancelAnimationFrame(streamFrameRef.current);
      streamFrameRef.current = null;
    }
    pendingDeltaRef.current = null;
  }, []);

  const flushAgentFrame = useCallback(() => {
    if (streamFrameRef.current !== null) {
      window.cancelAnimationFrame(streamFrameRef.current);
      streamFrameRef.current = null;
    }
    const pending = pendingDeltaRef.current;
    pendingDeltaRef.current = null;
    if (pending) applyStepNow(pending);
  }, [applyStepNow]);

  const handleAgentStep = useCallback((event: AgentStep) => {
    if (event.phase !== "delta") {
      flushAgentFrame();
      applyStepNow(event);
      return;
    }
    const pending = pendingDeltaRef.current;
    pendingDeltaRef.current = pending?.step === event.step
      ? { ...event, delta: pending.delta + event.delta }
      : event;
    if (streamFrameRef.current === null) {
      streamFrameRef.current = window.requestAnimationFrame(flushAgentFrame);
    }
  }, [applyStepNow, flushAgentFrame]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      scrollLiveAnchor(tailRef.current, { live: running });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [turns, runSteps, running, cancelling]);

  useEffect(() => {
    return () => {
      if (savedHintTimer.current) clearTimeout(savedHintTimer.current);
      cancelAgentFrame();
      const requestId = activeRequestRef.current;
      activeRequestRef.current = null;
      if (requestId) void cancel(requestId);
    };
  }, [cancelAgentFrame]);

  useEffect(() => {
    if (!initialSessionId) {
      setLoadingSession(false);
      return;
    }
    let alive = true;
    setLoadingSession(true);
    setTurns([]);
    setSessionId(initialSessionId);
    void getSession(initialSessionId)
      .then((record) => {
        if (!alive) return;
        if (record.kind !== "discuss") {
          throw new Error("该会话不是探讨记录，无法在此继续");
        }
        const restored: Turn[] = record.session.messages
          .filter((message) => (
            (message.role === "user" || message.role === "assistant") &&
            !message.tool_call &&
            !!message.content.trim()
          ))
          .map((message) => ({
            id: nextTurnId(),
            role: message.role as "user" | "assistant",
            content: message.role === "user"
              ? visibleUserMessage(message.content)
              : message.content,
            status: message.role === "assistant" ? "complete" : undefined,
          }));
        setTurns(restored);
        setSessionId(record.session.id);
      })
      .catch((error) => {
        if (!alive) return;
        setSessionId(null);
        toast.err(`载入会话失败：${describeError(error)}`);
      })
      .finally(() => {
        if (alive) setLoadingSession(false);
      });
    return () => {
      alive = false;
    };
  }, [initialSessionId, nextTurnId, toast]);

  useEffect(() => {
    let alive = true;
    const load = async () => {
      setLoadingModel(true);
      try {
        const settings = await getProviders();
        const active = settings.providers.find(
          (provider) => provider.id === settings.active_provider,
        );
        if (!alive) return;
        if (active && settings.active_model) {
          setActiveModel({
            provider: active.name || "未命名供应商",
            model: settings.active_model,
          });
          const sampling = active.sampling;
          setTemperature(sampling?.temperature ?? DEFAULT_TEMPERATURE);
          setTopP(sampling?.top_p ?? DEFAULT_TOP_P);
        } else {
          setActiveModel(null);
        }
      } catch {
        if (alive) setActiveModel(null);
      } finally {
        if (alive) setLoadingModel(false);
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

  const showSaved = useCallback(() => {
    setShowSavedHint(true);
    if (savedHintTimer.current) clearTimeout(savedHintTimer.current);
    savedHintTimer.current = setTimeout(() => setShowSavedHint(false), 2500);
  }, []);

  const sendMessage = useCallback(async (text?: string) => {
    const message = (text ?? draft).trim();
    if (!message || running || loadingSession) return;
    if (!activeModel) {
      toast.err("请先配置一个模型供应商");
      return;
    }

    setTurns((previous) => [
      ...previous,
      { id: nextTurnId(), role: "user", content: message },
    ]);
    setDraft("");
    setRunning(true);
    setCancelling(false);
    cancelRequestedRef.current = false;
    cancelAgentFrame();
    runStepsRef.current = [];
    setRunSteps([]);
    stepKeyRef.current = 0;

    const requestId = newRequestId("discuss-agent");
    activeRequestRef.current = requestId;
    const sampling: SamplingParams = {
      temperature,
      top_p: topP,
      top_k: 0,
      presence_penalty: 0,
      frequency_penalty: 0,
    };

    try {
      const result = await runGoalLive(
        buildDiscussionGoal(concept, message),
        discussionTitle(message),
        handleAgentStep,
        sessionId ?? undefined,
        "discuss",
        requestId,
        { sampling, thinkingLevel },
      );
      flushAgentFrame();
      setSessionId(result.session.id);

      const stoppedReason = result.outcome.stopped_reason;
      const wasCancelled = cancelRequestedRef.current || stoppedReason === "cancelled";
      const status: TurnStatus = wasCancelled
        ? "cancelled"
        : stoppedReason === "goal_reached"
          ? "complete"
          : "stopped";
      const note = status === "complete"
        ? `回应完成（共 ${result.outcome.steps} 步）`
        : status === "cancelled"
          ? "已由用户停止"
          : `${stopReasonLabel(stoppedReason)}（共 ${result.outcome.steps} 步）`;
      const settled = applyTerminalSteps(runStepsRef.current, status);
      const answer = result.outcome.final_answer?.trim()
        ? result.outcome.final_answer
        : lastAssistantAnswer(result.session.messages);

      setTurns((previous) => [
        ...previous,
        {
          id: nextTurnId(),
          role: "assistant",
          content: answer,
          status,
          trace: { steps: settled, status, note },
        },
      ]);
      if (result.outcome.warning) toast.info(result.outcome.warning);
      if (status === "cancelled") toast.info("已停止本轮探讨");
      if (status === "stopped") toast.info("本轮 Agent 未完整结束，可调整问题后继续");
      showSaved();
    } catch (error) {
      flushAgentFrame();
      const stopped = cancelRequestedRef.current;
      const messageText = stopped ? "已由用户停止" : describeError(error);
      const status: TurnStatus = stopped ? "cancelled" : "error";
      const settled = applyTerminalSteps(runStepsRef.current, status, messageText);
      setTurns((previous) => [
        ...previous,
        {
          id: nextTurnId(),
          role: "assistant",
          content: "",
          status,
          error: stopped ? undefined : messageText,
          trace: {
            steps: settled,
            status,
            note: stopped ? "已由用户停止" : "本轮回应中断",
          },
        },
      ]);
      if (!stopped) toast.err(`探讨失败：${messageText}`);
    } finally {
      cancelAgentFrame();
      runStepsRef.current = [];
      setRunSteps([]);
      if (activeRequestRef.current === requestId) activeRequestRef.current = null;
      setRunning(false);
      setCancelling(false);
      cancelRequestedRef.current = false;
    }
  }, [
    activeModel,
    cancelAgentFrame,
    concept,
    draft,
    flushAgentFrame,
    handleAgentStep,
    loadingSession,
    nextTurnId,
    running,
    sessionId,
    showSaved,
    temperature,
    thinkingLevel,
    toast,
    topP,
  ]);

  const stopRunning = useCallback(async () => {
    const requestId = activeRequestRef.current;
    if (!requestId || cancelling) return;
    cancelRequestedRef.current = true;
    setCancelling(true);
    flushAgentFrame();
    try {
      await cancel(requestId);
    } catch (error) {
      cancelRequestedRef.current = false;
      setCancelling(false);
      toast.err(`停止失败：${describeError(error)}`);
    }
  }, [cancelling, flushAgentFrame, toast]);

  const clearConversation = useCallback(() => {
    if (running) return;
    cancelAgentFrame();
    setTurns([]);
    setRunSteps([]);
    runStepsRef.current = [];
    setSessionId(null);
    setShowSavedHint(false);
    toast.ok("已清空对话");
  }, [cancelAgentFrame, running, toast]);

  const phase = derivePhase({
    running,
    steps: runSteps,
    finished: false,
    success: null,
    errored: false,
    cancelling,
  });
  const workflow = workflowView(phase, reachedWorkflowStage(runSteps));
  const currentToolCount = toolCount(runSteps);
  const currentStep = runSteps.length > 0 ? runSteps[runSteps.length - 1].step : 0;
  const modelDisplay = activeModel
    ? `${activeModel.provider} / ${activeModel.model}`
    : "未配置";

  return (
    <div className="discuss">
      <header className="discuss__anchor">
        <div className="discuss__anchor-inner">
          <div className="discuss__anchor-row">
            <div className="discuss__anchor-label">
              <IconStar size={12} />
              当前构思
            </div>
            <div className="discuss__anchor-meta">
              {loadingModel ? (
                <span className="discuss__model-loading">
                  <Spinner size={11} />
                  正在读取模型
                </span>
              ) : activeModel ? (
                <span className="discuss__model-active" title={modelDisplay}>
                  <IconProviders size={11} />
                  {modelDisplay}
                </span>
              ) : (
                <button type="button" className="link-btn" onClick={onOpenSettings}>
                  <IconProviders size={11} />
                  配置模型
                </button>
              )}
              {showSavedHint && (
                <span className="discuss__saved-hint" role="status">
                  <IconCheck size={10} />
                  已自动存档
                </span>
              )}
            </div>
          </div>
          <textarea
            className="discuss__concept-input"
            value={concept}
            onChange={(event) => setConcept(event.target.value)}
            placeholder="简述故事构思，Agent 会把它作为每轮探讨的锚点…"
            rows={2}
            spellCheck={false}
            disabled={running}
          />
          {turns.length > 0 && (
            <div className="discuss__anchor-actions">
              <button
                type="button"
                className="btn btn--ghost btn--sm"
                onClick={clearConversation}
                disabled={running}
              >
                <IconRefresh size={13} />
                清空对话
              </button>
            </div>
          )}
        </div>
      </header>

      <div className="discuss__workspace">
        <aside className={`discuss__controls${controlsOpen ? " is-open" : ""}`}>
          <button
            type="button"
            className="discuss__controls-toggle"
            onClick={() => setControlsOpen((open) => !open)}
            aria-expanded={controlsOpen}
            aria-controls="discuss-agent-controls"
            aria-label={controlsOpen ? "收起 Agent 控制" : "展开 Agent 控制"}
          >
            <span>
              <IconAgentMode size={15} />
              Agent 控制
            </span>
            <IconChevron size={15} />
          </button>

          <div className="discuss__controls-body" id="discuss-agent-controls">
            <section className="discuss__control-section" aria-labelledby="discuss-model-title">
              <div className="discuss__control-title" id="discuss-model-title">
                <IconProviders size={14} />
                当前模型
              </div>
              <div className="discuss__model-card">
                {loadingModel ? (
                  <span className="discuss__control-muted">正在读取配置…</span>
                ) : activeModel ? (
                  <>
                    <strong>{activeModel.provider}</strong>
                    <code title={activeModel.model}>{activeModel.model}</code>
                  </>
                ) : (
                  <span className="discuss__control-warning">尚未配置模型</span>
                )}
                <button
                  type="button"
                  className="discuss__settings-link"
                  onClick={onOpenSettings}
                  title="打开模型设置"
                >
                  <IconSettings size={13} />
                  模型设置
                </button>
              </div>
            </section>

            <section className="discuss__control-section" aria-labelledby="discuss-thinking-title">
              <div className="discuss__control-title" id="discuss-thinking-title">
                <IconThought size={14} />
                思考强度
              </div>
              <div className="discuss__thinking-tabs" role="group" aria-label="思考强度">
                {THINKING_OPTIONS.map((option) => (
                  <button
                    type="button"
                    key={option.value}
                    className={thinkingLevel === option.value ? "is-active" : ""}
                    onClick={() => setThinkingLevel(option.value)}
                    aria-pressed={thinkingLevel === option.value}
                    title={option.title}
                    disabled={running}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
            </section>

            <section className="discuss__control-section" aria-labelledby="discuss-sampling-title">
              <div className="discuss__control-title" id="discuss-sampling-title">
                <IconBrush size={14} />
                生成参数
              </div>
              <label className="discuss__slider-field">
                <span>
                  Temperature
                  <output>{temperature.toFixed(2)}</output>
                </span>
                <input
                  className="slider"
                  type="range"
                  min="0"
                  max="2"
                  step="0.05"
                  value={temperature}
                  onChange={(event) => setTemperature(Number(event.target.value))}
                  disabled={running}
                  aria-valuetext={temperature.toFixed(2)}
                />
                <small><span>稳定</span><span>灵活</span></small>
              </label>
              <label className="discuss__slider-field">
                <span>
                  Top-P
                  <output>{topP.toFixed(2)}</output>
                </span>
                <input
                  className="slider"
                  type="range"
                  min="0"
                  max="1"
                  step="0.05"
                  value={topP}
                  onChange={(event) => setTopP(Number(event.target.value))}
                  disabled={running}
                  aria-valuetext={topP.toFixed(2)}
                />
                <small><span>聚焦</span><span>发散</span></small>
              </label>
            </section>

            <section className="discuss__control-section" aria-labelledby="discuss-ecosystem-title">
              <div className="discuss__control-title" id="discuss-ecosystem-title">
                <IconAgentMode size={14} />
                原生能力
              </div>
              <div className="discuss__capability-row">
                <span className="discuss__capability-icon"><IconSearch size={14} /></span>
                <span><strong>知识库</strong><small>每轮自动检索</small></span>
                <em>{running ? "已接入" : "待命"}</em>
              </div>
              <div className="discuss__capability-row">
                <span className="discuss__capability-icon"><IconTools size={14} /></span>
                <span><strong>内置工具</strong><small>由 Agent 按需调用</small></span>
                <em>{currentToolCount > 0 ? `${currentToolCount} 次` : "待命"}</em>
              </div>
            </section>
          </div>
        </aside>

        <main className="discuss__main">
          <div className="discuss__stream">
            {loadingSession ? (
              <div className="discuss__empty" role="status">
                <Spinner size={24} />
                <p>正在载入会话…</p>
              </div>
            ) : turns.length === 0 && !running ? (
              <div className="discuss__empty">
                <div className="discuss__empty-icon">
                  <IconChat size={32} />
                </div>
                <h3>与 Agent 推演你的故事</h3>
                <p>深化人物、推演情节、核对设定，也可以让它按需调用知识库与工具。</p>
                <div className="discuss__quick-topics">
                  {QUICK_TOPICS.map((topic) => (
                    <button
                      type="button"
                      key={topic.label}
                      className="discuss__topic-chip"
                      onClick={() => void sendMessage(topic.prompt)}
                      disabled={!activeModel || running}
                    >
                      {topic.label}
                    </button>
                  ))}
                </div>
                {activeModel ? (
                  <div className="discuss__flow-hint">
                    <IconArrowRight size={13} />
                    Agent 会先理解问题，再决定是否检索或调用工具
                  </div>
                ) : (
                  <button type="button" className="btn btn--primary" onClick={onOpenSettings}>
                    <IconProviders size={14} />
                    配置模型
                  </button>
                )}
              </div>
            ) : null}

            {turns.map((turn, index) => {
              const trace = turn.trace;
              return (
                <article
                  key={turn.id}
                  className={`discuss__turn discuss__turn--${turn.role}`}
                  data-turn-status={turn.status}
                >
                  {trace && (
                    <HistoricalTrace
                      trace={trace}
                      initiallyOpen={index === turns.length - 1}
                    />
                  )}

                  <div
                    className={`discuss__bubble discuss__bubble--${turn.role}${turn.status ? ` is-${turn.status}` : ""}`}
                  >
                    <div className="discuss__bubble-who">
                      {turn.role === "user" ? (
                        <><IconUser size={14} />你</>
                      ) : (
                        <><IconCompass size={14} />Agent</>
                      )}
                    </div>
                    {turn.content && <div className="discuss__bubble-text">{turn.content}</div>}
                    {turn.status === "cancelled" && (
                      <div className="discuss__bubble-state is-cancelled" role="status">
                        已停止本轮探讨{turn.content ? "，上方为已生成内容" : ""}
                      </div>
                    )}
                    {turn.status === "stopped" && (
                      <div className="discuss__bubble-state is-stopped" role="status">
                        本轮未完整结束，可继续追问或调整思考强度
                      </div>
                    )}
                    {turn.status === "error" && (
                      <div className="discuss__bubble-state is-error" role="alert">
                        <strong>回应中断</strong>
                        <span>{turn.error || "连接模型时发生错误"}</span>
                        {(isNoProviderError(turn.error ?? "") ||
                          isProviderCompatibilityError(turn.error ?? "")) && (
                          <button type="button" className="link-btn" onClick={onOpenSettings}>
                            检查模型设置
                          </button>
                        )}
                      </div>
                    )}
                    {turn.role === "assistant" && turn.status === "complete" && (
                      <span className="a11y-only" role="status">Agent 回应完成</span>
                    )}
                  </div>
                </article>
              );
            })}

            {running && (
              <section className="discuss__agent-run" aria-label="本轮 Agent 过程">
                <div className="discuss__agent-overview">
                  <WorkStatus
                    phase={phase}
                    step={currentStep}
                    toolCount={currentToolCount}
                  />
                  <WorkflowSteps
                    stages={DISCUSS_STAGES}
                    current={workflow.current}
                    state={workflow.state}
                  />
                </div>
                <AgentFeed
                  steps={runSteps}
                  running={running}
                  phase={phase}
                  pendingText="Agent 正在结合构思与作品上下文运思…"
                />
              </section>
            )}
            <div ref={tailRef} />
          </div>

          <div className="discuss__composer">
            <textarea
              className="discuss__input"
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  void sendMessage();
                }
              }}
              placeholder={activeModel ? "说说你的想法，或让 Agent 查找相关设定…" : "请先配置模型"}
              rows={3}
              disabled={loadingSession || running || !activeModel}
              spellCheck={false}
            />
            <div className="discuss__composer-actions">
              <div className="discuss__hint">
                <kbd>Enter</kbd> 发送 · <kbd>Shift + Enter</kbd> 换行
              </div>
              <button
                type="button"
                className={`btn ${running ? "btn--danger" : "btn--primary"} discuss__send-btn`}
                onClick={() => running ? void stopRunning() : void sendMessage()}
                disabled={
                  loadingSession ||
                  (!running && (!draft.trim() || !activeModel)) ||
                  cancelling
                }
              >
                {cancelling ? (
                  <Spinner size={14} />
                ) : running ? (
                  <IconStop size={14} />
                ) : (
                  <IconBrush size={16} />
                )}
                {cancelling ? "停止中…" : running ? "停止" : "发送"}
              </button>
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}
