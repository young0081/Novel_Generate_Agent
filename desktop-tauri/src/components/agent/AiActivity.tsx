import { memo } from "react";
import { BrushStroke } from "../icons";

export type AiActivityKind =
  | "preparing"
  | "thinking"
  | "tool"
  | "writing"
  | "stopping";

interface AiActivityProps {
  kind?: AiActivityKind;
  label?: string;
  detail?: string;
  compact?: boolean;
  assertive?: boolean;
  announce?: boolean;
}

const DEFAULT_LABEL: Record<AiActivityKind, string> = {
  preparing: "正在整理上下文",
  thinking: "正在梳理情节",
  tool: "正在执行工具",
  writing: "正在生成文字",
  stopping: "正在停止",
};

/** A compact brush-and-seal activity mark for the full AI lifecycle. */
function AiActivity({
  kind = "thinking",
  label,
  detail,
  compact = false,
  assertive = false,
  announce = true,
}: AiActivityProps) {
  return (
    <div
      className={`ai-activity is-${kind}${compact ? " is-compact" : ""}`}
      role={announce ? "status" : undefined}
      aria-live={announce ? (assertive ? "assertive" : "polite") : undefined}
      aria-atomic="true"
      aria-busy="true"
      data-activity-kind={kind}
    >
      <span className="ai-activity__mark" aria-hidden="true">
        <span className="ai-activity__wash" />
        <BrushStroke className="ai-activity__brush" />
        <span className="ai-activity__dry-tail" />
        <span className="ai-activity__seal-dot" />
      </span>
      <span className="ai-activity__copy">
        <span className="ai-activity__label">{label ?? DEFAULT_LABEL[kind]}</span>
        {!compact && detail ? (
          <span className="ai-activity__detail">{detail}</span>
        ) : null}
      </span>
    </div>
  );
}

export default memo(AiActivity);
