// 工具调用 — UI components for a model's tool call.
//
//  • ToolCallCard: a richer, expandable row (icon + verb + tool name + a
//    one-line arg preview) that opens to reveal the full pretty-printed args.
//    Used in the live agent feed so the user can inspect exactly what the AI
//    asked each tool to do.
//  • ToolPill: a compact inline chip, for dense places where a card is too big.

import { memo, useMemo, useState } from "react";
import { IconChevron } from "../icons";
import { toolGlyph, previewArgs, formatArgs } from "../../lib/agentRun";

interface ToolCallProps {
  name: string;
  args: unknown;
}

function ToolCallCardInner({ name, args }: ToolCallProps) {
  const [open, setOpen] = useState(false);
  const { Icon, verb } = useMemo(() => toolGlyph(name), [name]);
  const preview = useMemo(() => previewArgs(args), [args]);
  const hasArgs = useMemo(
    () =>
      args != null &&
      (typeof args !== "object" || Object.keys(args as object).length > 0),
    [args],
  );
  const formattedArgs = useMemo(() => (hasArgs ? formatArgs(args) : ""), [args, hasArgs]);

  return (
    <div className={`toolcall${open ? " is-open" : ""}`}>
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
        {preview && <span className="toolcall__preview">{preview}</span>}
        {hasArgs && (
          <span className="toolcall__chevron" aria-hidden="true">
            <IconChevron size={14} />
          </span>
        )}
      </button>
      {open && hasArgs && (
        <pre className="toolcall__args">{formattedArgs}</pre>
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
