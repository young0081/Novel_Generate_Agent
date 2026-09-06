// The live agent feed — each model turn is a node on an inked spine, carrying
// its reasoning (模型推理) and any tool calls (工具调用). A pending row trails
// the spine while the agent thinks up its next move. Shared by the 创作 and
// 策划 generation views.

import { memo, type CSSProperties, type RefObject } from "react";
import { IconCheck, IconPencil, IconThought, IconTools, IconWarn } from "../icons";
import ReasoningBlock from "./ReasoningBlock";
import { ToolCallCard } from "./ToolCallCard";
import AiActivity, { type AiActivityKind } from "./AiActivity";
import { PHASE_META, type AgentPhase, type RunStep } from "../../lib/agentRun";

const PHASE_ANNOUNCER_STYLE: CSSProperties = {
  position: "absolute",
  width: 1,
  height: 1,
  padding: 0,
  margin: -1,
  overflow: "hidden",
  clip: "rect(0, 0, 0, 0)",
  whiteSpace: "nowrap",
  border: 0,
};

interface AgentFeedProps {
  steps: RunStep[];
  running: boolean;
  /** Text for the trailing pending row (default: thinking up the next move). */
  pendingText?: string;
  phase?: AgentPhase;
  /** Scroll anchor placed after the last row. */
  tailRef?: RefObject<HTMLDivElement | null>;
}

function AgentStepRow({
  step,
  active,
  reasoningActive,
  terminal,
}: {
  step: RunStep;
  active: boolean;
  reasoningActive: boolean;
  terminal: "success" | "warn" | null;
}) {
  const toolsRunning = active && step.toolCalls.some(
    (call) => call.status === "queued" || call.status === "running",
  );
  return (
    <div className={`livestep${active ? " is-active" : ""}${terminal === "warn" ? " is-warn" : ""}`}>
      <div className="livestep__rail" aria-hidden="true">
        <span className="livestep__node">
          {active && !toolsRunning && !step.text.trim() && (
            <span className="livestep__node-pulse" aria-hidden="true" />
          )}
          {terminal === "warn" ? (
            <IconWarn size={11} />
          ) : active ? (
            toolsRunning
              ? <IconTools size={11} />
              : step.streaming
                ? <IconPencil size={11} />
                : <IconThought size={11} />
          ) : (
            <IconCheck size={11} />
          )}
        </span>
      </div>
      <div className="livestep__body">
        <div className="livestep__head">
          <span className="livestep__no">第 {step.step} 步</span>
          {step.toolCalls.length > 0 && (
            <span className="livestep__tag is-tool">
              <IconTools size={11} />
              {step.toolCalls.length} 次落子
            </span>
          )}
          {step.usage?.cached_content_token_count != null &&
            step.usage.cached_content_token_count > 0 && (
              <span
                className="livestep__tag"
                title="本轮提示词中由 Gemini 上下文缓存复用的 token 数"
              >
                缓存命中 {step.usage.cached_content_token_count.toLocaleString()} token
              </span>
            )}
        </div>
        {step.text.trim() && (
          <ReasoningBlock
            text={step.text}
            active={reasoningActive && !toolsRunning}
            mode={step.streaming ? "writing" : "thinking"}
          />
        )}
        {step.toolCalls.length > 0 && (
          <div className="livestep__tools">
            {step.toolCalls.map((tc, i) => (
              <ToolCallCard
                key={`${tc.name}-${i}`}
                name={tc.name}
                args={tc.args}
                status={tc.status}
                durationMs={tc.durationMs}
                summary={tc.summary || tc.error}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

const MemoAgentStepRow = memo(AgentStepRow);

function activityKindForPhase(phase: AgentPhase | undefined, hasSteps: boolean): AiActivityKind {
  switch (phase) {
    case "warming": return "preparing";
    case "tooling": return "tool";
    case "cancelling": return "stopping";
    case "reasoning": return "thinking";
    case "streaming": return "writing";
    default: return hasSteps ? "thinking" : "preparing";
  }
}

function AgentFeed({
  steps,
  running,
  pendingText = "AI 正在思索下一笔…",
  phase,
  tailRef,
}: AgentFeedProps) {
  const last = steps[steps.length - 1];
  const lastToolsSettled = !!last && last.toolCalls.length > 0 && last.toolCalls.every(
    (call) => call.status !== "queued" && call.status !== "running",
  );
  const showActivity = running && (
    phase === "cancelling" ||
    !last ||
    (!last.text.trim() && last.toolCalls.length === 0) ||
    (phase === "reasoning" && lastToolsSettled)
  );
  const terminalWarn = phase === "error" || phase === "stopped" || phase === "cancelled";
  const announcedPhase: AgentPhase = phase ?? (running ? (steps.length > 0 ? "reasoning" : "warming") : "idle");
  const activityKind = activityKindForPhase(announcedPhase, steps.length > 0);
  const activityLabel = announcedPhase === "cancelling"
    ? "正在收锋停笔"
    : announcedPhase === "tooling"
      ? "正在调用工具"
      : announcedPhase === "streaming"
        ? "正在铺陈文字"
        : announcedPhase === "warming"
          ? "正在整理上下文"
          : pendingText;

  return (
    <>
      <span
        role="status"
        aria-live="polite"
        aria-atomic="true"
        style={PHASE_ANNOUNCER_STYLE}
      >
        AI 状态：{PHASE_META[announcedPhase].label}
      </span>

      <div
        className="livefeed"
        aria-busy={running}
        data-phase={phase}
      >
        {steps.map((s, idx) => {
          const isLast = idx === steps.length - 1;
          const active = running && isLast;
          const hasToolWarning = s.toolCalls.some(
            (call) => call.status === "error" || call.status === "cancelled",
          );
          return (
            <MemoAgentStepRow
              key={s.key}
              step={s}
              active={active}
              reasoningActive={active && s.streaming && phase !== "cancelling"}
              terminal={
                hasToolWarning || (!running && isLast && terminalWarn)
                  ? "warn"
                  : !running && isLast
                    ? "success"
                    : null
              }
            />
          );
        })}

        {showActivity && (
          <div className="livestep livestep--pending">
            <div className="livestep__rail" aria-hidden="true">
              <span className="livestep__node livestep__node--ghost">
                <span className="livestep__node-pulse" />
              </span>
            </div>
            <div className="livestep__pending-body">
              <AiActivity
                kind={activityKind}
                label={activityLabel}
                detail={
                  phase === "cancelling"
                    ? "等待当前步骤安全收束"
                    : steps.length === 0
                      ? "正在读取作品设定与当前任务"
                      : phase === "tooling"
                        ? "工具返回后将继续运思"
                        : undefined
                }
                announce={false}
              />
            </div>
          </div>
        )}

        {tailRef && <div ref={tailRef} />}
      </div>
    </>
  );
}

export default memo(AgentFeed);
