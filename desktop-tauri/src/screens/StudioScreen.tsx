// 创作 — the Writing Studio. State a 创作目标 + a 章节标题/书名, hit 开始创作,
// then watch the real agent loop work live through the Agent Console: a 工作流程
// stage tracker + a live 工作状态 strip up top, then each model turn streaming in
// as 模型推理 + 工具调用 cards. On finish the 成稿 is shown prominently above the
// full session transcript (system / user / assistant / tool).
//
// If no model provider is configured the core throws (mentioning 供应商/未选择);
// we catch that and show a friendly state with a button that jumps to 供应商.

import {
  memo,
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import Panel from "../components/Panel";
import EmptyState from "../components/EmptyState";
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
  BrushStroke,
} from "../components/icons";
import { describeError } from "../lib/core";
import { useToast } from "../components/Toast";
import {
  runGoalLive,
  type AgentStep,
  type Message,
  type Session,
} from "../lib/studio";
import { getSession } from "../lib/sessions";
import {
  derivePhase,
  workflowView,
  isNoProviderError,
  isProviderCompatibilityError,
  toolGlyph,
  previewArgs,
  upsertStep,
  WRITE_STAGES,
  type RunStep,
} from "../lib/agentRun";
import WorkStatus from "../components/agent/WorkStatus";
import WorkflowSteps from "../components/agent/WorkflowSteps";
import AgentFeed from "../components/agent/AgentFeed";
import type { ScreenId } from "../lib/screens";

const DEFAULT_TITLE = "新章节";
const GOAL_EXAMPLE = "例如：写第一章，介绍主角林惊羽在北境的登场";

interface StudioScreenProps {
  onNavigate?: (id: ScreenId) => void;
  /** When set, load this saved session and continue writing from it. */
  resumeId?: string;
  /** Called once a resume request has been consumed. */
  onResumed?: () => void;
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

export default function StudioScreen({
  onNavigate,
  resumeId,
  onResumed,
}: StudioScreenProps) {
  const toast = useToast();
  const [goal, setGoal] = useState("");
  const [title, setTitle] = useState(DEFAULT_TITLE);

  const [running, setRunning] = useState(false);
  const [steps, setSteps] = useState<RunStep[]>([]);
  const [finishNote, setFinishNote] = useState<string | null>(null);
  const [success, setSuccess] = useState<boolean | null>(null);
  const [session, setSession] = useState<Session | null>(null);
  const [finalAnswer, setFinalAnswer] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [noProvider, setNoProvider] = useState(false);
  const [providerCompat, setProviderCompat] = useState(false);

  // session continuity: when set, the next run CONTINUES this session.
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [continuingTitle, setContinuingTitle] = useState<string | null>(null);

  const stepSeq = useRef(0);
  const liveTailRef = useRef<HTMLDivElement>(null);
  const pendingDeltaRef = useRef<AgentStep | null>(null);
  const rafRef = useRef<number | null>(null);

  // Resume a saved session: load it, show its transcript + last 成稿, and arm
  // the next run to continue it. (deps: resumeId only — onResumed/toast are
  // used once and stable enough; re-running on a cleared id is a no-op.)
  useEffect(() => {
    if (!resumeId) return;
    let alive = true;
    void (async () => {
      try {
        const rec = await getSession(resumeId);
        if (!alive) return;
        setSessionId(rec.session.id);
        setContinuingTitle(rec.session.title);
        setTitle(rec.session.title || DEFAULT_TITLE);
        setSession(rec.session);
        setSteps([]);
        setFinishNote(null);
        setSuccess(null);
        setError(null);
        setNoProvider(false);
        setProviderCompat(false);
        setGoal("");
        const msgs = rec.session.messages ?? [];
        let fin: string | null = null;
        for (let i = msgs.length - 1; i >= 0; i--) {
          const m = msgs[i];
          if (m.role === "assistant" && m.content.trim() && !m.tool_call) {
            fin = m.content;
            break;
          }
        }
        setFinalAnswer(fin);
        toast.ok(`已载入会话「${rec.session.title || "未命名"}」，可接着写`);
      } catch (e) {
        if (alive) toast.err(`载入会话失败：${describeError(e)}`);
      } finally {
        onResumed?.();
      }
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resumeId]);

  // auto-scroll the live feed to the newest step
  useEffect(() => {
    if (running) {
      liveTailRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
    }
  }, [steps, running]);

  const handleStepNow = useCallback((s: AgentStep) => {
    if (s.phase === "delta" || s.phase === "model") {
      setSteps((prev) => upsertStep(prev, s, () => (stepSeq.current += 1)));
    } else if (s.phase === "finish") {
      setSuccess(s.success);
      setFinishNote(
        s.success
          ? `创作完成（共 ${s.steps} 步）`
          : `已停止：${s.reason || "未完成"}（共 ${s.steps} 步）`,
      );
      if (s.final != null) setFinalAnswer(s.final);
    }
    // phase:"step" is a heartbeat — the model cards already convey progress.
  }, []);

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
      if (rafRef.current != null) {
        window.cancelAnimationFrame(rafRef.current);
      }
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
    setError(null);
    setNoProvider(false);
    setProviderCompat(false);
    setSteps([]);
    setFinishNote(null);
    setSuccess(null);
    setSession(null);
    setFinalAnswer(null);
    stepSeq.current = 0;
    try {
      const run = await runGoalLive(g, t, handleStep, sessionId ?? undefined);
      setSession(run.session);
      // remember the (possibly new) session id so further runs continue it
      setSessionId(run.session.id);
      setContinuingTitle(run.session.title || t);
      setFinalAnswer((prev) => {
        if (prev && prev.trim()) return prev;
        const msgs = run.session.messages ?? [];
        for (let i = msgs.length - 1; i >= 0; i--) {
          const m = msgs[i];
          if (m.role === "assistant" && m.content.trim() && !m.tool_call) {
            return m.content;
          }
        }
        return prev;
      });
      toast.ok("创作完成");
    } catch (e) {
      const msg = describeError(e);
      setError(msg);
      if (isNoProviderError(msg)) {
        setNoProvider(true);
      } else if (isProviderCompatibilityError(msg)) {
        setProviderCompat(true);
      } else {
        toast.err(`创作失败：${msg}`);
      }
    } finally {
      setRunning(false);
    }
  }, [goal, title, handleStep, toast, sessionId]);

  const reset = useCallback(() => {
    setSteps([]);
    setFinishNote(null);
    setSuccess(null);
    setSession(null);
    setFinalAnswer(null);
    setError(null);
    setNoProvider(false);
    setProviderCompat(false);
    setSessionId(null);
    setContinuingTitle(null);
    setGoal("");
    stepSeq.current = 0;
  }, []);

  const hasRun = running || steps.length > 0 || session !== null || error !== null;
  const showResult = !running && (session !== null || finalAnswer !== null);

  const toolCount = steps.reduce((n, s) => n + s.toolCalls.length, 0);
  const lastStepNo = steps.length > 0 ? steps[steps.length - 1].step : 0;
  const phase = derivePhase({
    running,
    steps,
    finished: finishNote !== null,
    success,
    errored: error !== null,
  });
  const wf = workflowView(phase);
  const currentToolNote =
    phase === "tooling" && steps.length > 0 && steps[steps.length - 1].toolCalls[0]
      ? steps[steps.length - 1].toolCalls[0].name
      : undefined;

  const headerActions = hasRun ? (
    <button
      className="btn btn--ghost"
      onClick={reset}
      disabled={running}
      title="清空，另开一个新会话"
    >
      <IconRefresh size={16} />
      新建会话
    </button>
  ) : undefined;

  return (
    <Panel
      title="创作"
      en="Writing Studio"
      subtitle="立意 · 观 AI 运笔成章，实时见其思与所用之器"
      actions={headerActions}
    >
      <div className="studio">
        {/* ---- prompt composer ---- */}
        <div className="studio__composer">
          {sessionId && (
            <div className="studio__continuing">
              <IconHistory size={14} />
              <span className="studio__continuing-text">
                续写会话「{continuingTitle || title}」——「继续创作」将在已有内容基础上接着写
              </span>
              <button
                className="btn btn--ghost btn--sm"
                onClick={reset}
                disabled={running}
                title="清空，另开一个新会话"
              >
                新建会话
              </button>
            </div>
          )}
          <div className="field">
            <label className="field__label">创作目标</label>
            <textarea
              className="textarea studio__goal"
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
          </div>
          <div className="studio__composer-row">
            <div className="field studio__title-field">
              <label className="field__label">章节标题 / 书名</label>
              <input
                className="input"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder={DEFAULT_TITLE}
                disabled={running}
              />
            </div>
            <button
              className="btn btn--primary studio__go"
              onClick={() => void start()}
              disabled={running}
            >
              {running ? <Spinner size={16} /> : <IconBrush size={17} />}
              {running ? "运笔中…" : sessionId ? "继续创作" : "开始创作"}
            </button>
          </div>
          <p className="field__hint">
            <IconInfo size={12} className="field__hint-icon" />
            将调用当前启用的模型，由 AI 自主创作并可能写入 book/ 下的章节文件。可按
            <span className="kbd">Ctrl/⌘ + Enter</span> 快速开始。
          </p>
        </div>

        {/* ---- live / results area ---- */}
        <div className="studio__feed">
          {!hasRun ? (
            <EmptyState
              title="研墨以待"
              text="写下你的创作目标，点击「开始创作」。AI 会一步步构思、查阅与落笔，你将实时看到它的每一念与每一笔。"
            />
          ) : noProvider ? (
            <div className="studio__notice">
              <BrushStroke className="studio__notice-flourish" aria-hidden="true" />
              <span className="studio__notice-seal">
                <IconProviders size={26} />
              </span>
              <h3 className="studio__notice-title">尚未选用模型</h3>
              <p className="studio__notice-text">
                创作需要一个已启用的模型供应商。请先到「供应商」添加并启用一个模型，
                再回到这里开始创作。
              </p>
              {error && <div className="studio__notice-err">{error}</div>}
              <button
                className="btn btn--primary"
                onClick={() => onNavigate?.("providers")}
              >
                <IconProviders size={16} />
                前往供应商配置
              </button>
            </div>
          ) : providerCompat ? (
            <div className="studio__notice">
              <BrushStroke className="studio__notice-flourish" aria-hidden="true" />
              <span className="studio__notice-seal">
                <IconTools size={26} />
              </span>
              <h3 className="studio__notice-title">模型工具模式不兼容</h3>
              <p className="studio__notice-text">
                当前模型可能拒绝原生工具调用。请到「供应商」把工具调用模式改为「自动兼容」
                或「文本工具」，再重新开始创作。
              </p>
              {error && <div className="studio__notice-err">{error}</div>}
              <button
                className="btn btn--primary"
                onClick={() => onNavigate?.("providers")}
              >
                <IconProviders size={16} />
                调整供应商设置
              </button>
            </div>
          ) : (
            <>
              {/* ---- the agent console: 工作流程 + 工作状态 (during/after a run) ---- */}
              {(running || steps.length > 0) && (
                <div className="agent-console">
                  <WorkflowSteps
                    stages={WRITE_STAGES}
                    current={wf.current}
                    state={wf.state}
                  />
                  <WorkStatus
                    phase={phase}
                    step={lastStepNo}
                    toolCount={toolCount}
                    note={currentToolNote}
                  />
                </div>
              )}

              {/* the final answer, shown prominently once finished */}
              {showResult && finalAnswer && finalAnswer.trim() && (
                <div className="studio__final">
                  <div className="studio__final-head">
                    <span className="studio__final-mark">
                      <IconScroll size={16} />
                    </span>
                    <span className="studio__final-label">成稿</span>
                    {finishNote && (
                      <span className="studio__final-note">{finishNote}</span>
                    )}
                  </div>
                  <div className="studio__final-body">{finalAnswer}</div>
                  <div className="studio__final-hint">
                    <IconInfo size={12} className="field__hint-icon" />
                    AI 通常会用 <code>write_file</code> 把成稿写入 <code>book/</code>{" "}
                    目录，可到「章节」中查看与续编。
                  </div>
                </div>
              )}

              {!running && error && !finalAnswer && (
                <div className="banner banner--warn" style={{ margin: "0 0 16px" }}>
                  <IconWarn size={16} />
                  {error}
                </div>
              )}

              {/* live step feed */}
              {steps.length > 0 && (
                <div className="studio__live">
                  <div className="studio__section-label">
                    {running ? (
                      <>
                        <span className="ink-pulse" aria-hidden="true" />
                        运笔中… 第 {lastStepNo} 步
                      </>
                    ) : (
                      <>
                        <IconScroll size={13} />
                        创作历程 · {steps.length} 步
                      </>
                    )}
                  </div>
                  <AgentFeed
                    steps={steps}
                    running={running}
                    pendingText="AI 正在思索下一笔…"
                    tailRef={liveTailRef}
                  />
                </div>
              )}

              {/* the full transcript, once we have a session */}
              {showResult && session && session.messages.length > 0 && (
                <div className="studio__transcript">
                  <div className="studio__section-label">
                    <IconScroll size={13} />
                    全程记录 · {session.messages.length} 条
                  </div>
                  <div className="studio__messages">
                    {session.messages.map((m, i) => (
                      <MessageCard key={i} msg={m} />
                    ))}
                  </div>
                </div>
              )}

              {/* a calm working state before the first model step arrives */}
              {running && steps.length === 0 && (
                <div className="studio__warming">
                  <span className="studio__warming-orb">
                    <BrushStroke
                      className="studio__warming-brush"
                      aria-hidden="true"
                    />
                    <Spinner size={36} />
                  </span>
                  <span className="studio__warming-text">运笔中…</span>
                  <span className="studio__warming-sub">正在唤起模型，构思开篇</span>
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </Panel>
  );
}
