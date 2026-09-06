// 工具调用 — UI components for a model's tool call.
//
//  • ToolCallCard: a richer, expandable row (icon + verb + tool name + a
//    one-line arg preview) that opens to reveal the full pretty-printed args.
//    Used in the live agent feed so the user can inspect exactly what the AI
//    asked each tool to do.
//  • ToolPill: a compact inline chip, for dense places where a card is too big.

import { memo, useEffect, useMemo, useState } from "react";
import { IconChevron } from "../icons";
import {
  toolGlyph,
  previewArgs,
  formatArgs,
  type RunToolStatus,
} from "../../lib/agentRun";

interface ToolCallProps {
  name: string;
  args: unknown;
}

export type ToolCallStatus = RunToolStatus;

interface ToolCallCardProps extends ToolCallProps {
  status?: ToolCallStatus;
  durationMs?: number;
  summary?: string;
}

const TOOL_STATE_LABEL: Record<ToolCallStatus, string> = {
  queued: "等待执行",
  running: "执行中",
  success: "已完成",
  error: "执行失败",
  cancelled: "已取消",
};

function ToolCallCardInner({
  name,
  args,
  status = "success",
  durationMs,
  summary,
}: ToolCallCardProps) {
  const [open, setOpen] = useState(false);
  const [runningForMs, setRunningForMs] = useState(0);
  const { Icon, verb } = useMemo(() => toolGlyph(name), [name]);
  const preview = useMemo(() => previewArgs(args), [args]);
  const hasArgs = useMemo(
    () =>
      args != null &&
      (typeof args !== "object" || Object.keys(args as object).length > 0),
    [args],
  );
  const formattedArgs = useMemo(() => (hasArgs ? formatArgs(args) : ""), [args, hasArgs]);

  useEffect(() => {
    if (status !== "running") {
      setRunningForMs(0);
      return;
    }
    const startedAt = performance.now();
    const update = () => setRunningForMs(performance.now() - startedAt);
    update();
    const timer = window.setInterval(update, 500);
    return () => window.clearInterval(timer);
  }, [status]);

  const shownDuration = status === "running" ? runningForMs : durationMs;
  const durationLabel = typeof shownDuration === "number" && shownDuration >= 500
    ? shownDuration < 1_000
      ? `${Math.round(shownDuration)}ms`
      : `${(shownDuration / 1_000).toFixed(1)}s`
    : null;

  return (
    <div
      className={`toolcall is-${status}${open ? " is-open" : ""}`}
      role="group"
      aria-label={`${verb} ${name}，${TOOL_STATE_LABEL[status]}`}
      aria-busy={status === "queued" || status === "running"}
      data-tool-status={status}
    >
      <span
        className="a11y-only"
        role={status === "error" ? "alert" : "status"}
        aria-live={status === "error" ? "assertive" : "polite"}
        aria-atomic="true"
      >
        {verb} {name}：{TOOL_STATE_LABEL[status]}
      </span>
      <button
        type="button"
        className="toolcall__head"
        onClick={() => hasArgs && setOpen((v) => !v)}
        aria-expanded={open}
        title={hasArgs ? "展开参数" : name}
        disabled={!hasArgs}
      >
        <span className="toolcall__glyph">
          <Icon size={13} />
        </span>
        <span className="toolcall__verb">{verb}</span>
        <code className="toolcall__name">{name}</code>
        {(summary || preview) && (
          <span className="toolcall__preview" title={summary || preview || undefined}>{summary || preview}</span>
        )}
        <span className="toolcall__state">
          <span className="toolcall__state-dot" aria-hidden="true" />
          {TOOL_STATE_LABEL[status]}
          {durationLabel ? ` · ${durationLabel}` : ""}
        </span>
        {hasArgs && (
          <span className="toolcall__chevron" aria-hidden="true">
            <IconChevron size={14} />
          </span>
        )}
      </button>
      {open && hasArgs && (
        <div className="toolcall__details">
          <pre className="toolcall__args">{formattedArgs}</pre>
        </div>
      )}
    </div>
  );
}

export const ToolCallCard = memo(ToolCallCardInner);

/** A compact inline tool-call chip. */
function ToolPillInner({ name, args }: ToolCallProps) {
  const { Icon, verb } = useMemo(() => toolGlyph(name), [name]);
  const p = useMemo(() => previewArgs(args), [args]);
  return (
    <span className="toolpill" title={p ? `${name} · ${p}` : name}>
      <span className="toolpill__glyph">
        <Icon size={12} />
      </span>
      <span className="toolpill__verb">{verb}</span>
      <code>{name}</code>
      {p && <span className="toolpill__arg">{p}</span>}
    </span>
  );
}

export const ToolPill = memo(ToolPillInner);
