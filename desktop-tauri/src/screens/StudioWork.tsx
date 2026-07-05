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
} from "../components/icons";
import { describeError, invokeTool } from "../lib/core";
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
  upsertStep,
  WRITE_STAGES,
  type RunStep,
} from "../lib/agentRun";
import WorkStatus from "../components/agent/WorkStatus";
import WorkflowSteps from "../components/agent/WorkflowSteps";
import AgentFeed from "../components/agent/AgentFeed";

const DEFAULT_TITLE = "新章节";
const GOAL_EXAMPLE = "例如：写第一章，介绍主角林惊羽在北境的登场";

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

  const stepSeq = useRef(0);
  const liveTailRef = useRef<HTMLDivElement>(null);
  const pendingDeltaRef = useRef<AgentStep | null>(null);
  const rafRef = useRef<number | null>(null);

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
      if (rafRef.current != null) window.cancelAnimationFrame(rafRef.current);
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
      setSessionId(run.session.id);
      setContinuingTitle(run.session.title || t);

      // Resolve the final answer: prefer the event-streamed value (already in
      // state from the finish event), fall back to the last assistant message.
      // We compute this separately so TypeScript sees a concrete string type.
      const sessionMsgs = run.session.messages ?? [];
      let sessionFinal = "";
      for (let i = sessionMsgs.length - 1; i >= 0; i--) {
        const m = sessionMsgs[i];
        if (m.role === "assistant" && m.content.trim() && !m.tool_call) {
          sessionFinal = m.content;
          break;
        }
      }
      setFinalAnswer((prev) => {
        if (prev && prev.trim()) return prev;
        return sessionFinal || null;
      });

      // ── Frontend auto-save fallback ─────────────────────────────────────────
      // If the AI never called write_file but produced substantial text, save it
      // now. Skip if the backend already saved it (auto_saved_path present).
      const backendAlreadySaved =
        (run.outcome as { auto_saved_path?: string }).auto_saved_path != null;
      const writeFileCalled = sessionMsgs.some(
        (m) => m.tool_call?.name === "write_file",
      );
      // Use the richer of event-streamed vs session-extracted text
      // (we access state read via a ref-like trick — just use sessionFinal here
      //  since the finish event's finalAnswer state update may not have flushed)
      if (!backendAlreadySaved && !writeFileCalled && sessionFinal.length > 200) {
        const safeName = t.replace(/[/\\:*?"<>|]/g, "_");
        const filePath = `book/${safeName}.md`;
        try {
          await invokeTool("write_file", { path: filePath, content: sessionFinal });
          toast.ok(`创作完成，已保存至 ${filePath}`);
        } catch {
          toast.ok("创作完成");
        }
      } else {
        toast.ok("创作完成");
      }
    } catch (e) {
      const msg = describeError(e);
      setError(msg);
      if (isNoProviderError(msg)) setNoProvider(true);
      else if (isProviderCompatibilityError(msg)) setProviderCompat(true);
      else toast.err(`创作失败：${msg}`);
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
          <button
            className="btn btn--primary"
            onClick={() => void start()}
            disabled={running}
          >
            {running ? <Spinner size={16} /> : <IconBrush size={17} />}
            {running ? "运笔中…" : sessionId ? "继续创作" : "开始创作"}
          </button>
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
                    note={currentToolNote}
                  />
                </div>
              )}

              {showResult && finalAnswer && finalAnswer.trim() && (
                <div className="studio2__final">
                  <div className="studio2__final-head">
                    <span className="studio2__final-mark"><IconScroll size={16} /></span>
                    成稿
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

              {steps.length > 0 && (
                <div className="studio2__live">
                  <div className="studio2__section-label">
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

              {running && steps.length === 0 && (
                <div className="studio2__warming">
                  <Spinner size={32} />
                  <span>正在唤起模型，构思开篇…</span>
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
