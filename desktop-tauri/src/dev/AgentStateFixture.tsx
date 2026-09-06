import { useCallback, useEffect, useMemo, useState } from "react";
import AgentFeed from "../components/agent/AgentFeed";
import WorkflowSteps from "../components/agent/WorkflowSteps";
import WorkStatus from "../components/agent/WorkStatus";
import {
  IconArrowRight,
  IconBrush,
  IconStop,
} from "../components/icons";
import {
  WRITE_STAGES,
  reachedWorkflowStage,
  workflowView,
  type AgentPhase,
  type RunStep,
  type RunToolCall,
} from "../lib/agentRun";
import "./AgentStateFixture.css";

const FIXTURE_STATES = [
  "idle",
  "warming",
  "reasoning",
  "streaming",
  "tool-queued",
  "tool-running",
  "tool-success",
  "tool-error",
  "tool-cancelled",
  "cancelling",
  "done",
  "cancelled",
  "stopped",
  "error",
] as const;

type FixtureState = (typeof FIXTURE_STATES)[number];

interface FixtureMeta {
  label: string;
  detail: string;
}

interface FixtureView {
  phase: AgentPhase;
  running: boolean;
  steps: RunStep[];
  pendingText?: string;
  terminal?: string;
}

const STATE_META: Record<FixtureState, FixtureMeta> = {
  idle: { label: "待命", detail: "尚未开始任务，工作流停在立意。" },
  warming: { label: "唤起模型", detail: "正在读取作品设定与本次创作目标。" },
  reasoning: { label: "运思推理", detail: "模型正在梳理人物关系与叙事落点。" },
  streaming: { label: "流式落笔", detail: "推理文字正在逐段抵达并持续生长。" },
  "tool-queued": { label: "工具候笔", detail: "工具调用已生成，正在等待调度。" },
  "tool-running": { label: "工具执行", detail: "正在查阅记忆库，耗时会实时更新。" },
  "tool-success": { label: "工具完成", detail: "工具已返回结果，模型将继续运思。" },
  "tool-error": { label: "工具失败", detail: "工具返回错误，AI 正在调整方案并继续运思。" },
  "tool-cancelled": { label: "工具取消", detail: "工具调用已取消，不再继续等待结果。" },
  cancelling: { label: "正在停笔", detail: "等待当前工具安全退出并收束现场。" },
  done: { label: "创作完成", detail: "全部阶段已完成，结果已经成章。" },
  cancelled: { label: "已由用户停止", detail: "任务已安全取消，保留此前产生的过程。" },
  stopped: { label: "未完整完成", detail: "达到步骤上限后停止，可调整目标再继续。" },
  error: { label: "运行出错", detail: "供应商连接中断，保留已生成的推理内容。" },
};

const TOOL_BASE = {
  id: "fixture-memory-recall",
  name: "memory_recall",
  args: { query: "雨夜重逢、旧伞与未寄出的书信", limit: 5 },
} as const;

function isFixtureState(value: string | null): value is FixtureState {
  return FIXTURE_STATES.some((state) => state === value);
}

function stateFromLocation(): FixtureState {
  const requested = new URLSearchParams(window.location.search).get("state");
  return isFixtureState(requested) ? requested : "idle";
}

function toolCall(status: RunToolCall["status"]): RunToolCall {
  const terminal = status !== "queued" && status !== "running";
  return {
    ...TOOL_BASE,
    status,
    ...(terminal ? { durationMs: 1_460 } : {}),
    ...(status === "success" ? { summary: "检得三条相关人物记忆" } : {}),
    ...(status === "error" ? { error: "记忆索引暂时不可用" } : {}),
    ...(status === "cancelled" ? { summary: "已停止本次查阅" } : {}),
  };
}

function reasoningStep(text: string, streaming = false): RunStep {
  return { key: 1, step: 1, text, toolCalls: [], streaming };
}

function stepWithTool(status: RunToolCall["status"]): RunStep {
  return {
    key: 1,
    step: 1,
    text: "先从旧物中找回两人的共同记忆，再决定重逢时由谁先开口。",
    toolCalls: [toolCall(status)],
    streaming: false,
  };
}

function viewForState(state: FixtureState): FixtureView {
  switch (state) {
    case "idle":
      return { phase: "idle", running: false, steps: [] };
    case "warming":
      return {
        phase: "warming",
        running: true,
        steps: [],
        pendingText: "AI 正在研读作品设定…",
      };
    case "reasoning":
      return {
        phase: "reasoning",
        running: true,
        steps: [reasoningStep("雨势不宜写得喧闹，应让檐下的停顿先于对白抵达。")],
      };
    case "streaming":
      return {
        phase: "streaming",
        running: true,
        steps: [reasoningStep("她认出那把旧伞，却没有立刻抬头。雨水沿着伞骨，一滴一滴落在", true)],
      };
    case "tool-queued":
      return { phase: "tooling", running: true, steps: [stepWithTool("queued")] };
    case "tool-running":
      return { phase: "tooling", running: true, steps: [stepWithTool("running")] };
    case "tool-success":
      return {
        phase: "reasoning",
        running: true,
        steps: [stepWithTool("success")],
        pendingText: "工具已返回，AI 正在续接下一笔…",
      };
    case "tool-error":
      return {
        phase: "reasoning",
        running: true,
        steps: [stepWithTool("error")],
        pendingText: "工具未完成，AI 正在调整下一步…",
      };
    case "tool-cancelled":
      return {
        phase: "cancelled",
        running: false,
        steps: [stepWithTool("cancelled")],
        terminal: "工具调用已取消。",
      };
    case "cancelling":
      return {
        phase: "cancelling",
        running: true,
        steps: [stepWithTool("running")],
      };
    case "done":
      return {
        phase: "done",
        running: false,
        steps: [
          stepWithTool("success"),
          reasoningStep("旧伞微微偏向她那一侧。谁也没有提起离别，雨声却替他们说完了余下的话。"),
        ].map((step, index) => ({ ...step, key: index + 1, step: index + 1 })),
        terminal: "创作完成（共 2 步）",
      };
    case "cancelled":
      return {
        phase: "cancelled",
        running: false,
        steps: [stepWithTool("cancelled")],
        terminal: "已由用户停止，以上过程已保留。",
      };
    case "stopped":
      return {
        phase: "stopped",
        running: false,
        steps: [stepWithTool("success")],
        terminal: "已到本次步骤上限，任务未完整完成。",
      };
    case "error":
      return {
        phase: "error",
        running: false,
        steps: [reasoningStep("人物关系已经梳理完成，准备进入正式落笔。")],
        terminal: "连接模型供应商时发生错误。",
      };
  }
}

function nextState(state: FixtureState, offset: -1 | 1): FixtureState {
  const current = FIXTURE_STATES.indexOf(state);
  const next = (current + offset + FIXTURE_STATES.length) % FIXTURE_STATES.length;
  return FIXTURE_STATES[next];
}

export default function AgentStateFixture() {
  const [state, setState] = useState<FixtureState>(stateFromLocation);
  const [playing, setPlaying] = useState(false);
  const view = useMemo(() => viewForState(state), [state]);
  const meta = STATE_META[state];

  const updateState = useCallback((next: FixtureState) => {
    const url = new URL(window.location.href);
    url.searchParams.set("ui-fixture", "agent");
    url.searchParams.set("state", next);
    window.history.replaceState(null, "", url);
    setState(next);
  }, []);

  const chooseState = useCallback((next: FixtureState) => {
    setPlaying(false);
    updateState(next);
  }, [updateState]);

  const move = useCallback((offset: -1 | 1) => {
    chooseState(nextState(state, offset));
  }, [chooseState, state]);

  useEffect(() => {
    const onPopState = () => {
      setPlaying(false);
      setState(stateFromLocation());
    };
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  useEffect(() => {
    if (!playing) return;
    const timer = window.setTimeout(() => updateState(nextState(state, 1)), 2_200);
    return () => window.clearTimeout(timer);
  }, [playing, state, updateState]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.matches("input, textarea, select, button")) return;
      if (event.key === "ArrowLeft") {
        event.preventDefault();
        move(-1);
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        move(1);
      } else if (event.key === " ") {
        event.preventDefault();
        setPlaying((current) => !current);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [move]);

  const reachedStage = reachedWorkflowStage(view.steps);
  const workflow = workflowView(view.phase, reachedStage);
  const lastStep = view.steps[view.steps.length - 1];
  const latestStep = lastStep?.step ?? 0;
  const toolCount = view.steps.reduce((count, step) => count + step.toolCalls.length, 0);
  const activeTool = lastStep?.toolCalls.find(
    (tool) => tool.status === "queued" || tool.status === "running",
  );
  const currentIndex = FIXTURE_STATES.indexOf(state);
  const terminalTone = view.phase === "done"
    ? "done"
    : view.phase === "error"
      ? "error"
      : "warn";

  return (
    <div className="agent-fixture" data-fixture-state={state}>
      <header className="agent-fixture__toolbar">
        <div className="agent-fixture__identity">
          <span className="agent-fixture__seal" aria-hidden="true">校</span>
          <span className="agent-fixture__name">AI 状态校样</span>
        </div>

        <div className="agent-fixture__controls" role="group" aria-label="状态播放控制">
          <button
            type="button"
            className="agent-fixture__icon-button is-previous"
            onClick={() => move(-1)}
            aria-label="上一个状态"
            title="上一个状态"
          >
            <IconArrowRight size={15} />
          </button>

          <label className="agent-fixture__select-wrap">
            <span className="agent-fixture__select-label">状态</span>
            <select
              value={state}
              onChange={(event) => chooseState(event.target.value as FixtureState)}
              aria-label="选择 AI 状态"
            >
              {FIXTURE_STATES.map((item) => (
                <option key={item} value={item}>{STATE_META[item].label}</option>
              ))}
            </select>
          </label>

          <span className="agent-fixture__counter" aria-label={`第 ${currentIndex + 1} 个，共 ${FIXTURE_STATES.length} 个状态`}>
            {String(currentIndex + 1).padStart(2, "0")} / {FIXTURE_STATES.length}
          </span>

          <button
            type="button"
            className="agent-fixture__icon-button"
            onClick={() => move(1)}
            aria-label="下一个状态"
            title="下一个状态"
          >
            <IconArrowRight size={15} />
          </button>

          <button
            type="button"
            className={`agent-fixture__play${playing ? " is-playing" : ""}`}
            onClick={() => setPlaying((current) => !current)}
            aria-pressed={playing}
          >
            {playing ? <IconStop size={12} /> : <IconBrush size={13} />}
            {playing ? "暂停" : "自动播放"}
          </button>
        </div>
      </header>

      <main className="agent-fixture__main">
        <section className="agent-fixture__heading" aria-labelledby="agent-fixture-title">
          <p className="agent-fixture__kicker">开发校样 · Agent lifecycle</p>
          <div className="agent-fixture__title-row">
            <h1 id="agent-fixture-title">{meta.label}</h1>
            <code>{state}</code>
          </div>
          <p>{meta.detail}</p>
        </section>

        <section className="agent-fixture__stage" aria-label={`${meta.label}状态预览`}>
          <div className="agent-console">
            <WorkflowSteps
              stages={WRITE_STAGES}
              current={workflow.current}
              state={workflow.state}
            />
            <WorkStatus
              phase={view.phase}
              step={latestStep}
              toolCount={toolCount}
              note={activeTool?.name}
            />
          </div>

          <div className="agent-fixture__feed-head">
            <span>创作过程</span>
            <span>{latestStep > 0 ? `第 ${latestStep} 步` : "尚未落笔"}</span>
          </div>

          <div className="agent-fixture__feed">
            <AgentFeed
              steps={view.steps}
              running={view.running}
              phase={view.phase}
              pendingText={view.pendingText}
            />
            {!view.running && view.steps.length === 0 ? (
              <div className="agent-fixture__empty">研墨以待</div>
            ) : null}
          </div>

          {view.terminal ? (
            <div
              className={`agent-fixture__terminal is-${terminalTone}`}
              role={terminalTone === "error" ? "alert" : "status"}
            >
              <span className="agent-fixture__terminal-mark" aria-hidden="true">
                {terminalTone === "done" ? "成" : terminalTone === "error" ? "误" : "止"}
              </span>
              {view.terminal}
            </div>
          ) : null}
        </section>

        <nav className="agent-fixture__state-strip" aria-label="全部 AI 状态">
          {FIXTURE_STATES.map((item, index) => (
            <button
              type="button"
              key={item}
              className={item === state ? "is-active" : undefined}
              aria-current={item === state ? "step" : undefined}
              onClick={() => chooseState(item)}
            >
              <span>{String(index + 1).padStart(2, "0")}</span>
              {STATE_META[item].label}
            </button>
          ))}
        </nav>
      </main>
    </div>
  );
}
