// 创作 — Writing Studio (new desk-workflow layout).
// A focused composer up top; the live agent console + 成稿 flow below.
// Wires the real `runGoalLive` loop and reuses the agent console components.

import {
  memo,
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Spinner } from "../components/Spinner";
import {
  IconBrush,
  IconRefresh,
  IconObserve,
  IconScroll,
  IconUser,
  IconCheck,
  IconWarn,
  IconInfo,
  IconProviders,
  IconHistory,
  IconTools,
  IconStop,
} from "../components/icons";
import { cancel, describeError, newRequestId } from "../lib/core";
import { useToast } from "../components/Toast";
import {
  runGoalLive,
  type AgentStep,
  type Message,
  type Session,
} from "../lib/studio";
import {
  derivePhase,
  workflowView,
  isNoProviderError,
  isProviderCompatibilityError,
  toolGlyph,
  previewArgs,
  settlePendingTools,
  upsertStep,
  WRITE_STAGES,
  reachedWorkflowStage,
  stopReasonLabel,
  type RunStep,
} from "../lib/agentRun";
import WorkStatus from "../components/agent/WorkStatus";
import WorkflowSteps from "../components/agent/WorkflowSteps";
import AgentFeed from "../components/agent/AgentFeed";
import { getSession } from "../lib/sessions";
import { scrollLiveAnchor } from "../lib/liveScroll";
import { latestWritingAnswer, resolveWritingResult, writingAnswerText } from "../lib/studioResult";

const DEFAULT_TITLE = "新章节";
const GOAL_EXAMPLE = "例如：写第一章，介绍主角林惊羽在北境的登场";
const STEP_LIMIT_OPTIONS = [8, 16, 24, 32, 48, 64] as const;

interface StudioWorkProps {
  onOpenSettings?: () => void;
  initialSessionId?: string; // From SessionsDrawer "resume"
}

const ROLE_META: Record<
  Message["role"],
  { label: string; cls: string; Icon: (p: { size?: number }) => ReactNode }
> = {
  system: { label: "系统", cls: "msg--system", Icon: IconInfo },
  user: { label: "目标", cls: "msg--user", Icon: IconUser },
  assistant: { label: "创作", cls: "msg--assistant", Icon: IconBrush },
  tool: { label: "观察", cls: "msg--tool", Icon: IconObserve },
};

const MessageCard = memo(function MessageCard({ msg }: { msg: Message }) {
  const meta = ROLE_META[msg.role];
  const { Icon } = meta;
  const call = msg.tool_call ?? null;
  const result = msg.tool_result ?? null;
  return (
    <div className={`msg ${meta.cls}`}>
      <div className="msg__rail">
        <span className="msg__icon">
          <Icon size={15} />
        </span>
      </div>
      <div className="msg__body">
        <div className="msg__head">
          <span className="msg__role">{meta.label}</span>
          {call &&
            (() => {
              const { Icon, verb } = toolGlyph(call.name);
              return (
                <span className="msg__tag" title={`${verb} · ${call.name}`}>
                  <Icon size={11} />
                  {call.name}
                </span>
              );
            })()}
          {result && (
            <span
              className={`msg__tag ${result.ok ? "is-ok" : "is-err"}`}
              title={result.untrusted ? "外部来源，未受信任" : undefined}
            >
              {result.ok ? <IconCheck size={11} /> : <IconWarn size={11} />}
              {result.name}
              {result.untrusted ? " · 外部" : ""}
            </span>
          )}
        </div>
        {msg.content.trim() ? (
          <div className="msg__text">{msg.content}</div>
        ) : call ? (
          <div className="msg__text msg__text--muted">
            调用工具 <code>{call.name}</code>
            {(() => {
              const p = previewArgs(call.args);
              return p ? <span className="msg__args"> · {p}</span> : null;
            })()}
          </div>
        ) : (
          <div className="msg__text msg__text--muted">（无内容）</div>
        )}
      </div>
    </div>
  );
});

export default function StudioWork({ onOpenSettings, initialSessionId }: StudioWorkProps) {
  const toast = useToast();
  const [goal, setGoal] = useState("");
  const [title, setTitle] = useState(DEFAULT_TITLE);
  const [maxSteps, setMaxSteps] = useState(16);

  const [running, setRunning] = useState(false);
  const [steps, setSteps] = useState<RunStep[]>([]);
  const [finishNote, setFinishNote] = useState<string | null>(null);
  const [success, setSuccess] = useState<boolean | null>(null);
  const [session, setSession] = useState<Session | null>(null);
  const [finalAnswer, setFinalAnswer] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [noProvider, setNoProvider] = useState(false);
  const [providerCompat, setProviderCompat] = useState(false);

  const [sessionId, setSessionId] = useState<string | null>(initialSessionId ?? null);
  const [continuingTitle, setContinuingTitle] = useState<string | null>(null);
  const [loadingSession, setLoadingSession] = useState(!!initialSessionId);
  const [cancelling, setCancelling] = useState(false);
  const [cancelled, setCancelled] = useState(false);
  const [processExpanded, setProcessExpanded] = useState(true);

  const stepSeq = useRef(0);
  const liveTailRef = useRef<HTMLDivElement>(null);
  const pendingDeltaRef = useRef<AgentStep | null>(null);
  const rafRef = useRef<number | null>(null);
  const activeRequestRef = useRef<string | null>(null);
  const cancelRequestedRef = useRef(false);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      scrollLiveAnchor(liveTailRef.current, { live: running });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [steps, running]);

  const handleStepNow = useCallback((s: AgentStep) => {
    setSteps((prev) => upsertStep(prev, s, () => (stepSeq.current += 1)));
    if (s.phase === "finish") {
      setCancelled(s.reason === "cancelled");
      setSuccess(s.success);
      setFinishNote(
        s.success
          ? `创作完成（共 ${s.steps} 步）`
          : `${stopReasonLabel(s.reason)}（共 ${s.steps} 步）`,
      );
      setFinalAnswer(writingAnswerText(s.final));
    }
  }, []);

  useEffect(() => {
    if (!initialSessionId) {
      setLoadingSession(false);
      return;
    }
    let alive = true;
    setLoadingSession(true);
    void getSession(initialSessionId)
      .then((record) => {
        if (!alive) return;
        if (record.kind !== "writing") {
          throw new Error("该记录不是创作会话，无法在此续写");
        }
        setSessionId(record.session.id);
        setContinuingTitle(record.session.title || DEFAULT_TITLE);
        setTitle(record.session.title || DEFAULT_TITLE);
        setGoal(record.goal ?? "");
        setSession(record.session);
        setFinalAnswer(latestWritingAnswer(record.session.messages));
      })
      .catch((loadError) => {
        if (!alive) return;
        const message = describeError(loadError);
        setSessionId(null);
        setError(message);
        toast.err(`载入会话失败：${message}`);
      })
      .finally(() => {
        if (alive) setLoadingSession(false);
      });
    return () => {
      alive = false;
    };
  }, [initialSessionId]);

  const flushPendingDelta = useCallback(() => {
    rafRef.current = null;
    const pending = pendingDeltaRef.current;
    pendingDeltaRef.current = null;
    if (pending) handleStepNow(pending);
  }, [handleStepNow]);

  const handleStep = useCallback(
    (s: AgentStep) => {
      if (s.phase !== "delta") {
        if (pendingDeltaRef.current) flushPendingDelta();
        handleStepNow(s);
        return;
      }
      const cur = pendingDeltaRef.current;
      pendingDeltaRef.current =
        cur && cur.phase === "delta" && cur.step === s.step
          ? { ...cur, delta: cur.delta + s.delta }
          : s;
      if (rafRef.current == null) {
        rafRef.current = window.requestAnimationFrame(flushPendingDelta);
      }
    },
    [flushPendingDelta, handleStepNow],
  );

  useEffect(() => {
    return () => {
      if (rafRef.current != null) window.cancelAnimationFrame(rafRef.current);
      if (activeRequestRef.current) void cancel(activeRequestRef.current);
    };
  }, []);

  const start = useCallback(async () => {
    const g = goal.trim();
    if (!g) {
      toast.err("请先写下你的创作目标");
      return;
    }
    const t = title.trim() || DEFAULT_TITLE;
    setRunning(true);
    setCancelling(false);
    setCancelled(false);
    cancelRequestedRef.current = false;
    setError(null);
    setNoProvider(false);
    setProviderCompat(false);
    setSteps([]);
    setProcessExpanded(true);
    setFinishNote(null);
    setSuccess(null);
    setSession(null);
    setFinalAnswer(null);
    stepSeq.current = 0;
    const requestId = newRequestId("studio");
    activeRequestRef.current = requestId;
    try {
      const run = await runGoalLive(
        g,
        t,
        handleStep,
        sessionId ?? undefined,
        "writing",
        requestId,
        { maxSteps },
      );
      setSession(run.session);
      setSessionId(run.session.id);
      setContinuingTitle(run.session.title || t);

      const result = resolveWritingResult(run);
      setFinalAnswer(result.finalAnswer);
      setSuccess(result.success);
      setCancelled(result.cancelled);
      const stoppedReason = run.outcome.stopped_reason;
      setFinishNote(result.success
        ? `创作完成（共 ${run.outcome.steps} 步）`
        : `${stopReasonLabel(stoppedReason)}（共 ${run.outcome.steps} 步）`);
      if (result.success) setProcessExpanded(false);
      const terminalToolStatus = result.cancelled
        ? "cancelled"
        : result.success
          ? "success"
          : "error";
      setSteps((prev) => settlePendingTools(prev, terminalToolStatus));
      if (result.cancelled) {
        toast.info("已停止创作");
      } else if (!result.success) {
        toast.info("本次创作未完整完成，可调整目标后继续");
      } else if (run.outcome.auto_save_error) {
        toast.err(`创作完成，但成稿自动保存失败：${run.outcome.auto_save_error}`);
      } else if (run.outcome.auto_saved_path) {
        toast.ok(`创作完成，已保存至 ${run.outcome.auto_saved_path}`);
      } else {
        toast.ok("创作完成");
      }
      if (run.outcome.warning) toast.info(run.outcome.warning);
    } catch (e) {
      const stopped = cancelRequestedRef.current;
      const msg = stopped ? "已由用户停止" : describeError(e);
      if (stopped) {
        setSteps((prev) => settlePendingTools(prev, "cancelled", msg));
        setCancelled(true);
        setSuccess(false);
        setFinishNote("已停止创作");
      } else {
        setSteps((prev) => settlePendingTools(prev, "error", msg));
        setError(msg);
        if (isNoProviderError(msg)) setNoProvider(true);
        else if (isProviderCompatibilityError(msg)) setProviderCompat(true);
        else toast.err(`创作失败：${msg}`);
      }
    } finally {
      if (activeRequestRef.current === requestId) activeRequestRef.current = null;
      setRunning(false);
      setCancelling(false);
    }
  }, [goal, title, handleStep, toast, sessionId, maxSteps]);

  const reset = useCallback(() => {
    setSteps([]);
    setFinishNote(null);
    setSuccess(null);
    setSession(null);
    setFinalAnswer(null);
    setError(null);
    setCancelled(false);
    setNoProvider(false);
    setProviderCompat(false);
    setSessionId(null);
    setContinuingTitle(null);
    setGoal("");
    setProcessExpanded(true);
    stepSeq.current = 0;
  }, []);

  const stop = useCallback(async () => {
    if (!running || cancelling) return;
    setCancelling(true);
    cancelRequestedRef.current = true;
    try {
      const requestId = activeRequestRef.current;
      if (requestId) await cancel(requestId);
      toast.info("已请求停止，正在收束当前步骤…");
    } catch (cancelError) {
      cancelRequestedRef.current = false;
      toast.err(`停止失败：${describeError(cancelError)}`);
      setCancelling(false);
    }
  }, [running, cancelling, toast]);

  const hasRun = running || steps.length > 0 || session !== null || error !== null || finishNote !== null;
  const showResult = !running && (session !== null || finalAnswer !== null);
  const toolCount = steps.reduce((n, s) => n + s.toolCalls.length, 0);
  const lastStepNo = steps.length > 0 ? steps[steps.length - 1].step : 0;
  const phase = derivePhase({
    running,
    steps,
    finished: finishNote !== null,
    success,
    errored: error !== null,
    cancelling,
    cancelled,
  });
  const wf = workflowView(phase, reachedWorkflowStage(steps));
  const currentToolNote = phase === "tooling" && steps.length > 0
    ? steps[steps.length - 1].toolCalls.find(
        (call) => call.status === "queued" || call.status === "running",
      )?.name
    : undefined;

  return (
    <div className="work-content studio2">
      {/* composer panel */}
      <section className="panel studio2__composer">
        <div className="studio2__composer-head">
          <div>
            <p className="panel__kicker">第二事 · 创作</p>
            <h2 className="panel__title">观 AI 运笔成章</h2>
          </div>
          {hasRun && (
            <button className="btn btn--ghost" onClick={reset} disabled={running}>
              <IconRefresh size={16} />
              新建会话
            </button>
          )}
        </div>

        {sessionId && (
          <div className="studio2__continuing">
            <IconHistory size={14} />
            续写会话「{continuingTitle || title}」——「继续创作」将接着写
          </div>
        )}

        <textarea
          className="textarea studio2__goal"
          value={goal}
          onChange={(e) => setGoal(e.target.value)}
          placeholder={GOAL_EXAMPLE}
          disabled={running}
          spellCheck={false}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
              e.preventDefault();
              if (!running) void start();
            }
          }}
        />

        <div className="studio2__composer-row">
          <input
            className="input studio2__title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder={DEFAULT_TITLE}
            disabled={running}
            aria-label="章节标题"
          />
          <label className="studio2__steps-control">
            <span>步骤上限</span>
            <select
              className="select studio2__steps"
              value={maxSteps}
              onChange={(e) => setMaxSteps(Number(e.target.value))}
              disabled={running}
              aria-label="步骤上限"
            >
              {STEP_LIMIT_OPTIONS.map((value) => (
                <option key={value} value={value}>{value} 步</option>
              ))}
            </select>
          </label>
          <button
            className="btn btn--primary"
            onClick={() => void start()}
            disabled={loadingSession || running}
          >
            {loadingSession || running ? <Spinner size={16} /> : <IconBrush size={17} />}
            {loadingSession
              ? "载入会话…"
              : running
                ? "运笔中…"
                : sessionId
                  ? "继续创作"
                  : "开始创作"}
          </button>
          {running && (
            <button
              className="btn btn--danger"
              onClick={() => void stop()}
              disabled={cancelling}
            >
              {cancelling ? <Spinner size={15} /> : <IconStop size={15} />}
              {cancelling ? "停止中…" : "停止"}
            </button>
          )}
        </div>
        <div className="studio2__hints">
          <div className="studio2__hint">
            <IconInfo size={12} />
            将调用当前模型自主创作，可能写入 book/ 下的章节文件
          </div>
          <div className="studio2__shortcut-hint">
            <kbd>Ctrl</kbd> + <kbd>Enter</kbd> 快速开始
          </div>
        </div>
      </section>

      {/* live + result panel */}
      {hasRun && (
        <section className="panel studio2__stage">
          {noProvider ? (
            <div className="studio2__notice">
              <span className="studio2__notice-seal"><IconProviders size={26} /></span>
              <h3>尚未选用模型</h3>
              <p>创作需要一个已启用的模型供应商。请先添加并启用一个模型。</p>
              {error && <div className="studio2__notice-err">{error}</div>}
              <button className="btn btn--primary" onClick={onOpenSettings}>
                <IconProviders size={16} />
                前往供应商配置
              </button>
            </div>
          ) : providerCompat ? (
            <div className="studio2__notice">
              <span className="studio2__notice-seal"><IconTools size={26} /></span>
              <h3>模型工具模式不兼容</h3>
              <p>当前模型可能拒绝原生工具调用。请把工具调用模式改为「自动兼容」或「文本工具」。</p>
              {error && <div className="studio2__notice-err">{error}</div>}
              <button className="btn btn--primary" onClick={onOpenSettings}>
                <IconTools size={16} />
                调整供应商设置
              </button>
            </div>
          ) : (
            <>
              {(running || steps.length > 0) && (
                <div className="agent-console">
                  <WorkflowSteps stages={WRITE_STAGES} current={wf.current} state={wf.state} />
                  <WorkStatus
                    phase={phase}
                    step={lastStepNo}
                    toolCount={toolCount}
                    note={running ? currentToolNote : success === false ? finishNote ?? undefined : undefined}
                  />
                </div>
              )}

              {showResult && finalAnswer && finalAnswer.trim() && (
                <div className={`studio2__final${success ? "" : " is-partial"}`}>
                  <div className="studio2__final-head">
                    <span className="studio2__final-mark"><IconScroll size={16} /></span>
                    {success ? "成稿" : cancelled ? "已停止的草稿" : "未完成草稿"}
                    {finishNote && <span className="studio2__final-note">{finishNote}</span>}
                  </div>
                  <div className="studio2__final-body">{finalAnswer}</div>
                </div>
              )}

              {!running && error && !finalAnswer && (
                <div className="studio2__notice-err" style={{ margin: "12px 0" }}>
                  <IconWarn size={16} /> {error}
                </div>
              )}

              {(running || steps.length > 0) && (
                <div className="studio2__live">
                  <div className="studio2__section-label">
                    {running ? (
                      <>
                        创作过程 · 第 {Math.max(lastStepNo, 1)} 步
                      </>
                    ) : (
                      <>
                        <IconScroll size={13} />
                        创作历程 · {steps.length} 步
                      </>
                    )}
                    {steps.length > 0 && (
                      <button
                        type="button"
                        className="studio2__process-toggle"
                        onClick={() => setProcessExpanded((expanded) => !expanded)}
                        aria-expanded={processExpanded}
                        aria-controls="studio2-process-feed"
                      >
                        {processExpanded ? "收起步骤" : "展开步骤"}
                      </button>
                    )}
                  </div>
                  <div id="studio2-process-feed" hidden={!processExpanded}>
                    <AgentFeed
                      steps={steps}
                      running={running}
                      phase={phase}
                      pendingText="AI 正在思索下一笔…"
                      tailRef={liveTailRef}
                    />
                  </div>
                </div>
              )}

              {showResult && session && session.messages.length > 0 && (
                <div className="studio2__transcript">
                  <div className="studio2__section-label">
                    <IconScroll size={13} />
                    全程记录 · {session.messages.length} 条
                  </div>
                  <div className="studio2__messages">
                    {session.messages.map((m, i) => (
                      <MessageCard key={i} msg={m} />
                    ))}
                  </div>
                </div>
              )}

            </>
          )}
        </section>
      )}

      {!hasRun && (
        <div className="empty studio2__empty">
          <p className="empty__title">研墨以待</p>
          <p className="empty__text">
            写下创作目标，点击「开始创作」。AI 会一步步构思、查阅、落笔，你将实时看到它的每一念与每一笔。
          </p>
        </div>
      )}
    </div>
  );
}
