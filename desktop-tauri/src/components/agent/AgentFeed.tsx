// The live agent feed — each model turn is a node on an inked spine, carrying
// its reasoning (模型推理) and any tool calls (工具调用). A pending row trails
// the spine while the agent thinks up its next move. Shared by the 创作 and
// 策划 generation views.

import { memo, type RefObject } from "react";
import { Spinner } from "../Spinner";
import { IconCheck, IconTools, BrushStroke } from "../icons";
import ReasoningBlock from "./ReasoningBlock";
import { ToolCallCard } from "./ToolCallCard";
import type { RunStep } from "../../lib/agentRun";

interface AgentFeedProps {
  steps: RunStep[];
  running: boolean;
  /** Text for the trailing pending row (default: thinking up the next move). */
  pendingText?: string;
  /** Scroll anchor placed after the last row. */
  tailRef?: RefObject<HTMLDivElement | null>;
}

function AgentStepRow({
  step,
  active,
}: {
  step: RunStep;
  active: boolean;
}) {
  return (
    <div className={`livestep${active ? " is-active" : ""}`}>
      <div className="livestep__rail" aria-hidden="true">
        <span className="livestep__node">
          {active ? (
            <span className="livestep__node-pulse" />
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
        </div>
        <ReasoningBlock text={step.text} active={active} />
        {step.toolCalls.length > 0 && (
          <div className="livestep__tools">
            {step.toolCalls.map((tc, i) => (
              <ToolCallCard
                key={`${tc.name}-${i}`}
                name={tc.name}
                args={tc.args}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

const MemoAgentStepRow = memo(AgentStepRow);

function AgentFeed({
  steps,
  running,
  pendingText = "AI 正在思索下一笔…",
  tailRef,
}: AgentFeedProps) {
  return (
    <div className="livefeed">
      {steps.map((s, idx) => {
        const isLast = idx === steps.length - 1;
        const active = running && isLast;
        return (
          <MemoAgentStepRow key={s.key} step={s} active={active} />
        );
      })}

      {running && (
        <div className="livestep livestep--pending">
          <div className="livestep__rail" aria-hidden="true">
            <span className="livestep__node livestep__node--ghost" />
          </div>
          <div className="livestep__pending-body">
            <BrushStroke className="livestep__brush" aria-hidden="true" />
            <Spinner size={18} />
            <span className="livestep__pending-text">{pendingText}</span>
          </div>
        </div>
      )}

      {tailRef && <div ref={tailRef} />}
    </div>
  );
}

export default memo(AgentFeed);
