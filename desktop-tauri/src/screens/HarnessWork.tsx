// Harness workbench: durable plan -> execute -> verify -> deliver runs.

import { useCallback, useEffect, useRef, useState } from "react";
import { Spinner } from "../components/Spinner";
import {
  IconCheck,
  IconHistory,
  IconProviders,
  IconRefresh,
  IconStop,
  IconTools,
  IconWarn,
} from "../components/icons";
import { useToast } from "../components/Toast";
import {
  cancel,
  describeError,
  isDesktop,
  newRequestId,
} from "../lib/core";
import {
  runGoalLive,
  type AgentStep,
  type Session,
} from "../lib/studio";
import { getProviders, PROVIDERS_CHANGED_EVENT } from "../lib/providers";
import {
  derivePhase,
  isNoProviderError,
  isProviderCompatibilityError,
  reachedWorkflowStage,
  settlePendingTools,
  stopReasonLabel,
  upsertStep,
  workflowView,
  type RunStep,
} from "../lib/agentRun";
import WorkStatus from "../components/agent/WorkStatus";
import WorkflowSteps from "../components/agent/WorkflowSteps";
import AgentFeed from "../components/agent/AgentFeed";
import { getSession } from "../lib/sessions";
import {
  buildHarnessGoal,
  HARNESS_STAGES,
  harnessContinuationGoal,
  restoreHarnessSession,
} from "../lib/harness";

interface HarnessWorkProps {
  onOpenSettings?: () => void;
  initialSessionId?: string;
}

export default function HarnessWork({ onOpenSettings, initialSessionId }: HarnessWorkProps) {
  const toast = useToast();
  const [task, setTask] = useState("");
  const [acceptance, setAcceptance] = useState("");
  const [constraints, setConstraints] = useState("");
  const [followUp, setFollowUp] = useState("");
  const [session, setSession] = useState<Session | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(initialSessionId ?? null);
  const [running, setRunning] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [finished, setFinished] = useState(false);
  const [success, setSuccess] = useState<boolean | null>(null);
  const [cancelled, setCancelled] = useState(false);
  const [steps, setSteps] = useState<RunStep[]>([]);
  const [finalAnswer, setFinalAnswer] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loadingSession, setLoadingSession] = useState(!!initialSessionId);
  const [noProvider, setNoProvider] = useState(false);
  const [providerCompat, setProviderCompat] = useState(false);
  const [activeModel, setActiveModel] = useState<string | null>(null);
  const activeRequestRef = useRef<string | null>(null);
  const requestCancelledRef = useRef(false);
  const keyRef = useRef(0);
  const tailRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!initialSessionId) return;
    let alive = true;
    setLoadingSession(true);
    void getSession(initialSessionId).then((record) => {
      if (!alive) return;
      const restored = restoreHarnessSession(record);
      setTask(restored.task);
      setAcceptance(restored.acceptance);
      setConstraints(restored.constraints);
      setSession(record.session);
      setSessionId(record.session.id);
      setFinalAnswer(restored.finalAnswer);
      setFinished(true);
      setSuccess(null);
    }).catch((reason) => {
      if (alive) setError(`载入 Harness 会话失败：${describeError(reason)}`);
    }).finally(() => {
      if (alive) setLoadingSession(false);
    });
    return () => { alive = false; };
  }, [initialSessionId]);

  useEffect(() => {
    if (!isDesktop()) return;
    let alive = true;
    const load = async () => {
      try {
        const settings = await getProviders();
        const provider = settings.providers.find((item) => item.id === settings.active_provider);
        if (alive) setActiveModel(provider && settings.active_model
          ? `${provider.name || "未命名供应商"} · ${settings.active_model}`
          : null);
      } catch {
        if (alive) setActiveModel(null);
      }
    };
    void load();
    const handler = () => { void load(); };
    window.addEventListener(PROVIDERS_CHANGED_EVENT, handler);
    return () => {
      alive = false;
      window.removeEventListener(PROVIDERS_CHANGED_EVENT, handler);
    };
  }, []);

  useEffect(() => () => {
    if (activeRequestRef.current) void cancel(activeRequestRef.current);
  }, []);

  const reset = useCallback(() => {
    if (running) return;
    setTask("");
    setAcceptance("");
    setConstraints("");
    setFollowUp("");
    setSession(null);
    setSessionId(null);
    setFinished(false);
    setSuccess(null);
    setCancelled(false);
    setSteps([]);
    setFinalAnswer(null);
    setError(null);
    setNoProvider(false);
    setProviderCompat(false);
  }, [running]);

  const start = useCallback(async () => {
    if (running || loadingSession) return;
    if (!task.trim()) {
      toast.err("请先写下 Harness 任务目标");
      return;
    }
    setRunning(true);
    setCancelling(false);
    setFinished(false);
    setSuccess(null);
    setCancelled(false);
    setError(null);
    setNoProvider(false);
    setProviderCompat(false);
    setSteps([]);
    setFinalAnswer(null);
    keyRef.current = 0;
    const requestId = newRequestId("harness");
    activeRequestRef.current = requestId;
    requestCancelledRef.current = false;
    const applyStep = (step: AgentStep) => {
      setSteps((current) => upsertStep(current, step, () => (keyRef.current += 1)));
      if (step.phase === "finish") {
        setSteps((current) => settlePendingTools(
          current,
          step.reason === "cancelled" ? "cancelled" : step.success ? "success" : "error",
        ));
        setRunning(false);
        setFinished(true);
        setSuccess(step.success);
        setCancelled(step.reason === "cancelled");
      }
    };
    try {
      const goal = session
        ? harnessContinuationGoal(task, acceptance, constraints, followUp)
        : buildHarnessGoal(task, acceptance, constraints);
      const result = await runGoalLive(
        goal,
        session?.title || `Harness · ${task.trim().slice(0, 28)}`,
        applyStep,
        sessionId ?? undefined,
        "harness",
        requestId,
      );
      setSession(result.session);
      setSessionId(result.session.id);
      setFinalAnswer(result.outcome.final_answer);
      setFollowUp("");
      if (requestCancelledRef.current) {
        setCancelled(true);
        setSuccess(false);
        setFinished(true);
      } else if (!result.outcome.final_answer && result.outcome.stopped_reason !== "goal_reached") {
        setError(stopReasonLabel(result.outcome.stopped_reason));
      }
    } catch (reason) {
      const message = describeError(reason);
      setError(message);
      setNoProvider(isNoProviderError(message));
      setProviderCompat(isProviderCompatibilityError(message));
      setSteps((current) => settlePendingTools(current, "error", message));
    } finally {
      setRunning(false);
      setCancelling(false);
      setFinished(true);
      activeRequestRef.current = null;
    }
  }, [acceptance, constraints, followUp, loadingSession, running, session, sessionId, task, toast]);

  const stop = useCallback(async () => {
    const requestId = activeRequestRef.current;
    if (!requestId || cancelling) return;
    setCancelling(true);
    requestCancelledRef.current = true;
    try {
      await cancel(requestId);
      toast.info("已请求停止，Harness 将保留当前进度");
    } catch (reason) {
      setCancelling(false);
      requestCancelledRef.current = false;
      toast.err(`停止失败：${describeError(reason)}`);
    }
  }, [cancelling, toast]);

  const phase = derivePhase({
    running,
    steps,
    finished,
    success,
    errored: error !== null,
    cancelling,
    cancelled,
  });
  const workflow = workflowView(phase, reachedWorkflowStage(steps));
  const toolCount = steps.reduce((count, step) => count + step.toolCalls.length, 0);
  const lastStep = steps.length > 0 ? steps[steps.length - 1].step : 0;
  const hasRun = running || finished || steps.length > 0 || session !== null || error !== null;
  const showResult = !running && (finalAnswer !== null || session !== null);

  return (
    <div className="work-content harness-work">
      <section className="panel harness-work__hero">
        <div className="harness-work__heading">
          <div>
            <p className="panel__kicker">独立工作区 · Harness</p>
            <h2 className="panel__title">把长任务变成可验证的执行链</h2>
            <p className="harness-work__lede">Harness 会记录计划、工具行动、验证证据和交付结果；中断后可从会话历史继续。</p>
          </div>
          <div className="harness-work__model" data-ready={!!activeModel}>
            <span className="harness-work__model-dot" />
            {activeModel || "尚未选择模型"}
          </div>
        </div>
        {sessionId && (
          <div className="harness-work__continuing">
            <IconHistory size={14} />
            正在继续历史 Harness 会话「{session?.title || "未命名任务"}」
          </div>
        )}
        <label className="harness-field">
          <span>任务目标</span>
          <textarea value={task} onChange={(event) => setTask(event.target.value)} disabled={running} placeholder="例如：检查当前章节生成链路，修复一个问题并用回归测试证明它没有影响创作流程" />
        </label>
        <div className="harness-work__fields">
          <label className="harness-field">
            <span>验收标准</span>
            <textarea value={acceptance} onChange={(event) => setAcceptance(event.target.value)} disabled={running} placeholder="每条标准一行，例如：测试通过、历史会话可继续、没有改动无关文件" />
          </label>
          <label className="harness-field">
            <span>约束与边界</span>
            <textarea value={constraints} onChange={(event) => setConstraints(event.target.value)} disabled={running} placeholder="例如：保留旧版本；不得泄露密钥；沿用现有架构" />
          </label>
        </div>
        {sessionId && (
          <label className="harness-field harness-field--followup">
            <span>本轮继续要求</span>
            <input value={followUp} onChange={(event) => setFollowUp(event.target.value)} disabled={running} placeholder="留空则自动检查上次未完成的阶段" />
          </label>
        )}
        <div className="harness-work__actions">
          <button className="btn btn--primary" onClick={() => void start()} disabled={running || loadingSession}>
            {running || loadingSession ? <Spinner size={16} /> : <IconTools size={16} />}
            {loadingSession ? "载入会话…" : running ? "Harness 执行中…" : sessionId ? "继续执行" : "开始 Harness"}
          </button>
          {running && (
            <button className="btn btn--danger" onClick={() => void stop()} disabled={cancelling}>
              {cancelling ? <Spinner size={15} /> : <IconStop size={15} />}
              {cancelling ? "停止中…" : "停止并保留进度"}
            </button>
          )}
          {hasRun && !running && (
            <button className="btn btn--ghost" onClick={reset}><IconRefresh size={15} />新建任务</button>
          )}
        </div>
      </section>

      {hasRun && (
        <section className="panel harness-work__stage">
          {(noProvider || providerCompat) ? (
            <div className="harness-work__notice">
              <span className="harness-work__notice-icon"><IconProviders size={25} /></span>
              <h3>{noProvider ? "尚未选用模型" : "模型工具模式不兼容"}</h3>
              <p>{noProvider ? "Harness 需要一个已启用的模型供应商。" : "当前模型拒绝工具调用，请调整供应商的工具模式。"}</p>
              {error && <div className="harness-work__error">{error}</div>}
              <button className="btn btn--primary" onClick={onOpenSettings}><IconProviders size={15} />打开供应商设置</button>
            </div>
          ) : (
            <>
              <div className="agent-console">
                <WorkflowSteps stages={[...HARNESS_STAGES]} current={workflow.current} state={workflow.state} />
                <WorkStatus phase={phase} step={lastStep} toolCount={toolCount} />
              </div>
              {error && <div className="harness-work__error"><IconWarn size={15} />{error}</div>}
              {showResult && finalAnswer && (
                <div className={`harness-work__result${success === false ? " is-partial" : ""}`}>
                  <div className="harness-work__result-head"><IconCheck size={15} />交付结果</div>
                  <div className="harness-work__result-body">{finalAnswer}</div>
                </div>
              )}
              {(running || steps.length > 0) && (
                <div className="harness-work__feed">
                  <div className="harness-work__section-label">执行记录 · {steps.length} 个模型步骤</div>
                  <AgentFeed steps={steps} running={running} phase={phase} pendingText="Harness 正在推进下一阶段…" tailRef={tailRef} />
                </div>
              )}
            </>
          )}
        </section>
      )}

      {!hasRun && (
        <div className="empty harness-work__empty">
          <p className="empty__title">先定义验收，再开始执行</p>
          <p className="empty__text">Harness 适合跨文件、需要工具和验证的长任务。它与创作、策划页面隔离，自己的会话也会出现在历史记录中。</p>
        </div>
      )}
    </div>
  );
}
