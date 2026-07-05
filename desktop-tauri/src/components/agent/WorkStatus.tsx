// 工作状态 — a live one-line strip telling the user what the agent is doing
// right now (待命 / 唤起模型 / 运思推理 / 调用工具 / 已完成 / 已停止 / 出错),
// with a phase-tinted ink orb and at-a-glance counters (步数 · 用器次数).

import { Spinner } from "../Spinner";
import { memo } from "react";
import {
  IconThought,
  IconTools,
  IconCheck,
  IconWarn,
  IconBrush,
} from "../icons";
import { PHASE_META, type AgentPhase } from "../../lib/agentRun";

interface WorkStatusProps {
  phase: AgentPhase;
  /** The latest step number (1-based) reached. */
  step?: number;
  /** How many tool calls have been issued across the run. */
  toolCount?: number;
  /** An optional trailing note, e.g. the current tool name. */
  note?: string;
}

/** The glyph shown inside the orb for a given phase. */
const PhaseGlyph = memo(function PhaseGlyph({ phase }: { phase: AgentPhase }) {
  switch (phase) {
    case "warming":
      return <Spinner size={15} />;
    case "reasoning":
      return <IconThought size={14} />;
    case "tooling":
      return <IconTools size={14} />;
    case "done":
      return <IconCheck size={14} />;
    case "stopped":
    case "error":
      return <IconWarn size={14} />;
    default:
      return <IconBrush size={14} />;
  }
});

function WorkStatus({
  phase,
  step,
  toolCount,
  note,
}: WorkStatusProps) {
  const meta = PHASE_META[phase];
  return (
    <div
      className={`workstatus is-${meta.tone}${meta.live ? " is-live" : ""}`}
      role="status"
      aria-live="polite"
    >
      <span className="workstatus__orb">
        {meta.live && <span className="workstatus__ring" aria-hidden="true" />}
        <PhaseGlyph phase={phase} />
      </span>
      <span className="workstatus__label">{meta.label}</span>
      {note && <span className="workstatus__note">{note}</span>}
      <span className="workstatus__meta">
        {typeof step === "number" && step > 0 && (
          <span className="workstatus__stat">
            第 <b>{step}</b> 步
          </span>
        )}
        {typeof toolCount === "number" && toolCount > 0 && (
          <span className="workstatus__stat">
            <IconTools size={11} />
            <b>{toolCount}</b> 次落子
          </span>
        )}
      </span>
    </div>
  );
}

export default memo(WorkStatus);
