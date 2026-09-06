// 工作流程 — a horizontal stage tracker showing the agent's macro working flow
// (立意 → 运思 → 用器 → 成章). Completed stages carry an ink check, the active
// stage glows cinnabar, and on a non-success finish the final node reads as a
// warning. Pure presentational: driven by `current` + `state`.

import { memo } from "react";
import { IconCheck, IconWarn } from "../icons";
import type { WorkflowStageDef, WorkflowState } from "../../lib/agentRun";

interface WorkflowStepsProps {
  stages: WorkflowStageDef[];
  /** Index of the active stage. */
  current: number;
  state: WorkflowState;
}

type NodeStatus = "done" | "active" | "todo" | "warn";

const STATUS_LABEL: Record<NodeStatus, string> = {
  done: "已完成",
  active: "进行中",
  todo: "未开始",
  warn: "已中断",
};

function statusFor(
  i: number,
  current: number,
  state: WorkflowState,
): NodeStatus {
  if (state === "done") return "done";
  if (state === "idle") return i === 0 ? "active" : "todo";
  if (i < current) return "done";
  if (i === current) {
    return state === "stopped" || state === "error" || state === "cancelled"
      ? "warn"
      : "active";
  }
  return "todo";
}

function WorkflowSteps({
  stages,
  current,
  state,
}: WorkflowStepsProps) {
  return (
    <ol className={`workflow is-${state}`} aria-label="工作流程">
      {stages.map((s, i) => {
        const status = statusFor(i, current, state);
        const filled = state === "done" || (state !== "idle" && i <= current);
        return (
          <li
            className={`wf-stage is-${status}`}
            key={s.key}
            aria-label={`${s.label}：${STATUS_LABEL[status]}`}
            aria-current={status === "active" ? "step" : undefined}
          >
            {i > 0 && (
              <span
                className={`wf-stage__line${filled ? " is-filled" : ""}`}
                aria-hidden="true"
              />
            )}
            <span className="wf-stage__node">
              {status === "active" && (
                <span className="wf-stage__pulse" aria-hidden="true" />
              )}
              {status === "done" ? (
                <IconCheck size={13} />
              ) : status === "warn" ? (
                <IconWarn size={12} />
              ) : (
                <span className="wf-stage__num">{i + 1}</span>
              )}
            </span>
            <span className="wf-stage__label">{s.label}</span>
          </li>
        );
      })}
    </ol>
  );
}

export default memo(WorkflowSteps);
